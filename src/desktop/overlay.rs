//! The in-game overlays on Linux: windows that show what `crate::overlay` draws, over the game.
//!
//! Updated directly when a scan finishes, so a re-shown panel never shows the previous mission's
//! rewards. Windows are hidden and re-shown, never destroyed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use super::Backend;
use crate::app::App;
use crate::compositor::{self, Rect};
use crate::config::PanelPosition;
use crate::overlay::{self, Drawn, Screen};
use crate::services::scans::{RewardResult, RivenResult};

const CSS: &str = r#"
window.wfp-overlay { background-color: transparent; }
"#;

/// The drawn panels as cairo wants them: premultiplied ARGB in native byte order, at the
/// monitor's scale.
fn surface(d: &Drawn, scale: i32) -> Option<cairo::ImageSurface> {
    let (w, h) = (d.pixels.width() as i32, d.pixels.height() as i32);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        for (y, row) in d.pixels.pixels().chunks_exact(w as usize).enumerate() {
            for (x, p) in row.iter().enumerate() {
                let argb = u32::from_be_bytes([p.alpha(), p.red(), p.green(), p.blue()]);
                data[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&argb.to_ne_bytes());
            }
        }
    }
    surface.mark_dirty();
    surface.set_device_scale(scale as f64, scale as f64);
    Some(surface)
}

/// One overlay window: hidden, not destroyed, between uses.
struct Panel {
    window: gtk::Window,
    /// What the window shows.
    shown: Rc<RefCell<Option<cairo::ImageSurface>>>,
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

        let shown: Rc<RefCell<Option<cairo::ImageSurface>>> = Rc::default();
        let area = gtk::DrawingArea::new();
        let pixels = shown.clone();
        area.connect_draw(move |_, cr| {
            if let Some(s) = pixels.borrow().as_ref() {
                cr.set_operator(cairo::Operator::Source);
                if cr.set_source_surface(s, 0.0, 0.0).is_ok() {
                    let _ = cr.paint();
                }
            }
            glib::Propagation::Stop
        });
        window.add(&area);
        let panel = Rc::new(Panel {
            window,
            shown,
            hide_at: Cell::new(None),
            held: Cell::new(false),
            ticking: Cell::new(false),
        });
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

    /// Shows `drawn` from now on.
    fn set(&self, drawn: &Drawn, scale: i32) {
        *self.shown.borrow_mut() = surface(drawn, scale);
        self.window.queue_draw();
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
    state: Arc<App>,
    strip: RefCell<Option<Rc<Panel>>>,
    riven: RefCell<Option<Rc<Panel>>>,
}

impl Overlays {
    pub fn new(backend: Backend, state: Arc<App>) -> Overlays {
        let css = gtk::CssProvider::new();
        if css.load_from_data(CSS.as_bytes()).is_ok() {
            if let Some(screen) = gdk::Screen::default() {
                gtk::StyleContext::add_provider_for_screen(&screen, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            }
        }
        Overlays { backend, state, strip: RefCell::new(None), riven: RefCell::new(None) }
    }

    fn panel(&self, slot: &RefCell<Option<Rc<Panel>>>, namespace: &str) -> Rc<Panel> {
        // Both sit over the game while it is played: clicks go through to it.
        slot.borrow_mut().get_or_insert_with(|| Panel::new(self.backend, namespace, true)).clone()
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

    /// The game and how GDK draws over it: its monitor's scale (GDK coordinates are logical).
    fn screen(&self, ui_scale: f64) -> Screen {
        let game = self.game_rect();
        let display = gdk::Display::default();
        let scale = display
            .as_ref()
            .and_then(|d| monitor_at(d, game.x + game.width / 2, game.y + game.height / 2))
            .map_or(1, |m| m.scale_factor().max(1)) as f64;
        Screen { game, display: scale, dpi: scale, ui_scale }
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

    fn put_rewards(&self, r: &RewardResult, fresh: bool) {
        let panel = self.panel(&self.strip, "warframeproject-rewards");
        if !fresh && !panel.visible() {
            return; // a price update for panels already taken down with their screen
        }
        let settings = self.state.settings();
        let ui_scale = r.scale.or_else(|| self.state.ui_scales().first().copied()).unwrap_or(1.0);
        let screen = self.screen(ui_scale);
        let drawn = overlay::rewards(r, screen, settings.overlay_position == PanelPosition::Above);
        self.put(&panel, &drawn, screen, fresh, settings.overlay_seconds);
    }

    /// The grades beside the riven card. `fresh` is false when only the market listings arrived,
    /// which must not bring back a panel that has timed out.
    pub fn show_riven(&self, r: &RivenResult, fresh: bool) {
        let panel = self.panel(&self.riven, "warframeproject-riven");
        if !fresh && !panel.visible() {
            return;
        }
        let s = self.state.settings();
        let screen = self.screen(self.state.ui_scales().first().copied().unwrap_or(1.0));
        let drawn = overlay::riven(r, screen);
        self.put(&panel, &drawn, screen, fresh, s.riven_overlay_seconds.max(5));
    }

    fn put(&self, panel: &Rc<Panel>, drawn: &Drawn, screen: Screen, fresh: bool, secs: u32) {
        panel.set(drawn, screen.display as i32);
        if fresh || !panel.visible() {
            panel.show(self.backend, drawn.rect, secs);
        } else {
            panel.place(self.backend, drawn.rect);
            panel.window.show_all();
        }
    }
}
