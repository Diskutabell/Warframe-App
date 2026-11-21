//! The two overlays' look: the price panel under each reward card and the riven grade panel.
//! Sizes are multiples of `u`, the pixels per unit of the reward screen, so they grow with it.

use tiny_skia::Pixmap;

use super::canvas::{self, Canvas, Colour, Face, Style};
use crate::rivens::Graded;
use crate::services::scans::{RewardItem, RewardResult, RivenResult};

const NOTHING_READ: &str = "No reward names recognized.";
const PANEL: Colour = Colour(8, 12, 17, 0.97);
const PANEL_BEST: Colour = Colour(45, 37, 15, 0.97);
const EDGE: Colour = Colour(150, 170, 186, 0.3);
const RULE: Colour = Colour(150, 170, 186, 0.18);
const GOLD: Colour = Colour::hex(0xd9b45a);
const NAME: Colour = Colour::hex(0xa9b7c2);
const INK: Colour = Colour::hex(0x1b1407);
const PLAT: Colour = Colour::hex(0xf1d58b);
const NONE: Colour = Colour::hex(0x6f7d89);
const DUCATS: Colour = Colour::hex(0xc9aa65);
const MUTED: Colour = Colour::hex(0x8a99a6);
const NEEDED: Colour = Colour::hex(0x6fd1c6);
const VAULTED: Colour = Colour::hex(0xe39a6b);
const ERROR: Colour = Colour::hex(0xe2895f);
const RIVEN: Colour = Colour::hex(0x9a80e8);
const WEAPON: Colour = Colour::hex(0xeeeae0);
const STAT: Colour = Colour::hex(0xe9e4d6);
const DARK: Colour = Colour::hex(0x0b1016);

fn grade_colour(grade: &str) -> Colour {
    match grade.chars().next() {
        Some('A') => Colour::hex(0x8ce39a),
        Some('B') => GOLD,
        _ => ERROR,
    }
}

/// Text placed in a row `h` high from `top`, centred in it as GTK centres a label.
fn baseline(style: Style, top: f32, h: f32) -> f32 {
    top + (h - style.line()) / 2.0 + style.ascent()
}

/// A panel's box: the background, its outline, and the inside left for content.
fn frame(c: &mut Canvas, b: [f32; 4], u: f32, fill: Colour, edge: Colour) -> [f32; 4] {
    let [x, y, w, h] = b;
    c.fill(x, y, w, h, 3.0 * u, fill);
    c.outline(x, y, w, h, 3.0 * u, edge);
    [x + 1.0 + 9.0 * u, y + 1.0 + 5.0 * u, w - 2.0 - 18.0 * u, h - 2.0 - 10.0 * u]
}

// ---------------------------------------------------------------- the price panels

/// The price panels in a window `size` pixels big, one in each of `boxes` (x, y, width, height).
pub fn rewards(r: &RewardResult, size: (u32, u32), boxes: &[[f32; 4]], u: f32) -> Pixmap {
    let mut c = Canvas::new(size.0, size.1);
    if r.items.is_empty() {
        if let Some(&b) = boxes.first() {
            message(&mut c, b, u, r.error.as_deref().unwrap_or(NOTHING_READ));
        }
    } else {
        for (k, &b) in boxes.iter().enumerate() {
            // Items read without cards are in order; a card read without a name stays empty.
            let item = r.items.iter().enumerate().find(|(i, it)| it.slot.unwrap_or(*i) == k).map(|(_, it)| it);
            if let Some(it) = item {
                item_panel(&mut c, b, u, it, r.best.as_deref() == Some(it.name.as_str()));
            }
        }
    }
    c.pixmap
}

/// One card's panel: the name it was read as (so a misread shows itself), the platinum it
/// fetches, its ducats, and how many are owned, needed and whether it is vaulted.
/// Where its content ends, so a dry run measures how tall it has to be.
fn item_panel(c: &mut Canvas, b: [f32; 4], u: f32, it: &RewardItem, best: bool) -> f32 {
    let (fill, edge) = if best { (PANEL_BEST, GOLD) } else { (PANEL, EDGE) };
    let [x, mut y, w, _] = frame(c, b, u, fill, edge);

    // The name, and BEST at the right.
    let name = Style::new(Face::Label, 14.5 * u, NAME);
    let tag = Style::new(Face::Display, 12.0 * u, INK);
    let tag_w = if best { tag.width("BEST") + 8.0 * u } else { 0.0 };
    let h = if best { name.line().max(tag.line()) } else { name.line() };
    let room = w - if best { tag_w + 6.0 * u } else { 0.0 };
    // The whole name matters (it shows a misread): a second line before an ellipsis.
    let lines = name.wrap(&it.name, room);
    let first = lines.first().cloned().unwrap_or_default();
    let mut rest = lines.get(1..).unwrap_or_default().join(" ");
    // A first word wider than the room beside BEST: the whole name goes below instead.
    let first = if name.fit(&first, room).is_empty() {
        rest = lines.join(" ");
        String::new()
    } else {
        first
    };
    c.text(name, x, baseline(name, y, h), &name.fit(&first, room));
    if best {
        let tx = x + w - tag_w;
        let ty = y + (h - tag.line()) / 2.0;
        c.fill(tx, ty, tag_w, tag.line(), 2.0, GOLD);
        c.text(tag, tx + 4.0 * u, ty + tag.ascent(), "BEST");
    }
    y += h;
    if !rest.is_empty() {
        c.text(name, x, y + name.ascent(), &name.fit(&rest, w));
        y += name.line();
    }

    // Platinum on the left, ducats on the right. In a narrow panel the price gets smaller
    // before the two can meet, and the ducats give way if even that is not enough.
    let price_text = it.price.map_or("–".to_string(), |p| p.to_string());
    let plat_px = (20.0 * u).round() as u32;
    let plat = canvas::icon("platinum", plat_px);
    let plat_w = plat.as_ref().map_or(0.0, |i| i.width() as f32 + 4.0 * u);
    let ducat_style = Style::new(Face::Label, 16.0 * u, DUCATS);
    let ducat_text = it.ducats.filter(|d| *d > 0).map(|d| d.to_string());
    let ducat_icon = ducat_text.as_ref().and_then(|_| canvas::icon("ducats", (16.0 * u).round() as u32));
    let ducats_w = ducat_text.as_ref().map_or(0.0, |t| {
        ducat_style.width(t) + ducat_icon.as_ref().map_or(0.0, |i| i.width() as f32 + 3.0 * u) + 6.0 * u
    });
    let colour = if it.price.is_some() { PLAT } else { NONE };
    let mut price = Style::new(Face::Display, 27.0 * u, colour);
    let room = w - plat_w - ducats_w;
    if price.width(&price_text) > room {
        let px = (price.px * room / price.width(&price_text).max(1.0)).max(16.0 * u);
        price = Style::new(Face::Display, px, colour);
    }
    let show_ducats = plat_w + price.width(&price_text) + ducats_w <= w;
    let h = Style::new(Face::Display, 27.0 * u, colour).line().max(plat_px as f32);
    let mut px = x;
    if let Some(icon) = &plat {
        c.picture(icon, px, y + (h - icon.height() as f32) / 2.0);
        px += plat_w;
    }
    c.text(price, px, baseline(price, y, h), &price.fit(&price_text, w - plat_w));
    if let (Some(text), true) = (&ducat_text, show_ducats) {
        let mut dx = x + w - ducat_style.width(text);
        c.text(ducat_style, dx, baseline(ducat_style, y, h), text);
        if let Some(icon) = &ducat_icon {
            dx -= icon.width() as f32 + 3.0 * u;
            c.picture(icon, dx, y + (h - icon.height() as f32) / 2.0);
        }
    }
    y += h;

    // owned N · needed · vaulted; in a narrow panel "N owned · need · vault", still in colour.
    let plain = Style::new(Face::Text, 13.0 * u, MUTED);
    let status = |short: bool| {
        let sep = if short { " · " } else { "  ·  " };
        let mut runs = vec![(plain, if short { format!("{} owned", it.have) } else { format!("owned {}", it.have) })];
        if it.needed > it.have {
            runs.push((plain, sep.into()));
            runs.push((Style::new(Face::TextBold, 13.0 * u, NEEDED), if short { "need" } else { "needed" }.into()));
        }
        if it.vaulted == Some(true) {
            runs.push((plain, sep.into()));
            runs.push((Style::new(Face::Text, 13.0 * u, VAULTED), if short { "vault" } else { "vaulted" }.into()));
        }
        runs
    };
    let width = |runs: &[(Style, String)]| runs.iter().map(|(s, t)| s.width(t)).sum::<f32>();
    let mut runs = status(false);
    if width(&runs) > w {
        runs = status(true);
    }
    let line = plain.line();
    if width(&runs) > w {
        let all: String = runs.iter().map(|(_, t)| t.as_str()).collect();
        c.text(plain, x, baseline(plain, y, line), &plain.fit(&all, w));
    } else {
        let mut px = x;
        for (style, text) in &runs {
            px = c.text(*style, px, baseline(plain, y, line), text);
        }
    }
    y + line
}

/// How tall the price panels have to be, `widths` wide (pixels), so every one shows all it has:
/// the tallest one's height, for all of them alike.
pub fn rewards_height(r: &RewardResult, widths: &[f32], u: f32) -> f32 {
    if r.items.is_empty() {
        let text = r.error.as_deref().unwrap_or(NOTHING_READ);
        return widths
            .first()
            .map_or(0.0, |&w| message(&mut Canvas::measure(), [0.0, 0.0, w, 0.0], u, text) + 5.0 * u + 1.0);
    }
    let mut tallest: f32 = 0.0;
    for (k, &w) in widths.iter().enumerate() {
        let item = r.items.iter().enumerate().find(|(i, it)| it.slot.unwrap_or(*i) == k).map(|(_, it)| it);
        if let Some(it) = item {
            let end = item_panel(
                &mut Canvas::measure(),
                [0.0, 0.0, w, 0.0],
                u,
                it,
                r.best.as_deref() == Some(it.name.as_str()),
            );
            tallest = tallest.max(end + 5.0 * u + 1.0);
        }
    }
    tallest
}

/// A message in place of the panels: why nothing was recognised.
/// Where its text ends.
fn message(c: &mut Canvas, b: [f32; 4], u: f32, text: &str) -> f32 {
    let [x, mut y, w, _] = frame(c, b, u, PANEL, EDGE);
    let title = Style::new(Face::Label, 14.5 * u, NAME);
    c.text(title, x, y + title.ascent(), "Reward scan");
    y += title.line() + 2.0;
    let body = Style::new(Face::Text, 13.0 * u, ERROR);
    for line in body.wrap(text, w) {
        c.text(body, x, y + body.ascent(), &line);
        y += body.line();
    }
    y
}

// ---------------------------------------------------------------- the riven panel

/// The riven panel, `width` pixels wide and as tall as what it shows.
/// Taller than `max_height` it is cut off (a very small game window).
pub fn riven(r: &RivenResult, width: u32, max_height: u32, u: f32) -> Pixmap {
    // Laid out once without drawing to learn its height, then drawn.
    let pic = r.image.as_deref().and_then(|url| canvas::art(url, (40.0 * u).round() as u32));
    let height = riven_into(&mut Canvas::measure(), r, pic.as_ref(), width as f32, None, u);
    let height = height.min(max_height as f32);
    let mut c = Canvas::new(width, height.ceil() as u32);
    riven_into(&mut c, r, pic.as_ref(), width as f32, Some(height), u);
    c.pixmap
}

/// Draws the panel `w` wide (the background only when its `height` is known); its height.
fn riven_into(c: &mut Canvas, r: &RivenResult, pic: Option<&Pixmap>, w: f32, height: Option<f32>, u: f32) -> f32 {
    if let Some(h) = height {
        c.fill(0.0, 0.0, w, h, 3.0 * u, PANEL);
        c.outline(0.0, 0.0, w, h, 3.0 * u, EDGE);
        c.fill_top(0.0, 0.0, w, 3.0 * u, 3.0 * u, RIVEN);
    }
    let pad = 9.0 * u;
    let x = 1.0 + pad;
    let iw = w - 2.0 - 2.0 * pad;
    let gap = 6.0 * u;
    let mut y = 3.0 * u + pad;

    // The weapon's picture and name, what was read, and the overall grade.
    let weapon = Style::new(Face::Display, 21.0 * u, WEAPON);
    let sub = Style::new(Face::Text, 13.0 * u, MUTED);
    let badge = Style::new(Face::Display, 20.0 * u, DARK);
    let pic_w = pic.map_or(0.0, |p| p.width() as f32 + 8.0 * u);
    let badge_w = r.overall.map_or(0.0, |g| badge.width(g) + 8.0 * u);
    let h = (weapon.line() + sub.line()).max(pic.map_or(0.0, |p| p.height() as f32)).max(badge.line());
    if let Some(p) = pic {
        c.picture(p, x, y + (h - p.height() as f32) / 2.0);
    }
    let names = iw - pic_w - if badge_w > 0.0 { badge_w + 8.0 * u } else { 0.0 };
    c.text(weapon, x + pic_w, y + weapon.ascent(), &weapon.fit(r.weapon.as_deref().unwrap_or("Riven"), names));
    c.text(sub, x + pic_w, y + weapon.line() + sub.ascent(), &sub.fit(&summary(r), names));
    if let Some(grade) = r.overall {
        let bx = x + iw - badge_w;
        let by = y + (h - badge.line()) / 2.0;
        c.fill(bx, by, badge_w, badge.line(), 3.0 * u, grade_colour(grade));
        c.text(badge, bx + 4.0 * u, by + badge.ascent(), grade);
    }
    y += h;

    let warn = Style::new(Face::Text, 13.0 * u, ERROR);
    let wrapped = |c: &mut Canvas, y: &mut f32, text: &str| {
        *y += gap;
        for line in warn.wrap(text, iw) {
            c.text(warn, x, *y + warn.ascent(), &line);
            *y += warn.line();
        }
    };
    if let Some(err) = &r.error {
        wrapped(c, &mut y, err);
        return y + pad + 1.0;
    }
    for g in &r.stats {
        y += gap;
        y = stat_row(c, g, x, y, iw, u);
    }
    if r.stats.iter().any(|g| g.out_of_range) {
        wrapped(c, &mut y, "Some values are outside the ±10% a roll can land in: a stat was misread.");
    }
    if r.stats.iter().any(|g| g.sign_conflict) {
        wrapped(c, &mut y, "A stat shows a sign it cannot roll with: the sign was misread.");
    }
    y += gap;
    y = prices(c, r, x, y, iw, u);
    y + pad + 1.0
}

/// "3 positives + 1 negative · 1 not readable"
fn summary(r: &RivenResult) -> String {
    match (&r.error, r.positives, r.negatives) {
        (Some(_), _, _) => "Riven".to_string(),
        (None, Some(p), Some(n)) => {
            let mut t = format!("{p} positives{}", if n > 0 { " + 1 negative" } else { "" });
            match r.unread {
                0 => {}
                1 => t.push_str(" · 1 not readable"),
                k => t.push_str(&format!(" · {k} not readable")),
            }
            t
        }
        _ => String::new(),
    }
}

/// A line above a block, and the space under it.
fn rule(c: &mut Canvas, x: f32, y: f32, w: f32, u: f32) -> f32 {
    c.fill(x, y, w, 1.0, 0.0, RULE);
    y + 1.0 + 5.0 * u
}

/// One stat: its grade, the value as the card shows it, how far it is from an average roll, and
/// a bar showing where in the roll window (90-110% of the average) it landed. Where it ends.
fn stat_row(c: &mut Canvas, g: &Graded, x: f32, y: f32, w: f32, u: f32) -> f32 {
    let mut y = rule(c, x, y, w, u);
    let grade = Style::new(Face::Display, 17.0 * u, grade_colour(g.grade));
    let value = Style::new(Face::TextBold, 15.0 * u, STAT);
    let name = Style::new(Face::Text, 15.0 * u, STAT);
    let dev = Style::new(Face::Label, 13.0 * u, MUTED);
    let h = grade.line().max(value.line()).max(dev.line());
    c.text(grade, x, baseline(grade, y, h), g.grade);
    let grade_w = grade.width("00") + 8.0 * u;
    let deviation = format!("{}{:.1}%", if g.deviation > 0.0 { "+" } else { "" }, g.deviation);
    let dev_w = dev.width(&deviation);
    c.text(dev, x + w - dev_w, baseline(dev, y, h), &deviation);
    // The value in bold, then the stat, cut to the room between grade and deviation.
    let room = w - grade_w - dev_w - 8.0 * u;
    let printed = format!("{} ", g.printed);
    let sx = x + grade_w;
    if value.width(&printed) >= room {
        c.text(value, sx, baseline(value, y, h), &value.fit(&g.printed, room));
    } else {
        let after = c.text(value, sx, baseline(value, y, h), &printed);
        c.text(name, after, baseline(value, y, h), &name.fit(g.stat, room - (after - sx)));
    }
    y += h + 3.0 * u;

    // The roll window as a track; better rolls sit right. deviation runs from -10 to +10.
    let bar = (5.0 * u).max(3.0);
    let at = ((g.deviation + 10.0) / 20.0).clamp(0.0, 1.0) as f32;
    c.fill(x, y, w, bar, 0.0, Colour(255, 255, 255, 0.08));
    c.fill(x, y, w * at, bar, 0.0, grade_colour(g.grade));
    // The average roll.
    c.fill(x + w / 2.0 - 0.5, y, 1.0, bar, 0.0, Colour(255, 255, 255, 0.35));
    y + bar
}

/// What such rivens fetch: listings with the same positives on warframe.market (looked up after
/// the grades are shown) and the weapon's recent trades. Where it ends.
fn prices(c: &mut Canvas, r: &RivenResult, x: f32, y: f32, w: f32, u: f32) -> f32 {
    let mut y = rule(c, x, y, w, u);
    let sub = Style::new(Face::Text, 13.0 * u, MUTED);
    let price = Style::new(Face::Display, 16.0 * u, PLAT);
    let icon_px = (17.0 * u).round() as u32;
    let h = price.line().max(sub.line()).max(icon_px as f32);
    let mut px = x;
    if let Some(icon) = canvas::icon("platinum", icon_px) {
        c.picture(&icon, px, y + (h - icon.height() as f32) / 2.0);
        px += icon.width() as f32 + 5.0 * u;
    }
    let text = match (&r.market, &r.market_error) {
        (Some(m), _) if m.count > 0 => {
            px = c.text(price, px, baseline(price, y, h), &m.lowest.unwrap_or(0).to_string()) + 5.0 * u;
            match (m.median, m.lowest_online) {
                (Some(med), Some(on)) => format!("lowest · {med} median · {on} in game · {} listed", m.count),
                (Some(med), None) => format!("lowest · {med} median · {} listed", m.count),
                _ => format!("lowest · {} listed", m.count),
            }
        }
        (Some(_), _) => "No listings with these positives".into(),
        (None, Some(_)) => "warframe.market did not answer".into(),
        // Listings are looked up by the weapon and its positive stats (ScanService::
        // find_riven_listings); without them nothing is asked.
        (None, None)
            if r.weapon.is_none()
                || !r.stats.iter().any(|g| !g.negative && crate::rivens::market_slug(g.stat).is_some()) =>
        {
            "No positive stats to look listings up by".into()
        }
        (None, None) => "Looking up listings…".into(),
    };
    c.text(sub, px, baseline(sub, y, h), &sub.fit(&text, x + w - px));
    y += h + 3.0 * u;
    let line = |c: &mut Canvas, y: &mut f32, text: &str| {
        c.text(sub, x, *y + sub.ascent(), &sub.fit(text, w));
        *y += sub.line() + 3.0 * u;
    };
    line(c, &mut y, "warframe.market, same weapon and positives");
    let median = |p: Option<&crate::rivens::RivenPrices>| p.and_then(|p| p.median.as_ref()?.as_f64());
    let traded = r.traded.as_ref();
    let sold =
        match (median(traded.and_then(|t| t.unrolled.as_ref())), median(traded.and_then(|t| t.rerolled.as_ref()))) {
            (Some(a), Some(b)) => Some(format!("Sold lately: {a:.0} unrolled · {b:.0} rerolled")),
            (Some(a), None) => Some(format!("Sold lately: {a:.0} unrolled")),
            (None, Some(b)) => Some(format!("Sold lately: {b:.0} rerolled")),
            (None, None) => None,
        };
    if let Some(sold) = sold {
        line(c, &mut y, &sold);
    }
    y - 3.0 * u
}

/// A riven as a scan reads one, for the tests and the screenshots.
#[cfg(test)]
pub fn sample_riven() -> RivenResult {
    let stat = |stat, printed: &str, deviation, grade, out_of_range| Graded {
        stat,
        value: 0.0,
        centre: 0.0,
        deviation,
        grade,
        negative: false,
        inverted: false,
        printed: printed.into(),
        out_of_range,
        sign_conflict: false,
    };
    RivenResult {
        weapon: Some("Sobek".into()),
        positives: Some(3),
        negatives: Some(1),
        stats: vec![
            stat("Critical Chance", "+182.3%", 6.4, "A", false),
            stat("Multishot", "+101.6%", -2.1, "B-", false),
            stat("Damage to Corpus", "x1.45", 9.5, "A+", false),
            stat("Zoom", "-62.5%", -12.0, "C-", true),
        ],
        overall: Some("B+"),
        market: Some(crate::market::RivenListings {
            count: 12,
            lowest: Some(140),
            median: Some(260),
            lowest_online: Some(180),
            url: String::new(),
        }),
        ..Default::default()
    }
}
