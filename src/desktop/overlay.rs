//! The in-game overlays: the reward price strip and the riven grade panel.
//!
//! Native GTK widgets, updated directly when a scan finishes, so a re-shown strip never shows
//! the previous mission's rewards. Windows are hidden and re-shown, never destroyed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib, pango};
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use super::Backend;
use crate::compositor::{self, Rect};
use crate::rivens::Graded;
use crate::state::{RewardItem, RewardResult, RivenResult, State};

const CSS: &str = r#"
window.wfp-overlay { background-color: transparent; }
.wfp-bar { background-color: rgba(14, 22, 29, 0.92); border: 1px solid #2d4150; border-radius: 4px; padding: 6px; }
.wfp-card { background-color: rgba(33, 50, 64, 0.55); border-top: 3px solid #2d4150; padding: 8px 12px; }
.wfp-card.best { border-top-color: #d4b26a; background-color: rgba(56, 52, 34, 0.6); }
.wfp-name { font-family: "Rajdhani", "Arial Narrow", sans-serif; font-weight: 600; font-size: 15px; color: #e9e4d6; }
.wfp-price { font-family: "Rajdhani", "Arial Narrow", sans-serif; font-weight: 700; font-size: 34px; color: #d4b26a; }
.wfp-line, .wfp-muted { font-family: "Source Sans 3", "Noto Sans", sans-serif; font-size: 13px; color: #a3b0b8; }
.wfp-close { color: #a3b0b8; background: none; border: none; box-shadow: none; padding: 0 6px; min-height: 0; min-width: 0; font-size: 18px; }
.wfp-close:hover { color: #e9e4d6; }
.wfp-panel { background-color: rgba(8, 10, 14, 0.93); border: 1px solid #2d4150; border-left: 3px solid #d4b26a; border-radius: 4px; padding: 14px 16px; }
.wfp-weapon { font-family: "Rajdhani", sans-serif; font-weight: 700; font-size: 23px; color: #eeeeee; }
.wfp-dots { color: #d4b26a; font-size: 15px; }
.wfp-stat { font-family: "Source Sans 3", sans-serif; font-size: 16px; color: #eeeeee; }
.wfp-row { border-top: 1px solid #1e2b36; padding: 6px 0; }
.wfp-grade { font-family: "Rajdhani", sans-serif; font-weight: 700; font-size: 17px; }
.wfp-grade.a { color: #8ce39a; } .wfp-grade.b { color: #d4b26a; } .wfp-grade.c { color: #e2895f; }
.wfp-warn { color: #e2895f; font-size: 13px; }
.wfp-foot { color: #6d7b88; font-size: 11.5px; }
"#;

const DOTS: [&str; 6] = ["", "●○○○○", "●●○○○", "●●●○○", "●●●●○", "●●●●●"];
const STRIP_W: i32 = 1000;
const STRIP_H: i32 = 168;
const RIVEN_W: i32 = 470;

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
    fn new(backend: Backend, namespace: &str, close: bool) -> Rc<Panel> {
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
        window.add_events(gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);

        let layers = gtk::Overlay::new();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layers.add(&body);
        let panel = Rc::new(Panel {
            window,
            body,
            hide_at: Cell::new(None),
            held: Cell::new(false),
            ticking: Cell::new(false),
        });
        if close {
            let x = gtk::Button::with_label("×");
            x.style_context().add_class("wfp-close");
            x.set_halign(gtk::Align::End);
            x.set_valign(gtk::Align::Start);
            x.set_tooltip_text(Some("Hide"));
            let p = Rc::downgrade(&panel);
            x.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.hide();
                }
            });
            layers.add_overlay(&x);
        }
        panel.window.add(&layers);
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

    /// Shows the panel at `rect` (GDK coordinates) for `secs` seconds.
    fn show(self: &Rc<Self>, backend: Backend, rect: Rect, secs: u32) {
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
}

impl Overlays {
    pub fn new(backend: Backend, state: Arc<State>) -> Overlays {
        let css = gtk::CssProvider::new();
        if css.load_from_data(CSS.as_bytes()).is_ok() {
            if let Some(screen) = gdk::Screen::default() {
                gtk::StyleContext::add_provider_for_screen(&screen, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }
        }
        Overlays { backend, state, strip: RefCell::new(None), riven: RefCell::new(None) }
    }

    fn panel(&self, slot: &RefCell<Option<Rc<Panel>>>, namespace: &str, close: bool) -> Rc<Panel> {
        slot.borrow_mut().get_or_insert_with(|| Panel::new(self.backend, namespace, close)).clone()
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

    // ------------------------------------------------------------ the reward strip

    pub fn show_rewards(&self, r: &RewardResult, fresh: bool) {
        let s = self.state.settings();
        if !s.overlay_enabled || (r.items.is_empty() && r.error.is_none()) {
            return;
        }
        let panel = self.panel(&self.strip, "warframeproject-rewards", true);
        if !fresh && !panel.visible() {
            return; // a price update for a strip the user already closed
        }
        if r.items.is_empty() {
            panel.clear();
            let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
            body.style_context().add_class("wfp-panel");
            body.add(&label("Reward scan", "wfp-weapon"));
            let message = label(r.error.as_deref().unwrap_or("No reward names recognized."), "wfp-warn");
            message.set_line_wrap(true);
            body.add(&message);
            panel.body.add(&body);
        } else {
            self.fill_strip(&panel, &r.items, r.best.as_deref());
        }
        if fresh || !panel.visible() {
            let g = self.game_rect();
            let w = STRIP_W.min(g.width - 40).max(300);
            let y = g.y + (g.height as f64 * s.overlay_y_percent / 100.0) as i32;
            panel.show(
                self.backend,
                Rect { x: g.x + (g.width - w) / 2, y, width: w, height: STRIP_H },
                s.overlay_seconds,
            );
        } else {
            panel.window.show_all();
        }
    }

    fn fill_strip(&self, panel: &Panel, items: &[RewardItem], best: Option<&str>) {
        panel.clear();
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.style_context().add_class("wfp-bar");
        bar.set_homogeneous(true);
        bar.set_vexpand(true);
        for it in items {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
            card.style_context().add_class("wfp-card");
            if best == Some(it.name.as_str()) {
                card.style_context().add_class("best");
            }
            let name = label(&it.name, "wfp-name");
            name.set_line_wrap(true);
            name.set_lines(2);
            name.set_ellipsize(pango::EllipsizeMode::End);
            name.set_height_request(36);
            name.set_valign(gtk::Align::Start);
            card.add(&name);
            let price = it.price.map_or("?".to_string(), |p| p.to_string());
            card.add(&markup(&format!("{price}<span size=\"small\" foreground=\"#a3b0b8\">p</span>"), "wfp-price"));
            let mut bits = Vec::new();
            if let Some(d) = it.ducats.filter(|d| *d > 0) {
                bits.push(format!("{d}d"));
            }
            bits.push(format!("own {}", it.have));
            if it.needed > it.have {
                bits.push("<span foreground=\"#7fc8c2\" weight=\"600\">needed</span>".into());
            }
            if it.vaulted == Some(true) {
                bits.push("<span foreground=\"#df8a63\">vaulted</span>".into());
            }
            let line = markup(&bits.join(" · "), "wfp-line");
            line.set_ellipsize(pango::EllipsizeMode::End);
            card.add(&line);
            bar.add(&card);
        }
        panel.body.add(&bar);
    }

    pub fn show_test(&self) {
        let r = RewardResult::sample();
        // The test strip always shows, even with the reward overlay switched off.
        let panel = self.panel(&self.strip, "warframeproject-rewards", true);
        self.fill_strip(&panel, &r.items, r.best.as_deref());
        let s = self.state.settings();
        let g = self.game_rect();
        let w = STRIP_W.min(g.width - 40).max(300);
        let y = g.y + (g.height as f64 * s.overlay_y_percent / 100.0) as i32;
        panel.show(self.backend, Rect { x: g.x + (g.width - w) / 2, y, width: w, height: STRIP_H }, s.overlay_seconds);
    }

    // ------------------------------------------------------------ the riven panel

    pub fn show_riven(&self, r: &RivenResult) {
        let s = self.state.settings();
        let panel = self.panel(&self.riven, "warframeproject-riven", false);
        panel.clear();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.style_context().add_class("wfp-panel");
        card.set_vexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        top.add(&label(r.weapon.as_deref().unwrap_or("Riven"), "wfp-weapon"));
        if let Some(d) = r.disposition {
            let dots = DOTS.get(r.dots.unwrap_or(0) as usize).copied().unwrap_or("");
            let l = label(&format!("{dots} {d}×"), "wfp-dots");
            l.set_hexpand(true);
            l.set_xalign(1.0);
            top.add(&l);
        }
        card.add(&top);
        let rows = if let Some(err) = &r.error {
            card.add(&label(err, "wfp-muted"));
            1
        } else {
            let (p, n) = (r.positives.unwrap_or(0), r.negatives.unwrap_or(0));
            let plural = if p == 1 { "" } else { "s" };
            let unread = match r.unread {
                0 => String::new(),
                1 => " · 1 stat not readable".to_string(),
                k => format!(" · {k} stats not readable"),
            };
            card.add(&label(
                &format!("{p} positive{plural}{}{unread}", if n > 0 { " + negative" } else { "" }),
                "wfp-muted",
            ));
            for g in &r.stats {
                card.add(&stat_row(g));
            }
            if r.stats.iter().any(|g| g.out_of_range) {
                card.add(&label("Values outside ±10%: the stat count or weapon looks wrong.", "wfp-warn"));
            }
            if r.stats.iter().any(|g| g.sign_conflict) {
                card.add(&label("A stat shows a sign it cannot roll with: the sign was misread.", "wfp-warn"));
            }
            card.add(&label("Grades compare the roll with an average roll, not with what is useful.", "wfp-foot"));
            r.stats.len().max(1) as i32
        };
        panel.body.add(&card);

        // Beside the card, which the game draws centred.
        let g = self.game_rect();
        let h = 118 + 40 * rows;
        let mut x = g.x + (g.width as f64 * s.riven_overlay_x_percent / 100.0) as i32;
        let mut y = g.y + (g.height as f64 * s.riven_overlay_y_percent / 100.0) as i32 - h / 2;
        x = x.min(g.x + g.width - RIVEN_W - 20);
        y = y.min(g.y + g.height - h - 20).max(g.y + 20);
        panel.show(self.backend, Rect { x, y, width: RIVEN_W, height: h }, s.riven_overlay_seconds.max(5));
    }
}

fn stat_row(g: &Graded) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.style_context().add_class("wfp-row");
    let sign = if g.negative { "−" } else { "+" };
    let stat = markup(&format!("{sign}<b>{}</b> {}", g.value.abs(), escape(g.stat)), "wfp-stat");
    stat.set_hexpand(true);
    row.add(&stat);
    let grade = label(g.grade, "wfp-grade");
    grade.style_context().add_class(match g.grade.chars().next() {
        Some('A') => "a",
        Some('B') => "b",
        _ => "c",
    });
    row.add(&grade);
    let dev = label(&format!("{}{:.2}%", if g.deviation > 0.0 { "+" } else { "" }, g.deviation), "wfp-muted");
    dev.set_width_chars(8);
    dev.set_xalign(1.0);
    row.add(&dev);
    row
}
