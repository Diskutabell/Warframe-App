//! The in-game overlays, drawn once for both platforms: where the panels go over the game, and
//! the panels themselves as pixels. Each platform only puts the pixels on screen in a window of
//! its own (`desktop::overlay` on Linux, `windows/desktop.rs` on Windows).
//!
//! Sizes are in units of the reward screen (`reward::unit`), so the panels match the cards the
//! game draws at any resolution and interface scale.

mod canvas;
mod panels;

use tiny_skia::Pixmap;

use crate::compositor::Rect;
use crate::ocr::{reward, riven};
use crate::services::scans::{RewardResult, RivenResult};

/// Text drawn at less than this many pixels per unit is unreadable: the panels then grow away
/// from the cards instead (in pixels of a display at 100%).
const MIN_TEXT_UNIT: f64 = 0.75;

/// Keeps a `width` x `height` box at (`x`, `y`) inside the game.
fn fit(game: Rect, width: i32, height: i32, x: i32, y: i32) -> Rect {
    let width = width.clamp(1, game.width.max(1));
    let height = height.clamp(1, game.height.max(1));
    Rect {
        x: x.clamp(game.x, game.x + game.width.max(1) - width),
        y: y.clamp(game.y, game.y + game.height.max(1) - height),
        width,
        height,
    }
}

/// Where the game is and how it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    /// The game's window, in the window system's coordinates.
    pub game: Rect,
    /// Pixels per unit of those coordinates: 2 on a doubled Wayland desktop (logical
    /// coordinates), 1 on Windows (physical ones).
    pub display: f64,
    /// The desktop's scale (1.5 = 150%): text is never drawn smaller than readable at it.
    pub dpi: f64,
    /// The game's interface scale (EE.cfg).
    pub ui_scale: f64,
}

impl Screen {
    /// Pixels of `game` per unit of the reward screen, and the same for text, which never goes
    /// below what is readable on the display.
    fn units(&self) -> (f64, f64) {
        let u = reward::unit(self.game.width as f64, self.game.height as f64, self.ui_scale);
        (u, u.max(MIN_TEXT_UNIT * self.dpi / self.display.max(0.1)))
    }
}

/// An overlay ready to show: where its window goes and what it shows. `rect` is in the
/// coordinates the game's rect was given in; `pixels` may be larger by the display scale.
pub struct Drawn {
    pub rect: Rect,
    pub pixels: Pixmap,
}

/// The price panels: one under (or above) each card, as the game draws the cards at the scale
/// they were found at; names found without cards get as many panels where that many cards would
/// be, and a message one wide panel in the middle.
///
pub fn rewards(r: &RewardResult, screen: Screen, above: bool) -> Drawn {
    let Screen { game, display, .. } = screen;
    let cards = if r.items.is_empty() { 0 } else { r.cards.unwrap_or(r.items.len()).max(1) };
    let (u, text_u) = screen.units();
    let g = (game.x as f64, game.y as f64, game.width as f64, game.height as f64);
    let rects: Vec<[f64; 4]> = reward::panel_places(cards, above)
        .iter()
        .map(|p| reward::panel_rect(p, g, u, reward::PANEL_H * text_u))
        .collect();
    let x0 = rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
    let y0 = rects.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
    let x1 = rects.iter().map(|r| r[0] + r[2]).fold(f64::NEG_INFINITY, f64::max);
    let y1 = rects.iter().map(|r| r[1] + r[3]).fold(f64::NEG_INFINITY, f64::max);
    let rect = fit(game, (x1 - x0).round() as i32, (y1 - y0).round() as i32, x0.round() as i32, y0.round() as i32);
    // Kept inside the game, the window may have moved; the panels move with it. In pixels.
    let boxes: Vec<[f32; 4]> = rects
        .iter()
        .map(|b| {
            let px = |v: f64| (v * display) as f32;
            [px(b[0] - rect.x as f64), px(b[1] - rect.y as f64), px(b[2]), px(b[3])]
        })
        .collect();
    let size = ((rect.width as f64 * display).round() as u32, (rect.height as f64 * display).round() as u32);
    let pixels = panels::rewards(r, size, &boxes, (text_u * display) as f32);
    Drawn { rect, pixels }
}

/// The grades beside the riven card, which the game draws in the middle of the screen, sized with
/// the card like the price panels.
pub fn riven(r: &RivenResult, screen: Screen) -> Drawn {
    let Screen { game, display, .. } = screen;
    let (u, text_u) = screen.units();
    let width = riven::PANEL_W * text_u;
    let pixels = panels::riven(r, (width * display).round() as u32, (text_u * display) as f32);
    let height = (pixels.height() as f64 / display).round() as i32;
    // Beside the card, clear of the "fits in" box further right.
    let x = (game.x as f64 + game.width as f64 / 2.0 + (riven::CARD_HALF_W + riven::PANEL_GAP) * u).round() as i32;
    let y = (game.y as f64 + game.height as f64 / 2.0 - riven::PANEL_TOP * u).round() as i32;
    Drawn { rect: fit(game, width.round() as i32, height, x, y), pixels }
}

/// How the panels look with made-up values, for the screenshots and the tests.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::scans::RewardResult;

    fn at(game: Rect, display: f64, ui_scale: f64) -> Screen {
        Screen { game, display, dpi: display, ui_scale }
    }

    fn ink(p: &Pixmap) -> usize {
        p.pixels().iter().filter(|c| c.alpha() > 0).count()
    }

    #[test]
    fn overlays_stay_inside_the_game_on_small_and_offset_screens() {
        let game = Rect { x: -1600, y: 200, width: 400, height: 300 };
        assert_eq!(fit(game, 1000, 168, -2000, 500), Rect { x: -1600, y: 332, width: 400, height: 168 });
        assert_eq!(fit(game, 470, 600, 0, -100), Rect { x: -1600, y: 200, width: 400, height: 300 });
    }

    #[test]
    fn panels_sit_under_the_cards_inside_the_game() {
        let game = Rect { x: 1920, y: 0, width: 2560, height: 1440 };
        let d = rewards(&RewardResult::sample(0.85), at(game, 1.0, 0.85), false);
        assert!(d.rect.x >= game.x && d.rect.x + d.rect.width <= game.x + game.width);
        assert!(d.rect.y >= game.y && d.rect.y + d.rect.height <= game.y + game.height);
        assert_eq!((d.pixels.width() as i32, d.pixels.height() as i32), (d.rect.width, d.rect.height));
        assert!(ink(&d.pixels) > d.pixels.width() as usize * d.pixels.height() as usize / 2, "four filled panels");
        // A doubled display draws twice the pixels for the same place.
        let hidpi = rewards(&RewardResult::sample(0.85), at(game, 2.0, 0.85), false);
        assert_eq!(hidpi.rect, d.rect);
        assert_eq!(hidpi.pixels.width(), 2 * d.pixels.width());
    }

    #[test]
    fn a_message_gets_one_panel_and_a_riven_its_own_height() {
        let game = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        let msg = RewardResult { error: Some("No reward names recognized.".into()), ..Default::default() };
        let d = rewards(&msg, at(game, 1.0, 1.0), false);
        assert!(ink(&d.pixels) > 0);
        let riv = RivenResult {
            weapon: Some("Sobek".into()),
            error: Some("Could not read the card.".into()),
            ..Default::default()
        };
        let d = riven(&riv, at(game, 1.0, 1.0));
        assert!(d.rect.x + d.rect.width <= 1920 && d.rect.height > 40, "{:?}", d.rect);
    }

    /// `WFP_OVERLAY_SHOTS=dir cargo test overlay -- --ignored` writes the panels as PNGs to look at.
    #[test]
    #[ignore]
    fn write_shots() {
        let dir = std::path::PathBuf::from(std::env::var("WFP_OVERLAY_SHOTS").unwrap());
        let game = Rect { x: 0, y: 0, width: 2560, height: 1440 };
        let mut r = RewardResult::sample(0.85);
        r.items[1].name = "Braton Prime Barrel with a name long enough to be cut".into();
        rewards(&r, at(game, 1.0, 0.85), false).pixels.save_png(dir.join("rewards.png")).unwrap();
        rewards(&r, at(Rect { x: 0, y: 0, width: 1280, height: 720 }, 1.0, 1.0), false)
            .pixels
            .save_png(dir.join("rewards-small.png"))
            .unwrap();
        let msg = RewardResult {
            error: Some("No reward names recognized on this screen, try again".into()),
            ..Default::default()
        };
        rewards(&msg, at(game, 1.0, 0.85), false).pixels.save_png(dir.join("message.png")).unwrap();
        riven(&panels::sample_riven(), at(game, 1.0, 0.85)).pixels.save_png(dir.join("riven.png")).unwrap();
    }
}
