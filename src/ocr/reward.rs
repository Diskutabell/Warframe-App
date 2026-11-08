//! The relic reward screen: whether it is up, how many cards it shows, and the name on each.
//!
//! The screen is laid out around the middle of the game in units of a 1920x1080 screen at 100%
//! interface scale. One unit is min(width / 1920, height / 1080) x the interface scale pixels, so
//! ultrawide and 4:3 screens keep the 16:9 layout in the middle. WFInfo measured the layout
//! (github.com/WFCD/WFinfo, `Ocr.cs`); the numbers below were checked against this app's own
//! frames at 2560x1440 and 85%, where the old fixed band (27-56% of the height, 7-93% of the
//! width) also took in the player names, the scenery and, on the relic selection screen, the
//! relic's possible rewards - which were then reported as the rewards.

use std::collections::BTreeSet;

use anyhow::{Result, bail};

use super::image::{self, Rgb};
use super::matcher::{self, Names, RewardMatch, Word};
use super::tesseract;
use crate::difflib::py_round;

/// One card is this wide; up to four sit side by side, centred on the screen.
pub const CARD_W: f64 = 242.0;
/// The rows a name can use, above the middle of the screen. Names are bottom-aligned: one line
/// runs from 104 to 83 (descenders included), each wrapped line adds ~21 on top.
const NAME_TOP: f64 = 150.0;
const NAME_BOTTOM: f64 = 78.0;
/// The middle of the rarity bar under each card (bronze, silver or gold), and how far above and
/// below it its dark outline runs.
const BAR: f64 = 62.5;
const BAR_EDGE: f64 = 3.2;
/// A slot counts as holding a card when the bar runs under this share of it. Real cards measured
/// 0.89-0.96, every other screen saved so far (gameplay, relic selection, the pause before the
/// cards turn over) at most 0.22; half a card, as a neighbouring layout sees it, about 0.47; the
/// neighbour of a hovered card, its bar a third under the tooltip, about 0.65.
const BAR_SEEN: f64 = 0.6;
/// A bar is at least this bright (luma): silver ones measured ~72, bronze ~85-114.
const BAR_LEVEL: i32 = 50;
/// Where the price panels start, above the middle: just under the bar's outline, so they never
/// cover the names, nor the bar rows that tell whether the screen is still up.
pub const PANEL_TOP: f64 = 52.0;
/// How tall a price panel is.
pub const PANEL_H: f64 = 86.0;
/// Between two panels, as between two cards, and between a panel and what it stands on.
const PANEL_GAP: f64 = 8.0;
/// The top of the cards, above the middle of the screen.
const CARD_TOP: f64 = 318.0;
/// The choice countdown sits above the middle of the cards: half its box's width, and its top.
/// Panels above the cards keep clear of it (measured on a 2560x1440 frame at 85%).
const TIMER_HALF_W: f64 = 66.0;
const TIMER_TOP: f64 = 415.0;
/// The interface scales the game offers, nearest to 100% first, for a game whose EE.cfg cannot
/// be read.
const SCALES: [f64; 16] = [1.0, 0.95, 1.05, 0.9, 1.1, 0.85, 1.15, 0.8, 1.2, 0.75, 1.25, 0.7, 0.65, 0.6, 0.55, 0.5];

/// Pixels per unit for a game of `width` x `height` at interface scale `ui_scale` (1.0 = 100%).
pub fn unit(width: f64, height: f64, ui_scale: f64) -> f64 {
    (width / 1920.0).min(height / 1080.0) * ui_scale
}

/// Where one price panel goes, in units from the middle of the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelPlace {
    /// The left edge, right of the middle (negative: left of it).
    pub left: f64,
    pub width: f64,
    /// How far above the middle the panel's top hangs from, or its bottom stands on (`stands`).
    pub at: f64,
    pub stands: bool,
}

/// The price panels for `cards` cards, left to right: under each card, over the player names
/// below it, or, `above`, standing on top of it. Above the cards they keep the choice countdown
/// in view: the two middle panels of an even row leave its width free, and the middle panel of
/// an odd row stands on top of it. No cards (a message) gives one panel two cards wide.
pub fn panel_places(cards: usize, above: bool) -> Vec<PanelPlace> {
    let (count, width) = if cards == 0 { (1, 2.0 * CARD_W) } else { (cards, CARD_W) };
    let clear = TIMER_HALF_W + PANEL_GAP / 2.0;
    (0..count)
        .map(|k| {
            let left = (k as f64 - count as f64 / 2.0) * width + PANEL_GAP / 2.0;
            let mut p = PanelPlace { left, width: width - PANEL_GAP, at: PANEL_TOP, stands: false };
            if above {
                p.stands = true;
                p.at = CARD_TOP + PANEL_GAP;
                let right = p.left + p.width;
                if count % 2 == 1 && k == count / 2 {
                    p.at = TIMER_TOP + PANEL_GAP;
                } else if right > -clear && p.left < clear {
                    if p.left < 0.0 {
                        p.width = -clear - p.left;
                    } else {
                        p.width = right - clear;
                        p.left = clear;
                    }
                }
            }
            p
        })
        .collect()
}

/// A panel's rectangle in pixels (x, y, width, height), for a game window at `game` (x, y,
/// width, height) with `u` pixels per unit and panels `h` pixels tall.
pub fn panel_rect(p: &PanelPlace, game: (f64, f64, f64, f64), u: f64, h: f64) -> [f64; 4] {
    let (gx, gy, gw, gh) = game;
    let edge = gy + gh / 2.0 - p.at * u;
    [gx + gw / 2.0 + p.left * u, if p.stands { edge - h } else { edge }, p.width * u, h]
}

/// Where the cards are on one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// How many cards: one per player in the squad.
    pub cards: usize,
    /// The interface scale the cards were found at (1.0 = 100%).
    pub ui_scale: f64,
    width: usize,
    height: usize,
}

impl Layout {
    fn unit(&self) -> f64 {
        unit(self.width as f64, self.height as f64, self.ui_scale)
    }

    /// The left edge of the leftmost card and the width of one card, in pixels.
    fn span(&self) -> (f64, f64) {
        let c = CARD_W * self.unit();
        (self.width as f64 / 2.0 - self.cards as f64 * c / 2.0, c)
    }
}

/// The reward screen in `frame`, if it shows one with its cards turned over.
///
/// `scales` are the interface scales to try, the game's own (EE.cfg) first; with none known,
/// every scale the game offers is tried.
pub fn find_layout(frame: &Rgb, scales: &[f64]) -> Option<Layout> {
    let scales = if scales.is_empty() { &SCALES[..] } else { scales };
    scales.iter().find_map(|&s| {
        cards_at(frame, s).map(|cards| Layout { cards, ui_scale: s, width: frame.width, height: frame.height })
    })
}

/// Whether the cards of `layout` are still on screen: any of them still has its bar.
pub fn still_up(frame: &Rgb, layout: &Layout) -> bool {
    if (frame.width, frame.height) != (layout.width, layout.height) {
        return false;
    }
    let (left, c) = layout.span();
    (0..layout.cards).any(|k| {
        let x0 = left + k as f64 * c;
        bar_coverage(frame, layout.unit(), x0, x0 + c).0 >= BAR_SEEN
    })
}

/// How many cards the screen shows at interface scale `ui_scale`, judged by their rarity bars.
///
/// Four cards fill four slots, three cards (and one) sit half a slot over, so both sets of slots
/// are measured. Each layout is recognised by an outer slot no smaller layout reaches, so one
/// visible bar there is enough: hovering a card opens a tooltip centred on it that hides its bar
/// and a third of its neighbours'. Then the bars must break where the cards meet and end where
/// the row ends: stairs in the scenery passed for a row of bars at 70% on a gameplay frame, and
/// stair lines run straight on.
fn cards_at(frame: &Rgb, ui_scale: f64) -> Option<usize> {
    let u = unit(frame.width as f64, frame.height as f64, ui_scale);
    let (mid, c) = (frame.width as f64 / 2.0, CARD_W * u);
    let seen = |from: f64| bar_coverage(frame, u, mid + from * c, mid + (from + 1.0) * c).0 >= BAR_SEEN;
    let even: Vec<bool> = [-2.0, -1.0, 0.0, 1.0].into_iter().map(seen).collect();
    let odd: Vec<bool> = [-1.5, -0.5, 0.5].into_iter().map(seen).collect();
    let cards = if even[0] || even[3] {
        4
    } else if odd[0] || odd[2] {
        3
    } else if even[1] || even[2] {
        2
    } else if odd[1] {
        1
    } else {
        return None;
    };
    let left = mid - cards as f64 * c / 2.0;
    let (_, row) = bar_coverage(frame, u, left, left + cards as f64 * c);
    breaks_at_edges(frame, row?, left, c, cards).then_some(cards)
}

fn luma(frame: &Rgb, x: usize, y: usize) -> i32 {
    let i = (y * frame.width + x) * 3;
    let p = &frame.data[i..i + 3];
    (p[0] as i32 * 299 + p[1] as i32 * 587 + p[2] as i32 * 114) / 1000
}

/// How much of x0..x1 (its middle 80%) has a rarity bar under it - a line clearly brighter than
/// the dark outline just above and below - and the row where it has most. The bar's height is
/// searched within a few units, as the measurement and the rounding of small scales leave slack.
fn bar_coverage(frame: &Rgb, u: f64, x0: f64, x1: f64) -> (f64, Option<usize>) {
    let inset = (x1 - x0) * 0.1;
    let xa = (x0 + inset).max(0.0) as usize;
    let xb = ((x1 - inset).max(0.0) as usize).min(frame.width);
    if xb <= xa {
        return (0.0, None);
    }
    let edge = (BAR_EDGE * u).round().max(2.0) as usize;
    let centre = frame.height as f64 / 2.0 - BAR * u;
    let rows: BTreeSet<usize> = (-8..=8)
        .map(|step| (centre + step as f64 * 0.5 * u).round())
        .filter(|y| *y >= edge as f64 && *y + (edge as f64) < frame.height as f64)
        .map(|y| y as usize)
        .collect();
    let mut best = (0.0f64, None);
    for y in rows {
        let lit = (xa..xb)
            .filter(|&x| luma(frame, x, y) - luma(frame, x, y - edge).max(luma(frame, x, y + edge)) >= 25)
            .count();
        let share = lit as f64 / (xb - xa) as f64;
        if share > best.0 {
            best = (share, Some(y));
        }
    }
    best
}

/// Whether the bar in row `y` breaks at the edges of the `cards` cards that start at `left`,
/// each `c` wide. On real frames the row drops to under 65% of the bar beside it over 3-10
/// pixels between two cards and at both ends of the row, silver bars included. (The bar is
/// brighter near its ends than in its middle, so it is compared with itself right there.)
///
/// An edge without a bright, even bar beside it says nothing either way: a hovered card's
/// tooltip covers both of its edges, and in a fight damage numbers and health bars are anything
/// but even. At least two edges must be judged - an enemy's health bar at the
/// right height ends anywhere, not at two card edges - three in five of those must break, and
/// one between two cards if any of those can be judged: that is where a line in the scenery would
/// run on.
fn breaks_at_edges(frame: &Rgb, y: usize, left: f64, c: f64, cards: usize) -> bool {
    let reach = (0.05 * c).max(4.0);
    let row = |x0: f64, x1: f64| -> Vec<i32> {
        (x0.max(0.0) as usize..(x1.max(0.0) as usize).min(frame.width)).map(|x| luma(frame, x, y)).collect()
    };
    // Per edge: None when it cannot be judged, else whether the bar breaks there.
    let edges: Vec<Option<bool>> = (0..=cards)
        .map(|j| {
            let edge = left + j as f64 * c;
            // The bar's brightness next to the edge, on the side or sides where a card is.
            let mut beside = Vec::new();
            if j > 0 {
                beside.extend(row(edge - 3.0 * reach, edge - reach));
            }
            if j < cards {
                beside.extend(row(edge + reach, edge + 3.0 * reach));
            }
            if beside.is_empty() {
                return None;
            }
            beside.sort_unstable();
            let level = beside[beside.len() / 2];
            // A rarity bar runs bright and even right up to its card's edge (the dimmest seen, silver,
            // at ~72). A tooltip's background (~20-45) or a busy scene beside the edge says nothing.
            let even = beside.iter().filter(|l| **l * 4 >= level * 3).count() * 5 >= beside.len() * 4;
            if level < BAR_LEVEL || !even {
                return None;
            }
            Some(row(edge - reach, edge + reach).into_iter().filter(|l| l * 20 < level * 13).count() >= 2)
        })
        .collect();
    let judged: Vec<bool> = edges.iter().flatten().copied().collect();
    let breaks = judged.iter().filter(|b| **b).count();
    let inner: Vec<bool> = edges[1..cards].iter().flatten().copied().collect();
    judged.len() >= 2 && breaks * 5 >= judged.len() * 3 && (inner.is_empty() || inner.contains(&true))
}

/// What the cards say.
pub struct CardsRead {
    /// One entry per card, left to right; None where no name was recognised.
    pub names: Vec<Option<RewardMatch>>,
    /// The words read on each card, per image variant, for the record.
    pub text: String,
}

/// Reads the name on every card of `layout`.
///
/// Only the rows the names can use are read, each card on its own: a card's words never mix
/// with its neighbour's, and a name is accepted only where a card is. Four images of the rows
/// go to tesseract side by side, and each card keeps the best name any of them gave.
pub fn read(frame: &Rgb, layout: &Layout, names: &Names) -> Result<CardsRead> {
    if (frame.width, frame.height) != (layout.width, layout.height) {
        bail!("the frame is not the one the cards were found on");
    }
    if crate::compositor::which("tesseract").is_none() {
        bail!("tesseract is not installed, so the reward screen cannot be read.");
    }
    let u = layout.unit();
    let (left, c) = layout.span();
    let x0 = left.max(0.0) as usize;
    let x1 = ((left + layout.cards as f64 * c) as usize).min(frame.width);
    let y0 = (frame.height as f64 / 2.0 - NAME_TOP * u).max(0.0) as usize;
    let y1 = ((frame.height as f64 / 2.0 - NAME_BOTTOM * u) as usize).min(frame.height);
    if x1 <= x0 || y1 <= y0 {
        bail!("the reward cards lie outside the frame");
    }
    let variants = image::name_variants(&frame.crop(x0, y0, x1, y1));
    // One uniform block of text (page mode 6): on 4K frames in the red Stalker theme it read
    // two-line names whole where sparse text (11) dropped words, and as well on the rest.
    let passes: Vec<Result<Vec<tesseract::TsvWord>>> = std::thread::scope(|s| {
        let jobs: Vec<_> = variants.iter().map(|v| s.spawn(move || tesseract::words(v, 6))).collect();
        jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| bail!("tesseract thread panicked"))).collect()
    });

    let mut best: Vec<Option<RewardMatch>> = vec![None; layout.cards];
    let mut text = Vec::new();
    let mut first_error = None;
    for pass in passes {
        let words = match pass {
            Ok(words) => words,
            Err(e) => {
                first_error = first_error.or(Some(e));
                continue;
            }
        };
        // Which card each word is on, by its middle. The images are twice the frame's size.
        // Tesseract lists the words in reading order, line by line across the cards, so each
        // card's words stay in order: sorting them by position instead put a word whose box had
        // taken in the card's border ("Systemsl") on the line above its neighbours.
        let mut per_card: Vec<Vec<&Word>> = vec![Vec::new(); layout.cards];
        for w in words.iter().filter(|w| w.conf >= 20.0) {
            let middle = x0 as f64 + (w.word.left + w.word.right) as f64 / 4.0;
            let k = ((middle - left) / c).floor();
            if k >= 0.0 && (k as usize) < layout.cards {
                per_card[k as usize].push(&w.word);
            }
        }
        let mut line = Vec::new();
        for (k, card) in per_card.iter().enumerate() {
            line.push(card.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "));
            if let Some(found) = matcher::best_name(card, names) {
                if best[k].as_ref().is_none_or(|b| better(&found, b)) {
                    best[k] = Some(found);
                }
            }
        }
        text.push(line.join(" | "));
    }
    if text.is_empty() {
        return Err(first_error.unwrap_or_else(|| anyhow::anyhow!("tesseract read nothing")));
    }
    Ok(CardsRead { names: best, text: text.join("\n") })
}

/// A longer name read at least this well beats a shorter one it contains.
const WHOLE: f64 = 0.88;

/// Whether `a` is a better reading of a card than `b`.
///
/// Tesseract drops a word far more often than it makes one up: on real frames "Voruna Prime
/// Chassis Blueprint" was now and then read as "Voruna Prime Blueprint", which then matched the
/// shorter sibling perfectly. Text without the extra word caps the longer name near 0.85, so a
/// longer name containing the shorter one, read at `WHOLE` or better, wins. Otherwise scores
/// within a hundredth tie, and the longer name is the more specific one, as in the matcher.
pub fn better(a: &RewardMatch, b: &RewardMatch) -> bool {
    if a.score >= WHOLE && contains_name(&a.name, &b.name) {
        return true;
    }
    if b.score >= WHOLE && contains_name(&b.name, &a.name) {
        return false;
    }
    let key = |m: &RewardMatch| (py_round(m.score, 2), m.name.chars().count());
    let (ka, kb) = (key(a), key(b));
    ka.0 > kb.0 || (ka.0 == kb.0 && ka.1 > kb.1)
}

/// Whether `long` has every word of `short`, in order, and more.
fn contains_name(long: &str, short: &str) -> bool {
    let (long, short) = (matcher::norm(long), matcher::norm(short));
    let mut rest = long.iter();
    long.len() > short.len() && short.iter().all(|word| rest.any(|w| w == word))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dark frame with a rarity bar under each of `cards` cards at interface scale `scale`.
    fn screen(width: usize, height: usize, scale: f64, cards: usize) -> Rgb {
        let mut frame = Rgb { width, height, data: vec![30; width * height * 3] };
        let u = unit(width as f64, height as f64, scale);
        let c = CARD_W * u;
        let left = width as f64 / 2.0 - cards as f64 * c / 2.0;
        let bar = height as f64 / 2.0 - BAR * u;
        for k in 0..cards {
            // Cards stand a few pixels apart.
            let (xa, xb) = ((left + k as f64 * c + 4.0) as usize, (left + (k + 1) as f64 * c - 4.0) as usize);
            for y in (bar - 4.0 * u) as usize..=(bar + 4.0 * u) as usize {
                let lit = (y as f64 - bar).abs() <= 2.2 * u;
                let colour = if lit { [130, 78, 56] } else { [12, 8, 10] };
                for x in xa..xb {
                    let i = (y * width + x) * 3;
                    frame.data[i..i + 3].copy_from_slice(&colour);
                }
            }
        }
        frame
    }

    #[test]
    fn counts_the_cards_by_their_bars() {
        for cards in 1..=4 {
            let frame = screen(2560, 1440, 0.85, cards);
            let found = find_layout(&frame, &[0.85]).map(|l| l.cards);
            assert_eq!(found, Some(cards), "{cards} card(s)");
        }
    }

    #[test]
    fn ultrawide_and_unknown_scales_are_found() {
        let frame = screen(3440, 1440, 1.0, 3);
        assert_eq!(find_layout(&frame, &[1.0]).map(|l| l.cards), Some(3));
        // Without EE.cfg the scale is searched for.
        let frame = screen(1920, 1080, 0.7, 4);
        let layout = find_layout(&frame, &[]).expect("found at some scale");
        assert_eq!(layout.cards, 4);
        assert!((layout.ui_scale - 0.7).abs() <= 0.05 + 1e-9, "{}", layout.ui_scale);
    }

    #[test]
    fn a_screen_without_bars_is_not_a_reward_screen() {
        let frame = Rgb { width: 1920, height: 1080, data: vec![30; 1920 * 1080 * 3] };
        assert_eq!(find_layout(&frame, &[]), None);
        // One bright line across the whole screen is not a row of cards at another height.
        let mut lined = frame.clone();
        let y = 300;
        lined.data[y * 1920 * 3..(y + 1) * 1920 * 3].fill(200);
        assert_eq!(find_layout(&lined, &[1.0]), None);
    }

    #[test]
    fn a_line_that_runs_on_across_the_card_edges_is_not_a_row_of_cards() {
        // Like the stairs on a gameplay frame: bar-shaped, at bar height, but without the gaps
        // between cards.
        let mut frame = screen(1920, 1080, 1.0, 4);
        let bar = (540.0 - BAR) as usize;
        for y in bar - 4..=bar + 4 {
            let lit = (y as f64 - (540.0 - BAR)).abs() <= 2.2;
            let colour = if lit { [130, 78, 56] } else { [12, 8, 10] };
            for x in 400..1520 {
                let i = (y * 1920 + x) * 3;
                frame.data[i..i + 3].copy_from_slice(&colour);
            }
        }
        assert_eq!(find_layout(&frame, &[1.0]), None);
        assert_eq!(find_layout(&frame, &[]), None);
    }

    /// Darkens what a hovered card's tooltip covers: from just above the bar down, 0.87 cards to
    /// either side of the card's middle.
    fn hover(frame: &mut Rgb, scale: f64, cards: usize, card: usize) {
        let u = unit(frame.width as f64, frame.height as f64, scale);
        let c = CARD_W * u;
        let middle = frame.width as f64 / 2.0 + (card as f64 - (cards as f64 - 1.0) / 2.0) * c;
        let top = (frame.height as f64 / 2.0 - (BAR + 4.0) * u) as usize;
        for y in top..top + (80.0 * u) as usize {
            for x in (middle - 0.87 * c) as usize..(middle + 0.87 * c) as usize {
                let i = (y * frame.width + x) * 3;
                frame.data[i..i + 3].copy_from_slice(&[24, 14, 34]);
            }
        }
    }

    #[test]
    fn a_hovered_cards_tooltip_does_not_hide_the_row() {
        // Two cards or one, hovered, leave fewer than two edges to judge: the next look, with the
        // mouse moved, finds them.
        for (cards, hovered) in [(4, 0), (4, 2), (4, 3), (3, 0), (3, 1), (3, 2)] {
            let mut frame = screen(2560, 1440, 0.85, cards);
            hover(&mut frame, 0.85, cards, hovered);
            let found = find_layout(&frame, &[0.85]).map(|l| l.cards);
            assert_eq!(found, Some(cards), "{cards} cards, card {hovered} hovered");
        }
    }

    #[test]
    fn a_name_read_whole_beats_its_sibling_without_a_word() {
        let read = |name: &str, score| RewardMatch { name: name.into(), uid: String::new(), score };
        let whole = read("Voruna Prime Chassis Blueprint", 0.93);
        let short = read("Voruna Prime Blueprint", 1.0);
        assert!(better(&whole, &short) && !better(&short, &whole));
        // Not when the longer one was itself a guess from text missing the word.
        let guess = read("Voruna Prime Chassis Blueprint", 0.85);
        assert!(better(&short, &guess));
        // Names that do not contain each other compare by score.
        assert!(better(&read("Paris Prime Grip", 0.95), &read("Paris Prime String", 0.9)));
    }

    #[test]
    fn panels_sit_under_the_cards_or_above_them_clear_of_the_countdown() {
        // Under: one per card, 8 apart, hanging from just under the bars.
        let below = panel_places(4, false);
        let lefts: Vec<f64> = below.iter().map(|p| p.left).collect();
        assert_eq!(lefts, [-480.0, -238.0, 4.0, 246.0]);
        assert!(below.iter().all(|p| p.width == 234.0 && p.at == PANEL_TOP && !p.stands));
        // Above, four cards: the middle two leave the countdown's width free.
        let above = panel_places(4, true);
        assert!(above.iter().all(|p| p.stands && p.at == CARD_TOP + PANEL_GAP));
        assert_eq!((above[1].left + above[1].width, above[2].left), (-70.0, 70.0));
        assert_eq!((above[0].width, above[3].width), (234.0, 234.0));
        // Above, three cards: the middle one stands on top of the countdown.
        let three = panel_places(3, true);
        assert_eq!(three[1].at, TIMER_TOP + PANEL_GAP);
        assert_eq!((three[0].at, three[0].width), (CARD_TOP + PANEL_GAP, 234.0));
        // A message: one panel, two cards wide, in the middle.
        assert_eq!(panel_places(0, false), [PanelPlace { left: -238.0, width: 476.0, at: PANEL_TOP, stands: false }]);
        // In pixels: a 2560x1440 game at 85%, panels 97 px tall.
        let u = unit(2560.0, 1440.0, 0.85);
        let r = panel_rect(&above[0], (0.0, 0.0, 2560.0, 1440.0), u, 97.0);
        assert_eq!(r.map(f64::round), [736.0, 254.0, 265.0, 97.0]);
    }

    #[test]
    fn the_cards_are_gone_when_their_bars_are() {
        let up = screen(1920, 1080, 1.0, 2);
        let layout = find_layout(&up, &[1.0]).unwrap();
        assert!(still_up(&up, &layout));
        let gone = Rgb { width: 1920, height: 1080, data: vec![30; 1920 * 1080 * 3] };
        assert!(!still_up(&gone, &layout));
    }
}
