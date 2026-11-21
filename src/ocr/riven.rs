//! Reading a riven card from OCR text: the weapon, and each rolled stat with its sign.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::difflib::SequenceMatcher;
use crate::rivens::{self, Unit, WORDING};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParsedStat {
    pub stat: &'static str,
    /// The value as the card prints it, sign included.
    pub value: f64,
    /// Whether it is the riven's negative (its curse). For most stats that is the printed minus;
    /// Recoil prints its good rolls with a minus (`rivens::Stat::inverted`).
    pub negative: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RivenRead {
    pub weapon: Option<String>,
    pub stats: Vec<ParsedStat>,
}

/// "+244.4% Damage", "-101.5% Impact", "+2.2 Punch Through", "+0.5s Combo Duration". What follows
/// the number is examined separately (`stat_after`), because the card wraps long names and
/// tesseract sprinkles junk between the words ("Magazine . ~ v & Capacity"). A unit letter counts
/// only written onto the number: "+90.5 slash" (the % lost) must keep its "s".
static SIGNED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([+\-\u{2212}])\s*(\d+(?:[.,]\d+)?)(?:\s*%|[smx]\b)?").unwrap());
/// Faction damage can be printed as a multiplier: "x1.45 Damage to Corpus" (x0.55 for a negative).
static MULTIPLIER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)[xX\u{d7}]\s?(\d+(?:[.,]\d+)?)").unwrap());

/// The longest wording, in words: the exact pass must try it before its own prefixes, or
/// "Critical Chance for Slide Attack" reads as plain "Critical Chance".
static LONGEST_WORDING: LazyLock<usize> =
    LazyLock::new(|| WORDING.iter().map(|(w, _)| w.split(' ').count()).max().unwrap_or(1));

/// A word as OCR read it against a wording's word: exact, or the same first four letters at about
/// the same length ("magazme" for "magazine"). The length check keeps "punch" from passing for
/// "puncture" - both are stats.
fn close_word(read: &str, tok: &str) -> bool {
    read == tok
        || (tok.len() > 3 && read.len() > 3 && read.starts_with(&tok[..4]) && read.len().abs_diff(tok.len()) <= 2)
}

/// The stat named at the start of `window`, tolerating OCR noise between its words.
pub fn stat_after(window: &str) -> Option<&'static str> {
    let letters: String = window
        .chars()
        .map(|c| if c.is_ascii_alphabetic() || c == ' ' { c.to_ascii_lowercase() } else { ' ' })
        .collect();
    let words: Vec<&str> = letters.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    for n in (1..=words.len().min(*LONGEST_WORDING)).rev() {
        let key = words[..n].join(" ");
        if let Some((_, stat)) = WORDING.iter().find(|(w, _)| *w == key) {
            return Some(stat);
        }
    }
    // Ordered-subsequence match: every word of the wording, in order, starting within the first
    // two words; a word matches exactly or on its first four letters.
    let mut best: Option<(usize, &'static str)> = None;
    for (key, stat) in WORDING {
        let kw: Vec<&str> = key.split(' ').collect();
        let (mut i, mut start) = (0usize, None);
        for (j, w) in words.iter().take(kw.len() + 5).enumerate() {
            if close_word(w, kw[i]) {
                if i == 0 {
                    start = Some(j);
                    if j > 1 {
                        break; // the name has to begin right after the number
                    }
                }
                i += 1;
                if i == kw.len() {
                    break;
                }
            }
        }
        if i == kw.len() && start.is_some_and(|s| s <= 1) && best.is_none_or(|(len, _)| kw.len() > len) {
            best = Some((kw.len(), stat));
        }
    }
    if let Some((_, stat)) = best {
        return Some(stat);
    }
    if let Some(stat) = behind_an_icon(words[0]) {
        return Some(stat);
    }
    // The card wraps long names, and the wrapped line is what tesseract loses on a busy background:
    // "+57.5% Magazine Capacity" can come out as "+57.5% Magazine". The first word still
    // names the stat if no other wording starts with it - and if it never occurs inside a longer
    // name, where it could be what is left of that one ("Chance" from "Critical Chance").
    let first = words[0];
    let close = |tok: &str| close_word(first, tok);
    if WORDING.iter().any(|(w, _)| w.split(' ').skip(1).any(close)) {
        return None;
    }
    let mut owners = WORDING.iter().filter(|(w, _)| w.split(' ').next().is_some_and(close)).map(|(_, s)| *s);
    let stat = owners.next()?;
    owners.all(|s| s == stat).then_some(stat)
}

/// A damage type's name read with its icon glued on: the card draws the icon right before
/// "Impact", "Toxin" and the like, and tesseract reads it as letters ("-105% AMlmpact"). The end of
/// the word must be close to a one-word name.
fn behind_an_icon(word: &str) -> Option<&'static str> {
    WORDING.iter().filter(|(w, _)| !w.contains(' ') && w.len() >= 4 && word.len() >= w.len()).find_map(|(w, stat)| {
        let tail = &word[word.len() - w.len()..];
        (SequenceMatcher::new(tail, w).ratio() >= 0.8).then_some(*stat)
    })
}

/// The text after a number that may name its stat: up to the next number on the card (the next
/// stat's), and at most 70 characters.
fn window_after(blob: &str, end: usize, next: usize) -> &str {
    let rest = &blob[end..next.max(end)];
    let cut = rest.char_indices().nth(70).map_or(rest.len(), |(i, _)| i);
    &rest[..cut]
}

fn number(s: &str) -> Option<f64> {
    s.replace(',', ".").parse().ok()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric()
}

/// Whether `needle` appears in `hay` as whole words: "Bo" must not match inside "Bolto" or
/// "about", and "War" not inside "Warframe".
fn contains_words(hay: &str, needle: &str) -> bool {
    !needle.is_empty()
        && hay.match_indices(needle).any(|(i, _)| {
            let before = hay[..i].chars().next_back();
            let after = hay[i + needle.len()..].chars().next();
            !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
        })
}

/// The weapon named on the card. A riven is called "<Weapon> <Riven-name>", so the weapon's name
/// should appear verbatim; failing that, fuzzy-match the start of each line.
pub fn find_weapon(text: &str, weapons: &[String]) -> Option<String> {
    let low = text.to_lowercase();
    let mut best: Option<&String> = None;
    for w in weapons {
        if contains_words(&low, &w.to_lowercase()) && best.is_none_or(|b| w.chars().count() > b.chars().count()) {
            best = Some(w);
        }
    }
    if let Some(w) = best {
        return Some(w.clone());
    }
    // Each name against as many words as it has at the start of a line: the riven's own name
    // follows on the same line and must not count against it ("Latr0n Critatis").
    let (mut best, mut score) = (None, 0.0);
    for line in text.lines() {
        let line = line.to_lowercase();
        let tokens: Vec<&str> = line.trim_start_matches(|c: char| !c.is_alphanumeric()).split_whitespace().collect();
        for w in weapons {
            let words = w.split_whitespace().count();
            if tokens.len() < words {
                continue;
            }
            let start = tokens[..words].join(" ");
            let r = SequenceMatcher::new(&w.to_lowercase(), &start).ratio();
            if r > score {
                (best, score) = (Some(w), r);
            }
        }
    }
    best.filter(|_| score >= 0.82).cloned()
}

/// Where the text of a riven card shown in the middle of the screen lies, in units of a
/// 1920x1080 screen at 100% interface scale (see `reward::unit`): the name and the stats below the
/// card's picture, the card about 280 wide. Measured on the cycling and confirm screens at
/// 2560x1440 and 85%.
pub const CARD_HALF_W: f64 = 150.0;
const CARD_TEXT_TOP: f64 = 70.0;
const CARD_TEXT_BOTTOM: f64 = 285.0;

/// The grade panel beside such a card, in the same units: its width, how far above the middle
/// its top sits (level with the top of the card's picture), its height for `rows` stat lines,
/// and its gap to the card.
pub const PANEL_W: f64 = 330.0;
pub const PANEL_TOP: f64 = 95.0;
pub const PANEL_GAP: f64 = 12.0;
pub fn panel_height(rows: usize) -> f64 {
    96.0 + 44.0 * rows.max(1) as f64 + 58.0
}

/// The pixel box (x0, y0, x1, y1) of that text at `ui_scale`, if it fits on the frame.
pub fn card_box(width: usize, height: usize, ui_scale: f64) -> Option<(usize, usize, usize, usize)> {
    let u = super::reward::unit(width as f64, height as f64, ui_scale);
    let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
    let x0 = (cx - CARD_HALF_W * u).max(0.0) as usize;
    let x1 = ((cx + CARD_HALF_W * u) as usize).min(width);
    let y0 = (cy + CARD_TEXT_TOP * u) as usize;
    let y1 = ((cy + CARD_TEXT_BOTTOM * u) as usize).min(height);
    (x1 > x0 + 10 && y1 > y0 + 10).then_some((x0, y0, x1, y1))
}

/// Whether reading `a` found more of a card than `b`: more stats, then a longer weapon name
/// ("Kuva Bramma" over "Bramma" when one reading lost a word).
pub fn fuller(a: &RivenRead, b: &RivenRead) -> bool {
    let len = |r: &RivenRead| r.weapon.as_ref().map_or(0, |w| w.chars().count());
    (a.stats.len(), len(a)) > (b.stats.len(), len(b))
}

/// Pulls the weapon and its rolled stats out of OCR'd riven-card text.
pub fn parse(text: &str, weapons: &[String]) -> RivenRead {
    // Long stat names wrap ("+57.5% Magazine" / "Capacity"), so match across the whole text, not
    // per line: the window stops mattering at the next stat anyway.
    let blob = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // Where every number starts: a stat's name is looked for only up to the next one.
    let mut starts: Vec<usize> = SIGNED
        .find_iter(&blob)
        .chain(MULTIPLIER.find_iter(&blob))
        .map(|m| m.start())
        .chain(std::iter::once(blob.len()))
        .collect();
    starts.sort_unstable();
    let next_after = |end: usize| starts.iter().copied().find(|&s| s >= end).unwrap_or(blob.len());
    let mut found: Vec<(usize, ParsedStat)> = Vec::new();
    for m in SIGNED.captures_iter(&blob) {
        let whole = m.get(0).expect("match");
        let Some(stat) = stat_after(window_after(&blob, whole.end(), next_after(whole.end()))) else { continue };
        let Some(val) = number(&m[2]) else { continue };
        let shown_negative = &m[1] != "+";
        // Recoil is printed the other way round: a good roll is a reduction ("-91.1%").
        let negative = shown_negative != rivens::stat(stat).is_some_and(|s| s.inverted);
        found.push((whole.start(), ParsedStat { stat, value: if shown_negative { -val } else { val }, negative }));
    }
    for m in MULTIPLIER.captures_iter(&blob) {
        let whole = m.get(0).expect("match");
        let Some(stat) = stat_after(window_after(&blob, whole.end(), next_after(whole.end()))) else { continue };
        if rivens::stat(stat).is_none_or(|s| s.unit != Unit::Multiplier) {
            continue; // "(x2 for Heavy Attacks)" and the like are descriptions, not rolls
        }
        let Some(mult) = number(&m[1]) else { continue };
        let pct = ((mult - 1.0) * 100.0 * 1e4).round() / 1e4;
        found.push((whole.start(), ParsedStat { stat, value: pct, negative: pct < 0.0 }));
    }
    found.sort_by_key(|(pos, _)| *pos);
    let mut stats: Vec<ParsedStat> = Vec::new();
    for (_, s) in found {
        if stats.iter().all(|o| o.stat != s.stat) {
            stats.push(s);
        }
    }
    RivenRead { weapon: find_weapon(text, weapons), stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapons() -> Vec<String> {
        ["Latron", "Latron Prime", "Bo", "War", "Kuva Bramma", "Bramma", "Soma"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_a_wrapped_card() {
        // What tesseract read off a riven card.
        let text = "Latron Critatis\n+56.9% Ammo Maximum\n+167.4% Critical Chance\n+57.5% Magazine\nCapacity\nMR 9";
        let r = parse(text, &weapons());
        assert_eq!(r.weapon.as_deref(), Some("Latron"));
        let got: Vec<(&str, f64)> = r.stats.iter().map(|s| (s.stat, s.value)).collect();
        assert_eq!(got, [("Ammo Maximum", 56.9), ("Critical Chance", 167.4), ("Magazine Capacity", 57.5)]);
    }

    /// What each reading of `text` says: (stat, value, curse?).
    fn stats_of(text: &str) -> Vec<(&'static str, f64, bool)> {
        parse(text, &[]).stats.iter().map(|s| (s.stat, s.value, s.negative)).collect()
    }

    #[test]
    fn a_stat_never_takes_the_next_stats_name() {
        // "Capacity" lost from a stat that is not the last one.
        assert_eq!(
            stats_of("+57.5% Magazine\n+167.4% Critical Chance\n+56.9% Ammo Maximum"),
            [("Magazine Capacity", 57.5, false), ("Critical Chance", 167.4, false), ("Ammo Maximum", 56.9, false)]
        );
        // An icon glued onto "Impact", with another stat after it.
        assert_eq!(
            stats_of("-105% AMlmpact\n+65% Fire Rate"),
            [("Impact Damage", -105.0, true), ("Fire Rate / Attack Speed", 65.0, false)]
        );
        // Junk before "Damage", and faction damage after it.
        assert_eq!(
            stats_of("+120% o Damage\n+45% Damage to Grineer"),
            [("Damage", 120.0, false), ("Damage to Grineer", 45.0, false)]
        );
    }

    #[test]
    fn a_lost_percent_sign_does_not_eat_the_stats_first_letter() {
        assert_eq!(
            stats_of("+90.5 slash\n+112.0 status chance\n+57.5 magazine capacity"),
            [("Slash Damage", 90.5, false), ("Status Chance", 112.0, false), ("Magazine Capacity", 57.5, false)]
        );
        // Units written on the number still parse.
        assert_eq!(
            stats_of("+0.5s Combo Duration +2.2m Punch Through"),
            [("Combo Duration", 0.5, false), ("Punch Through", 2.2, false)]
        );
    }

    #[test]
    fn recoil_is_printed_the_other_way_round() {
        // On the card a good recoil roll is a reduction ("-91.1%"), a curse an increase ("+72.9%").
        assert_eq!(stats_of("+163% Damage -91.1% Weapon Recoil"), [("Damage", 163.0, false), ("Recoil", -91.1, false)]);
        assert_eq!(
            stats_of("+155% Damage +140% Critical Chance +84% Multishot +72.9% Weapon Recoil"),
            [
                ("Damage", 155.0, false),
                ("Critical Chance", 140.0, false),
                ("Multishot", 84.0, false),
                ("Recoil", 72.9, true)
            ]
        );
    }

    #[test]
    fn a_weapon_name_with_an_ocr_slip_is_still_found() {
        let w = weapons();
        assert_eq!(find_weapon("Latr0n Critatis\n+120% Damage", &w).as_deref(), Some("Latron"));
        assert_eq!(find_weapon("Kuva Brama Acri-tox\n+12% Damage", &w).as_deref(), Some("Kuva Bramma"));
        assert_eq!(find_weapon("+120% Damage\n+45% Critical Chance", &w), None);
    }

    #[test]
    fn short_weapon_names_need_whole_words() {
        assert_eq!(find_weapon("About this Warframe mod: bonus", &weapons()), None);
        assert_eq!(find_weapon("KUVA BRAMMA Acri-tox\n+12% Damage", &weapons()).as_deref(), Some("Kuva Bramma"));
        assert_eq!(find_weapon("Bo Visi-tron", &weapons()).as_deref(), Some("Bo"));
    }

    #[test]
    fn longest_wording_wins_and_new_stats_parse() {
        let text = "+120.5% Critical Chance for Slide Attack -45.2% Weak Point Damage +99.1% Status Damage";
        let got: Vec<(&str, bool)> = parse(text, &[]).stats.iter().map(|s| (s.stat, s.negative)).collect();
        assert_eq!(got, [("Slide Critical Chance", false), ("Weakpoint Damage", true), ("Status Damage", false)]);
    }

    #[test]
    fn a_lost_wrapped_line_still_names_the_stat() {
        // Tesseract lost "Capacity" against the busy background of the cycle screen.
        let text = "- Latron Arma- -\nampicron @\no> +56.9% Ammo Maximum . : F\n\
                    +167.4% Critical Chance ' -\n- v +57.5% Magazine - - .\nMR 6 9 _ Al |\nLOCK TRAIT . p S8";
        let r = parse(text, &weapons());
        let got: Vec<&str> = r.stats.iter().map(|s| s.stat).collect();
        assert_eq!(got, ["Ammo Maximum", "Critical Chance", "Magazine Capacity"]);
        // A first word that is part of several names, or the tail of another, proves nothing.
        assert_eq!(stat_after("Ammo"), None);
        assert_eq!(stat_after("Chance"), None);
        assert_eq!(stat_after("Reload - ."), None);
        assert_eq!(stat_after("Punch ~"), Some("Punch Through"));
    }

    #[test]
    fn a_damage_types_icon_glued_to_its_name() {
        // The Kuva Bramma card of 2026-10-25: tesseract read the impact icon as letters.
        let r = parse("+65% Fire Rate (x2 for\nBows)\nN, 0-105% AMlmpact /", &[]);
        let got: Vec<(&str, f64)> = r.stats.iter().map(|s| (s.stat, s.value)).collect();
        assert_eq!(got, [("Fire Rate / Attack Speed", 65.0), ("Impact Damage", -105.0)]);
        assert_eq!(stat_after("Mmpact"), Some("Impact Damage"));
        assert_eq!(stat_after("Capacity"), None);
    }

    #[test]
    fn faction_multipliers() {
        let r =
            parse("x1.45 Damage to Corpus\nx0.62 Damage to Techrot\n+80% Critical Chance (x2 for Heavy Attacks)", &[]);
        let got: Vec<(&str, f64)> = r.stats.iter().map(|s| (s.stat, s.value)).collect();
        assert_eq!(got, [("Damage to Corpus", 45.0), ("Damage to Techrot", -38.0), ("Critical Chance", 80.0)]);
    }

    #[test]
    fn wrapped_and_noisy_names() {
        let r = parse("+57.5% Magazine . ~ v & Capacity +12% Multishot", &[]);
        let got: Vec<&str> = r.stats.iter().map(|s| s.stat).collect();
        assert_eq!(got, ["Magazine Capacity", "Multishot"]);
    }
}
