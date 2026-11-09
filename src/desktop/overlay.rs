//! The in-game overlays: the price panels under the reward cards and the riven grade panel.
//!
//! Native GTK widgets, updated directly when a scan finishes, so a re-shown panel never shows
//! the previous mission's rewards. Windows are hidden and re-shown, never destroyed.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib, pango};
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use super::Backend;
use crate::compositor::{self, Rect};
use crate::config::PanelPosition;
use crate::ocr::reward;
use crate::ocr::riven as riven_card;
use crate::rivens::{self, Graded};
use crate::state::{RewardItem, RewardResult, RivenResult, State};

const CSS: &str = r#"
window.wfp-overlay { background-color: transparent; }
"#;

/// The price panels' style for `u` pixels per unit of the reward screen, so they match the size
/// the game draws its cards at.
fn panel_css(u: f64) -> String {
    let px = |v: f64| format!("{:.1}px", v * u);
    format!(
        ".wfp-slot {{ background-color: rgba(8, 12, 17, 0.97); border: 1px solid rgba(150, 170, 186, 0.3); \
           border-radius: {radius}; padding: {pad_v} {pad_h}; }}\n\
         .wfp-slot.best {{ background-color: rgba(45, 37, 15, 0.97); border-color: #d9b45a; }}\n\
         .wfp-slot-name {{ font-family: \"Rajdhani\", sans-serif; font-weight: 600; font-size: {name}; color: #a9b7c2; }}\n\
         .wfp-best {{ font-family: \"Rajdhani\", sans-serif; font-weight: 700; font-size: {tag}; color: #1b1407; \
           background-color: #d9b45a; border-radius: 2px; padding: 0 {tag_pad}; }}\n\
         .wfp-plat {{ font-family: \"Rajdhani\", sans-serif; font-weight: 700; font-size: {price}; color: #f1d58b; }}\n\
         .wfp-plat.none {{ color: #6f7d89; }}\n\
         .wfp-ducats {{ font-family: \"Rajdhani\", sans-serif; font-weight: 600; font-size: {ducats}; color: #c9aa65; }}\n\
         .wfp-info {{ font-family: \"Source Sans 3\", \"Noto Sans\", sans-serif; font-size: {info}; color: #8a99a6; }}\n\
         .wfp-error {{ font-family: \"Source Sans 3\", \"Noto Sans\", sans-serif; font-size: {info}; color: #e2895f; }}\n\
         .wfp-riven {{ background-color: rgba(8, 12, 17, 0.97); border: 1px solid rgba(150, 170, 186, 0.3); \
           border-top: {accent} solid #9a80e8; border-radius: {radius}; padding: {pad_h}; }}\n\
         .wfp-weapon {{ font-family: \"Rajdhani\", sans-serif; font-weight: 700; font-size: {weapon}; color: #eeeae0; }}\n\
         .wfp-sub {{ font-family: \"Source Sans 3\", \"Noto Sans\", sans-serif; font-size: {info}; color: #8a99a6; }}\n\
         .wfp-overall {{ font-family: \"Rajdhani\", sans-serif; font-weight: 700; font-size: {overall}; \
           border-radius: {radius}; padding: 0 {tag_pad}; color: #0b1016; }}\n\
         .wfp-overall.a {{ background-color: #8ce39a; }} .wfp-overall.b {{ background-color: #d9b45a; }} \
         .wfp-overall.c {{ background-color: #e2895f; }}\n\
         .wfp-stat {{ font-family: \"Source Sans 3\", \"Noto Sans\", sans-serif; font-size: {stat}; color: #e9e4d6; }}\n\
         .wfp-grade {{ font-family: \"Rajdhani\", sans-serif; font-weight: 700; font-size: {grade}; }}\n\
         .wfp-grade.a {{ color: #8ce39a; }} .wfp-grade.b {{ color: #d9b45a; }} .wfp-grade.c {{ color: #e2895f; }}\n\
         .wfp-dev {{ font-family: \"Rajdhani\", sans-serif; font-weight: 600; font-size: {info}; color: #8a99a6; }}\n\
         .wfp-rule {{ border-top: 1px solid rgba(150, 170, 186, 0.18); padding-top: {pad_v}; }}\n\
         .wfp-price {{ font-family: \"Rajdhani\", sans-serif; font-weight: 700; font-size: {ducats}; color: #f1d58b; }}\n",
        radius = px(3.0),
        pad_v = px(5.0),
        pad_h = px(9.0),
        name = px(14.5),
        tag = px(12.0),
        tag_pad = px(4.0),
        price = px(27.0),
        ducats = px(16.0),
        info = px(13.0),
        accent = px(3.0),
        weapon = px(21.0),
        overall = px(20.0),
        stat = px(15.0),
        grade = px(17.0),
    )
}

fn escape(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.style_context().add_class(class);
    l.set_xalign(0.0);
    l
}

fn markup(m: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    l.set_markup(m);
    l.style_context().add_class(class);
    l.set_xalign(0.0);
    l
}

/// One overlay window: hidden, not destroyed, between uses.
struct Panel {
    window: gtk::Window,
    body: gtk::Box,
    hide_at: Cell<Option<Instant>>,
    held: Cell<bool>,
    ticking: Cell<bool>,
}

impl Panel {
    /// A click-through panel never takes the pointer: clicks land on the game under it.
    fn new(backend: Backend, namespace: &str, click_through: bool) -> Rc<Panel> {
        let window = match backend {
            Backend::LayerShell => {
                let w = gtk::Window::new(gtk::WindowType::Toplevel);
                w.init_layer_shell();
                w.set_layer(Layer::Overlay);
                w.set_namespace(namespace);
                // Never take keyboard focus from the game.
                w.set_keyboard_mode(KeyboardMode::None);
                w.set_anchor(Edge::Top, true);
                w.set_anchor(Edge::Left, true);
                // Place against the monitor edge itself, not the area left over by bars.
                w.set_exclusive_zone(-1);
                w
            }
            Backend::X11 => {
                // Override-redirect: no window manager placement, no focus, stacked above.
                let w = gtk::Window::new(gtk::WindowType::Popup);
                w.set_keep_above(true);
                w.set_accept_focus(false);
                w
            }
        };
        window.set_title("warframeproject overlay");
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_app_paintable(true);
        if let Some(visual) = WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
            window.set_visual(Some(&visual));
        }
        window.style_context().add_class("wfp-overlay");
        if click_through {
            // An empty input region: the panels sit over the reward screen while the cards are
            // being picked, and must never catch a click meant for the game.
            window.input_shape_combine_region(Some(&gtk::cairo::Region::create()));
        } else {
            window.add_events(gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);
        }

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let panel = Rc::new(Panel {
            window,
            body,
            hide_at: Cell::new(None),
            held: Cell::new(false),
            ticking: Cell::new(false),
        });
        panel.window.add(&panel.body);
        // Hovering keeps it open; leaving gives it three more seconds.
        let p = Rc::downgrade(&panel);
        panel.window.connect_enter_notify_event(move |_, _| {
            if let Some(p) = p.upgrade() {
                p.held.set(true);
            }
            glib::Propagation::Proceed
        });
        let p = Rc::downgrade(&panel);
        panel.window.connect_leave_notify_event(move |_, ev| {
            if let Some(p) = p.upgrade() {
                if ev.detail() != gdk::NotifyType::Inferior {
                    p.held.set(false);
                    let later = Instant::now() + Duration::from_secs(3);
                    p.hide_at.set(Some(p.hide_at.get().map_or(later, |t| t.max(later))));
                }
            }
            glib::Propagation::Proceed
        });
        panel
    }

    fn clear(&self) {
        for child in self.body.children() {
            self.body.remove(&child);
        }
    }

    fn visible(&self) -> bool {
        self.window.is_visible()
    }

    /// Moves the panel to `rect` (GDK coordinates).
    fn place(&self, backend: Backend, rect: Rect) {
        self.window.set_size_request(rect.width, rect.height);
        self.window.resize(rect.width, rect.height);
        match backend {
            Backend::LayerShell => {
                let display = gdk::Display::default();
                let monitor = display.as_ref().and_then(|d| monitor_at(d, rect.x, rect.y));
                let origin = monitor.as_ref().map_or((0, 0), |m| (m.geometry().x(), m.geometry().y()));
                if let Some(m) = &monitor {
                    self.window.set_monitor(m);
                }
                self.window.set_layer_shell_margin(Edge::Left, rect.x - origin.0);
                self.window.set_layer_shell_margin(Edge::Top, rect.y - origin.1);
            }
            Backend::X11 => self.window.move_(rect.x, rect.y),
        }
    }

    /// Shows the panel at `rect` (GDK coordinates) for `secs` seconds.
    fn show(self: &Rc<Self>, backend: Backend, rect: Rect, secs: u32) {
        self.place(backend, rect);
        self.held.set(false);
        self.hide_at.set(Some(Instant::now() + Duration::from_secs(secs.max(4) as u64)));
        self.window.show_all();
        if !self.ticking.replace(true) {
            let p = Rc::downgrade(self);
            glib::timeout_add_local(Duration::from_millis(200), move || {
                let Some(p) = p.upgrade() else { return glib::ControlFlow::Break };
                if !p.visible() {
                    p.ticking.set(false);
                    return glib::ControlFlow::Break;
                }
                if !p.held.get() && p.hide_at.get().is_some_and(|t| Instant::now() >= t) {
                    p.hide();
                    p.ticking.set(false);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }
    }

    fn hide(&self) {
        self.window.hide();
    }
}

fn monitor_at(display: &gdk::Display, x: i32, y: i32) -> Option<gdk::Monitor> {
    (0..display.n_monitors())
        .filter_map(|i| display.monitor(i))
        .find(|m| {
            let g = m.geometry();
            x >= g.x() && x < g.x() + g.width() && y >= g.y() && y < g.y() + g.height()
        })
        .or_else(|| display.primary_monitor())
        .or_else(|| display.monitor(0))
}

pub struct Overlays {
    backend: Backend,
    state: Arc<State>,
    strip: RefCell<Option<Rc<Panel>>>,
    riven: RefCell<Option<Rc<Panel>>>,
    /// The price panels' style, rewritten when the size of the cards changes.
    panel_css: gtk::CssProvider,
    panel_unit: Cell<f64>,
    /// The bundled currency icons, by name and height.
    icons: RefCell<HashMap<(&'static str, i32), gdk_pixbuf::Pixbuf>>,
}

impl Overlays {
    pub fn new(backend: Backend, state: Arc<State>) -> Overlays {
        let panel_css = gtk::CssProvider::new();
        let css = gtk::CssProvider::new();
        if css.load_from_data(CSS.as_bytes()).is_ok() {
            if let Some(screen) = gdk::Screen::default() {
                gtk::StyleContext::add_provider_for_screen(&screen, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
                gtk::StyleContext::add_provider_for_screen(
                    &screen,
                    &panel_css,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
                );
            }
        }
        Overlays {
            backend,
            state,
            strip: RefCell::new(None),
            riven: RefCell::new(None),
            panel_css,
            panel_unit: Cell::new(0.0),
            icons: RefCell::new(HashMap::new()),
        }
    }

    fn panel(&self, slot: &RefCell<Option<Rc<Panel>>>, namespace: &str, click_through: bool) -> Rc<Panel> {
        slot.borrow_mut().get_or_insert_with(|| Panel::new(self.backend, namespace, click_through)).clone()
    }

    /// Where Warframe's window is, in GDK coordinates.
    ///
    /// The compositor reports it in layout coordinates. GDK on Wayland uses the same ones; GDK on
    /// X11 uses XWayland's layout, which orders the monitors differently (here DP-1 sits at 0 for
    /// XWayland and at 4480 for Hyprland), so the rect is re-based onto the same monitor's X11
    /// origin. Without a game window, the primary monitor stands in.
    fn game_rect(&self) -> Rect {
        let display = gdk::Display::default();
        let fallback = || {
            display
                .as_ref()
                .and_then(|d| d.primary_monitor().or_else(|| d.monitor(0)))
                .map(|m| {
                    let g = m.geometry();
                    Rect { x: g.x(), y: g.y(), width: g.width(), height: g.height() }
                })
                .unwrap_or(Rect { x: 0, y: 0, width: 1920, height: 1080 })
        };
        let Some(game) = compositor::game_rect() else { return fallback() };
        if self.backend == Backend::LayerShell {
            return game;
        }
        let output = compositor::game_output();
        let on = compositor::monitors().into_iter().find(|m| output.as_deref() == Some(m.name.as_str()));
        let gdk_monitor = on.as_ref().and_then(|m| {
            let d = display.as_ref()?;
            (0..d.n_monitors()).filter_map(|i| d.monitor(i)).find(|g| g.model().is_some_and(|model| model == m.name))
        });
        match (on, gdk_monitor) {
            (Some(m), Some(g)) => {
                let o = g.geometry();
                Rect { x: o.x() + (game.x - m.x), y: o.y() + (game.y - m.y), ..game }
            }
            _ => game,
        }
    }

    pub fn hide_all(&self) {
        for slot in [&self.strip, &self.riven] {
            if let Some(p) = slot.borrow().as_ref() {
                p.hide();
            }
        }
    }

    /// The reward screen has closed: its prices are of no use any more.
    pub fn hide_rewards(&self) {
        if let Some(panel) = self.strip.borrow().as_ref() {
            panel.hide();
        }
    }

    // ------------------------------------------------------------ the price panels

    pub fn show_rewards(&self, r: &RewardResult, fresh: bool) {
        if !self.state.settings().overlay_enabled || (r.items.is_empty() && r.error.is_none()) {
            return;
        }
        self.put_rewards(r, fresh);
    }

    pub fn show_test(&self) {
        // The test panels always show, even with the reward overlay switched off.
        let scale = self.state.ui_scales().first().copied().unwrap_or(1.0);
        self.put_rewards(&RewardResult::sample(scale), true);
    }

    /// Lays one panel under each card, where the game draws the cards at the scale they were
    /// found at. Names found without cards get as many panels where that many cards would be.
    fn put_rewards(&self, r: &RewardResult, fresh: bool) {
        let panel = self.panel(&self.strip, "warframeproject-rewards", true);
        if !fresh && !panel.visible() {
            return; // a price update for panels already taken down with their screen
        }
        let settings = self.state.settings();
        let scale = r.scale.or_else(|| self.state.ui_scales().first().copied()).unwrap_or(1.0);
        // A message has no cards to sit under: one wide panel in the middle (`panel_places(0)`).
        let cards = if r.items.is_empty() { 0 } else { r.cards.unwrap_or(r.items.len()).max(1) };
        let g = self.game_rect();
        let u = reward::unit(g.width as f64, g.height as f64, scale);
        // Text smaller than this is unreadable; the panels then grow away from the cards.
        let text_u = u.max(0.75);
        let above = settings.overlay_position == PanelPosition::Above;
        let game = (g.x as f64, g.y as f64, g.width as f64, g.height as f64);
        let rects: Vec<[f64; 4]> = reward::panel_places(cards, above)
            .iter()
            .map(|p| reward::panel_rect(p, game, u, reward::PANEL_H * text_u))
            .collect();
        // One window around all the panels, each placed inside it.
        let (x0, y0) = (
            rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min),
            rects.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min),
        );
        let x1 = rects.iter().map(|r| r[0] + r[2]).fold(f64::NEG_INFINITY, f64::max);
        let y1 = rects.iter().map(|r| r[1] + r[3]).fold(f64::NEG_INFINITY, f64::max);
        let rect = Rect {
            x: x0.round() as i32,
            y: y0.round() as i32,
            width: (x1 - x0).round() as i32,
            height: (y1 - y0).round() as i32,
        };
        let put = |fixed: &gtk::Fixed, widget: &gtk::Box, r: &[f64; 4]| {
            widget.set_size_request(r[2].round() as i32, r[3].round() as i32);
            fixed.put(widget, (r[0] - x0).round() as i32, (r[1] - y0).round() as i32);
        };
        self.style_panels(text_u);
        panel.clear();
        let fixed = gtk::Fixed::new();
        if r.items.is_empty() {
            let message = r.error.as_deref().unwrap_or("No reward names recognized.");
            put(&fixed, &self.error_panel(message), &rects[0]);
        } else {
            for (k, place) in rects.iter().enumerate() {
                // Items read without cards are in order; a card read without a name stays empty.
                let item = r.items.iter().enumerate().find(|(i, it)| it.slot.unwrap_or(*i) == k).map(|(_, it)| it);
                if let Some(it) = item {
                    put(&fixed, &self.item_panel(it, r.best.as_deref() == Some(it.name.as_str()), text_u), place);
                }
            }
        }
        panel.body.add(&fixed);
        if fresh || !panel.visible() {
            panel.show(self.backend, rect, settings.overlay_seconds);
        } else {
            panel.place(self.backend, rect);
            panel.window.show_all();
        }
    }

    fn style_panels(&self, u: f64) {
        if (self.panel_unit.get() - u).abs() > 1e-3 && self.panel_css.load_from_data(panel_css(u).as_bytes()).is_ok() {
            self.panel_unit.set(u);
        }
    }

    /// One card's panel: the name it was read as (so a misread shows itself), the platinum it
    /// fetches, its ducats, and how many are owned, needed and whether it is vaulted.
    fn item_panel(&self, it: &RewardItem, best: bool, u: f64) -> gtk::Box {
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.style_context().add_class("wfp-slot");
        if best {
            panel.style_context().add_class("best");
        }
        let top = gtk::Box::new(gtk::Orientation::Horizontal, (6.0 * u) as i32);
        let name = label(&it.name, "wfp-slot-name");
        name.set_ellipsize(pango::EllipsizeMode::End);
        name.set_hexpand(true);
        top.add(&name);
        if best {
            let tag = label("BEST", "wfp-best");
            tag.set_valign(gtk::Align::Center);
            top.add(&tag);
        }
        panel.add(&top);

        let money = gtk::Box::new(gtk::Orientation::Horizontal, (4.0 * u) as i32);
        if let Some(icon) = self.icon("platinum", (20.0 * u).round() as i32) {
            money.add(&gtk::Image::from_pixbuf(Some(&icon)));
        }
        let price = label(&it.price.map_or("–".to_string(), |p| p.to_string()), "wfp-plat");
        if it.price.is_none() {
            price.style_context().add_class("none");
        }
        money.add(&price);
        if let Some(d) = it.ducats.filter(|d| *d > 0) {
            let ducats = gtk::Box::new(gtk::Orientation::Horizontal, (3.0 * u) as i32);
            ducats.set_hexpand(true);
            ducats.set_halign(gtk::Align::End);
            if let Some(icon) = self.icon("ducats", (16.0 * u).round() as i32) {
                ducats.add(&gtk::Image::from_pixbuf(Some(&icon)));
            }
            ducats.add(&label(&d.to_string(), "wfp-ducats"));
            money.add(&ducats);
        }
        panel.add(&money);

        let mut bits = vec![format!("owned {}", it.have)];
        if it.needed > it.have {
            bits.push("<span foreground=\"#6fd1c6\" weight=\"bold\">needed</span>".into());
        }
        if it.vaulted == Some(true) {
            bits.push("<span foreground=\"#e39a6b\">vaulted</span>".into());
        }
        let info = markup(&bits.join("  ·  "), "wfp-info");
        info.set_ellipsize(pango::EllipsizeMode::End);
        panel.add(&info);
        panel
    }

    fn error_panel(&self, message: &str) -> gtk::Box {
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 2);
        panel.style_context().add_class("wfp-slot");
        panel.add(&label("Reward scan", "wfp-slot-name"));
        let text = label(message, "wfp-error");
        text.set_line_wrap(true);
        panel.add(&text);
        panel
    }

    /// A bundled currency icon (`web/icons/<name>.png`), `px` pixels high.
    fn icon(&self, name: &'static str, px: i32) -> Option<gdk_pixbuf::Pixbuf> {
        let px = px.max(8);
        if let Some(icon) = self.icons.borrow().get(&(name, px)) {
            return Some(icon.clone());
        }
        let file = crate::server::Web::get(&format!("icons/{name}.png"))?;
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(file.data.into_owned()));
        let icon = gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, -1, px, true, gio::Cancellable::NONE).ok()?;
        self.icons.borrow_mut().insert((name, px), icon.clone());
        Some(icon)
    }

    // ------------------------------------------------------------ the riven panel

    /// The grades beside the riven card, which the game draws in the middle of the screen; the
    /// panel matches the price panels and is sized with the card. `fresh` is false when only the
    /// market listings arrived, which must not bring back a panel that has timed out.
    pub fn show_riven(&self, r: &RivenResult, fresh: bool) {
        let panel = self.panel(&self.riven, "warframeproject-riven", true);
        if !fresh && !panel.visible() {
            return;
        }
        let s = self.state.settings();
        let scale = self.state.ui_scales().first().copied().unwrap_or(1.0);
        let g = self.game_rect();
        let u = reward::unit(g.width as f64, g.height as f64, scale);
        let text_u = u.max(0.75);
        self.style_panels(text_u);
        panel.clear();
        let card = gtk::Box::new(gtk::Orientation::Vertical, (6.0 * text_u) as i32);
        card.style_context().add_class("wfp-riven");
        card.set_vexpand(true);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, (8.0 * text_u) as i32);
        if let Some(pic) = r.image.as_deref().and_then(|url| self.art(url, (40.0 * text_u) as i32)) {
            top.add(&gtk::Image::from_pixbuf(Some(&pic)));
        }
        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let weapon = label(r.weapon.as_deref().unwrap_or("Riven"), "wfp-weapon");
        weapon.set_ellipsize(pango::EllipsizeMode::End);
        names.add(&weapon);
        let sub = match (&r.error, r.positives, r.negatives) {
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
        };
        names.add(&label(&sub, "wfp-sub"));
        names.set_hexpand(true);
        top.add(&names);
        if let Some(grade) = r.overall {
            let badge = label(grade, "wfp-overall");
            badge.style_context().add_class(grade_class(grade));
            badge.set_valign(gtk::Align::Center);
            badge.set_tooltip_text(Some("The average roll"));
            top.add(&badge);
        }
        card.add(&top);

        if let Some(err) = &r.error {
            card.add(&wrapped(err, "wfp-error"));
        } else {
            for g in &r.stats {
                card.add(&stat_row(g, text_u));
            }
            if r.stats.iter().any(|g| g.out_of_range) {
                card.add(&wrapped(
                    "Some values are outside the ±10% a roll can land in: a stat was misread.",
                    "wfp-error",
                ));
            }
            if r.stats.iter().any(|g| g.sign_conflict) {
                card.add(&wrapped("A stat shows a sign it cannot roll with: the sign was misread.", "wfp-error"));
            }
            card.add(&self.riven_prices(r, text_u));
        }
        panel.body.add(&card);

        // Beside the card, clear of the "fits in" box further right.
        let (cx, cy) = (g.x as f64 + g.width as f64 / 2.0, g.y as f64 + g.height as f64 / 2.0);
        let width = (riven_card::PANEL_W * text_u).round() as i32;
        let rows = if r.error.is_some() { 1 } else { r.stats.len() };
        let height = (riven_card::panel_height(rows) * text_u).round() as i32;
        let beside = riven_card::CARD_HALF_W + riven_card::PANEL_GAP;
        let x = ((cx + beside * u).round() as i32).min(g.x + g.width - width - 8);
        let y = ((cy - riven_card::PANEL_TOP * u).round() as i32).min(g.y + g.height - height - 8).max(g.y + 8);
        let rect = Rect { x, y, width, height };
        if fresh {
            panel.show(self.backend, rect, s.riven_overlay_seconds.max(5));
        } else {
            panel.place(self.backend, rect);
            panel.window.show_all();
        }
    }

    /// What such rivens fetch: listings with the same positives on warframe.market (looked up
    /// after the grades are shown) and the weapon's recent trades.
    fn riven_prices(&self, r: &RivenResult, u: f64) -> gtk::Box {
        let block = gtk::Box::new(gtk::Orientation::Vertical, (3.0 * u) as i32);
        block.style_context().add_class("wfp-rule");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, (5.0 * u) as i32);
        if let Some(icon) = self.icon("platinum", (17.0 * u) as i32) {
            line.add(&gtk::Image::from_pixbuf(Some(&icon)));
        }
        match (&r.market, &r.market_error) {
            (Some(m), _) if m.count > 0 => {
                line.add(&label(&format!("{}", m.lowest.unwrap_or(0)), "wfp-price"));
                let more = match (m.median, m.lowest_online) {
                    (Some(med), Some(on)) => format!("lowest · {med} median · {on} in game · {} listed", m.count),
                    (Some(med), None) => format!("lowest · {med} median · {} listed", m.count),
                    _ => format!("lowest · {} listed", m.count),
                };
                let l = label(&more, "wfp-sub");
                l.set_ellipsize(pango::EllipsizeMode::End);
                line.add(&l);
            }
            (Some(_), _) => line.add(&label("No listings with these positives", "wfp-sub")),
            (None, Some(_)) => line.add(&label("warframe.market did not answer", "wfp-sub")),
            (None, None) => line.add(&label("Looking up listings…", "wfp-sub")),
        }
        block.add(&line);
        block.add(&label("warframe.market, same weapon and positives", "wfp-sub"));
        let median = |kind: &str| r.traded.as_ref().and_then(|t| t[kind]["median"].as_f64());
        let sold = match (median("unrolled"), median("rerolled")) {
            (Some(a), Some(b)) => Some(format!("Sold lately: {a:.0} unrolled · {b:.0} rerolled")),
            (Some(a), None) => Some(format!("Sold lately: {a:.0} unrolled")),
            (None, Some(b)) => Some(format!("Sold lately: {b:.0} rerolled")),
            (None, None) => None,
        };
        if let Some(sold) = sold {
            block.add(&label(&sold, "wfp-sub"));
        }
        block
    }

    /// An item picture from the art cache, if it is there: the overlay never waits for a download.
    fn art(&self, url: &str, px: i32) -> Option<gdk_pixbuf::Pixbuf> {
        let art = crate::art::cached(url, 128)?;
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(art.bytes));
        gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, px, px, true, gio::Cancellable::NONE).ok()
    }
}

fn grade_class(grade: &str) -> &'static str {
    match grade.chars().next() {
        Some('A') => "a",
        Some('B') => "b",
        _ => "c",
    }
}

fn wrapped(text: &str, class: &str) -> gtk::Label {
    let l = label(text, class);
    l.set_line_wrap(true);
    l.set_max_width_chars(40);
    l
}

/// One stat: its grade, the value as the card shows it, how far it is from an average roll, and
/// a bar showing where in the roll window (90-110% of the average) it landed.
fn stat_row(g: &Graded, u: f64) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, (3.0 * u) as i32);
    row.style_context().add_class("wfp-rule");
    let line = gtk::Box::new(gtk::Orientation::Horizontal, (8.0 * u) as i32);
    let grade = label(g.grade, "wfp-grade");
    grade.style_context().add_class(grade_class(g.grade));
    grade.set_width_chars(2);
    line.add(&grade);
    let sign = if g.negative { "−" } else { "+" };
    let unit = rivens::stat(g.stat).map_or("", |s| s.unit.suffix());
    let stat = markup(&format!("{sign}<b>{}{unit}</b> {}", g.value.abs(), escape(g.stat)), "wfp-stat");
    stat.set_hexpand(true);
    stat.set_ellipsize(pango::EllipsizeMode::End);
    line.add(&stat);
    line.add(&label(&format!("{}{:.1}%", if g.deviation > 0.0 { "+" } else { "" }, g.deviation), "wfp-dev"));
    row.add(&line);
    row.add(&roll_bar(g, u));
    row
}

/// The roll window as a track, with a mark where this roll landed; better rolls sit right.
fn roll_bar(g: &Graded, u: f64) -> gtk::DrawingArea {
    let bar = gtk::DrawingArea::new();
    bar.set_size_request(-1, (5.0 * u).max(3.0) as i32);
    // deviation runs from -10 (worst) to +10 (best).
    let at = ((g.deviation + 10.0) / 20.0).clamp(0.0, 1.0);
    let colour = match grade_class(g.grade) {
        "a" => (0.55, 0.89, 0.60),
        "b" => (0.85, 0.71, 0.35),
        _ => (0.89, 0.54, 0.37),
    };
    bar.connect_draw(move |w, cr| {
        let (width, height) = (w.allocated_width() as f64, w.allocated_height() as f64);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
        cr.rectangle(0.0, 0.0, width, height);
        let _ = cr.fill();
        cr.set_source_rgb(colour.0, colour.1, colour.2);
        cr.rectangle(0.0, 0.0, width * at, height);
        let _ = cr.fill();
        // The average roll.
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
        cr.rectangle(width / 2.0 - 0.5, 0.0, 1.0, height);
        let _ = cr.fill();
        glib::Propagation::Proceed
    });
    bar
}
