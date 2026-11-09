//! Windows: the main window (the web UI in Edge's WebView2) and the overlays over the game.
//!
//! This stands in for `desktop/` on Windows (see `main.rs`). tao makes the windows and runs the
//! event loop, wry puts a WebView2 page in each. The overlays are web pages too
//! (`web/overlay.html`) in transparent, always-on-top windows that never take focus from the game.
//! Like the GTK overlays they are hidden and re-shown, never destroyed.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tao::dpi::LogicalSize;
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, IsWindowVisible, MB_ICONERROR, MB_OK, MessageBoxW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow,
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder};

use crate::compositor::{self, Rect};
use crate::config::{self, PanelPosition, Settings};
use crate::ocr::{reward, riven};
use crate::state::{RewardResult, RivenResult, State, UiEvent};

/// Linux picks an overlay backend here; Windows has only one way.
pub struct Backend;

pub fn prepare_environment(_settings: &Settings) -> Backend {
    Backend
}

/// First thing in `main`: print to the terminal the app was started from, if any, and work in
/// real pixels on scaled screens.
pub fn process_setup() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    // SAFETY: these only change settings of this process, before any window exists.
    unsafe {
        // The exe is a GUI program (no console window when started from the desktop), so
        // `warframeproject trigger` in a terminal would print nothing without this. Output
        // that is already redirected (to a file or pipe) is left where it goes.
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if out.is_null() || out == INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
        // Without it Windows scales our windows itself and reports scaled coordinates, and the
        // overlays would land in the wrong place over the game.
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// An error shown in a message box: started from the desktop, there is no terminal to read.
pub fn error_box(text: &str) {
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("warframeproject"));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

/// Opens a link in the default browser.
pub fn open_url(url: &str) {
    // url.dll's handler is what Windows itself uses for links; unlike `cmd /c start` it does not
    // treat '&' in the URL as a command separator.
    let _ = std::process::Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(url).spawn();
}

/// What the event loop receives: the engine's events, and the overlay pages asking to be hidden.
enum Event {
    Ui(UiEvent),
    Hide(Which),
    Ready(Which),
}

#[derive(Clone, Copy)]
enum Which {
    Strip,
    Riven,
}

/// The size an overlay window starts at, before it is first placed.
const START_W: f64 = 1000.0;
const START_H: f64 = 168.0;

fn fit_rect(game: Rect, width: i32, height: i32, x: i32, y: i32) -> Rect {
    let width = width.clamp(1, game.width.max(1));
    let height = height.clamp(1, game.height.max(1));
    Rect {
        x: x.clamp(game.x, game.x + game.width.max(1) - width),
        y: y.clamp(game.y, game.y + game.height.max(1) - height),
        width,
        height,
    }
}

/// One overlay window with its page.
struct Overlay {
    window: Window,
    webview: WebView,
    ready: Cell<bool>,
    pending: RefCell<Vec<String>>,
    hide_at: Cell<Option<Instant>>,
}

impl Overlay {
    fn new(
        target: &EventLoopWindowTarget<Event>,
        context: &mut WebContext,
        url: &str,
        proxy: EventLoopProxy<Event>,
        which: Which,
    ) -> Result<Overlay> {
        let window = WindowBuilder::new()
            .with_title("warframeproject overlay")
            .with_decorations(false)
            .with_undecorated_shadow(false)
            .with_transparent(true)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            // Never take keyboard focus from the game (tao sets WS_EX_NOACTIVATE for this).
            .with_focused(false)
            .with_focusable(false)
            .with_skip_taskbar(true)
            .with_inner_size(LogicalSize::new(START_W, START_H))
            .build(target)?;
        // The panels sit over the game (the reward screen while the cards are being picked, the
        // riven screen beside the card): every click goes through to it.
        window.set_ignore_cursor_events(true)?;
        let webview = WebViewBuilder::new_with_web_context(context)
            .with_url(format!("{url}overlay.html"))
            .with_transparent(true)
            .with_ipc_handler(move |req| {
                let event = match req.body().as_str() {
                    "hide" => Some(Event::Hide(which)),
                    "ready" => Some(Event::Ready(which)),
                    _ => None,
                };
                if let Some(event) = event {
                    let _ = proxy.send_event(event);
                }
            })
            .build(&window)?;
        Ok(Overlay {
            window,
            webview,
            ready: Cell::new(false),
            pending: RefCell::new(Vec::new()),
            hide_at: Cell::new(None),
        })
    }

    fn hwnd(&self) -> HWND {
        self.window.hwnd() as HWND
    }

    fn visible(&self) -> bool {
        // SAFETY: the window lives as long as `self`.
        unsafe { IsWindowVisible(self.hwnd()) != 0 }
    }

    /// Shows the page with `result` at `rect` (screen pixels) for `secs` seconds. `unit` is the
    /// size the page draws at (CSS pixels per unit of the reward screen) and `boxes` where its
    /// panels go (CSS pixels inside the window: x, y, width, height).
    fn show(&self, rect: Rect, kind: &str, result: &impl serde::Serialize, secs: u32, unit: f64, boxes: &[[f64; 4]]) {
        self.hide_at.set(Some(Instant::now() + Duration::from_secs(secs as u64)));
        self.render(kind, result, unit, boxes);
        self.place(rect);
    }

    /// Moves the window to `rect` (screen pixels) and shows it.
    fn place(&self, rect: Rect) {
        // SAFETY: the window lives as long as `self`. SWP_NOACTIVATE: shown without focus.
        unsafe {
            SetWindowPos(
                self.hwnd(),
                HWND_TOPMOST,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    fn update(&self, kind: &str, result: &impl serde::Serialize, unit: f64, boxes: &[[f64; 4]]) {
        self.render(kind, result, unit, boxes);
    }

    fn render(&self, kind: &str, result: &impl serde::Serialize, unit: f64, boxes: &[[f64; 4]]) {
        let (json, boxes) = match (serde_json::to_string(result), serde_json::to_string(boxes)) {
            (Ok(json), Ok(boxes)) => (json, boxes),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("could not serialize overlay: {e}");
                return;
            }
        };
        let script = format!("render({kind:?}, {json}, {unit}, {boxes})");
        if self.ready.get() {
            self.evaluate(&script);
        } else {
            self.pending.borrow_mut().push(script);
        }
    }

    fn evaluate(&self, script: &str) {
        if let Err(e) = self.webview.evaluate_script(script) {
            eprintln!("could not update overlay: {e}");
        }
    }

    fn ready(&self) {
        self.ready.set(true);
        for script in self.pending.borrow_mut().drain(..) {
            self.evaluate(&script);
        }
    }

    fn hide(&self) {
        self.hide_at.set(None);
        self.pending.borrow_mut().clear();
        // SAFETY: the window lives as long as `self`.
        unsafe {
            ShowWindow(self.hwnd(), SW_HIDE);
        }
    }

    fn expire(&self, now: Instant) {
        if self.hide_at.get().is_some_and(|deadline| now >= deadline) {
            self.hide();
        }
    }
}

struct Overlays {
    state: Arc<State>,
    strip: Overlay,
    riven: Overlay,
}

impl Overlays {
    /// The game's window and its display scale; the primary monitor when the game is not up.
    fn game(&self) -> (Rect, f64) {
        (compositor::game_rect().unwrap_or_else(compositor::game_monitor), compositor::game_scale())
    }

    /// Where the price panels go, under or above each card as the game draws the cards at the
    /// interface scale they were found at: the window around them (screen pixels), each panel in
    /// it (CSS pixels: x, y, width, height), and the page's size in CSS pixels per unit.
    fn reward_layout(&self, r: &RewardResult) -> (Rect, Vec<[f64; 4]>, f64) {
        let (g, dpi) = self.game();
        let scale = r.scale.or_else(|| self.state.ui_scales().first().copied()).unwrap_or(1.0);
        // A message has no cards to sit under: one wide panel in the middle (`panel_places(0)`).
        let cards = if r.items.is_empty() { 0 } else { r.cards.unwrap_or(r.items.len()).max(1) };
        let u = reward::unit(g.width as f64, g.height as f64, scale);
        // Text smaller than this is unreadable; the panels then grow away from the cards.
        let text_u = (u / dpi).max(0.75);
        let above = self.state.settings().overlay_position == PanelPosition::Above;
        let game = (g.x as f64, g.y as f64, g.width as f64, g.height as f64);
        let rects: Vec<[f64; 4]> = reward::panel_places(cards, above)
            .iter()
            .map(|p| reward::panel_rect(p, game, u, reward::PANEL_H * text_u * dpi))
            .collect();
        let x0 = rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
        let y0 = rects.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
        let x1 = rects.iter().map(|r| r[0] + r[2]).fold(f64::NEG_INFINITY, f64::max);
        let y1 = rects.iter().map(|r| r[1] + r[3]).fold(f64::NEG_INFINITY, f64::max);
        let window =
            fit_rect(g, (x1 - x0).round() as i32, (y1 - y0).round() as i32, x0.round() as i32, y0.round() as i32);
        // Kept inside the game, the window may have moved; the panels move with it.
        let boxes = rects
            .iter()
            .map(|r| [(r[0] - window.x as f64) / dpi, (r[1] - window.y as f64) / dpi, r[2] / dpi, r[3] / dpi])
            .collect();
        (window, boxes, text_u)
    }

    fn show_rewards(&self, r: &RewardResult, fresh: bool) {
        let s = self.state.settings();
        if !s.overlay_enabled || (r.items.is_empty() && r.error.is_none()) {
            return;
        }
        let (rect, boxes, unit) = self.reward_layout(r);
        if !fresh {
            // The rest of the names or the prices: update panels that are up, never bring back
            // ones taken down with their screen.
            if self.strip.visible() {
                self.strip.update("rewards", r, unit, &boxes);
                self.strip.place(rect);
            }
            return;
        }
        self.strip.show(rect, "rewards", r, s.overlay_seconds.max(4), unit, &boxes);
    }

    fn show_test(&self) {
        // The test panels always show, even with the reward overlay switched off.
        let s = self.state.settings();
        let sample = RewardResult::sample(self.state.ui_scales().first().copied().unwrap_or(1.0));
        let (rect, boxes, unit) = self.reward_layout(&sample);
        self.strip.show(rect, "rewards", &sample, s.overlay_seconds.max(4), unit, &boxes);
    }

    /// The grades beside the riven card in the middle of the screen, sized with the card like the
    /// price panels. `fresh` is false when only the market listings arrived.
    fn show_riven(&self, r: &RivenResult, fresh: bool) {
        if !fresh && !self.riven.visible() {
            return;
        }
        let s = self.state.settings();
        let (g, dpi) = self.game();
        let scale = self.state.ui_scales().first().copied().unwrap_or(1.0);
        let u = reward::unit(g.width as f64, g.height as f64, scale);
        let text_u = (u / dpi).max(0.75);
        let rows = if r.error.is_some() { 1 } else { r.stats.len() };
        let w = (riven::PANEL_W * text_u * dpi).round() as i32;
        let h = (riven::panel_height(rows) * text_u * dpi).round() as i32;
        let x = (g.x as f64 + g.width as f64 / 2.0 + (riven::CARD_HALF_W + riven::PANEL_GAP) * u).round() as i32;
        let y = (g.y as f64 + g.height as f64 / 2.0 - riven::PANEL_TOP * u).round() as i32;
        let rect = fit_rect(g, w, h, x, y);
        if fresh {
            self.riven.show(rect, "riven", r, s.riven_overlay_seconds.max(5), text_u, &[]);
        } else {
            self.riven.update("riven", r, text_u, &[]);
        }
    }

    fn hide_all(&self) {
        self.strip.hide();
        self.riven.hide();
    }

    fn expire(&self, now: Instant) {
        self.strip.expire(now);
        self.riven.expire(now);
    }

    fn next_deadline(&self) -> Option<Instant> {
        [self.strip.hide_at.get(), self.riven.hide_at.get()].into_iter().flatten().min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlays_stay_inside_the_game_on_small_and_offset_screens() {
        let game = Rect { x: -1600, y: 200, width: 400, height: 300 };
        assert_eq!(fit_rect(game, 1000, 168, -2000, 500), Rect { x: -1600, y: 332, width: 400, height: 168 });
        assert_eq!(fit_rect(game, 470, 600, 0, -100), Rect { x: -1600, y: 200, width: 400, height: 300 });
    }
}

/// Runs the event loop until the main window closes; tao ends the process then.
pub fn run(state: Arc<State>, url: &str, _backend: Backend) -> Result<()> {
    let event_loop = EventLoopBuilder::<Event>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // The engine sends its events on a channel; a small thread hands them to the event loop.
    let (tx, rx) = async_channel::unbounded::<UiEvent>();
    state.set_ui(tx);
    let forward = proxy.clone();
    std::thread::Builder::new().name("ui-events".into()).spawn(move || {
        while let Ok(ev) = rx.recv_blocking() {
            if forward.send_event(Event::Ui(ev)).is_err() {
                break;
            }
        }
    })?;

    let window = WindowBuilder::new()
        .with_title("warframeproject")
        .with_inner_size(LogicalSize::new(1320.0, 860.0))
        .with_min_inner_size(LogicalSize::new(900.0, 600.0))
        .build(&event_loop)?;
    // WebView2 keeps a profile folder, by default next to the exe, which may not be writable.
    let mut context = WebContext::new(Some(config::dirs().cache.join("webview2")));
    let debug = std::env::var_os("WARFRAMEPROJECT_DEBUG").is_some();
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        // The page's own background, so nothing flashes white while it loads.
        .with_background_color((11, 18, 24, 255))
        .with_devtools(debug)
        // Links the UI opens in a new tab (warframe.market pages) belong in the browser.
        .with_new_window_req_handler(|uri, _| {
            open_url(&uri);
            NewWindowResponse::Deny
        })
        .build(&window)
        .map_err(|e| {
            anyhow::anyhow!("The window could not open: {e}. The app needs the Microsoft Edge WebView2 Runtime.")
        })?;

    let overlays = Overlays {
        strip: Overlay::new(&event_loop, &mut context, url, proxy.clone(), Which::Strip)?,
        riven: Overlay::new(&event_loop, &mut context, url, proxy, Which::Riven)?,
        state,
    };

    event_loop.run(move |event, _, flow| {
        // Owned by the loop, so they live as long as it runs.
        let _ = (&webview, &context);
        match event {
            TaoEvent::WindowEvent { event: WindowEvent::CloseRequested, window_id, .. } if window_id == window.id() => {
                *flow = ControlFlow::Exit;
            }
            TaoEvent::UserEvent(Event::Ui(ev)) => match ev {
                UiEvent::ShowWindow => {
                    window.set_minimized(false);
                    window.set_visible(true);
                    window.set_focus();
                }
                UiEvent::Rewards { result, fresh } => overlays.show_rewards(&result, fresh),
                UiEvent::HideRewards => overlays.strip.hide(),
                UiEvent::Riven { result, fresh } => overlays.show_riven(&result, fresh),
                UiEvent::TestOverlay => overlays.show_test(),
                UiEvent::HideForCapture(ack) => {
                    overlays.hide_all();
                    let _ = ack.send(());
                }
                UiEvent::Quit => *flow = ControlFlow::Exit,
            },
            TaoEvent::UserEvent(Event::Hide(Which::Strip)) => overlays.strip.hide(),
            TaoEvent::UserEvent(Event::Hide(Which::Riven)) => overlays.riven.hide(),
            TaoEvent::UserEvent(Event::Ready(Which::Strip)) => overlays.strip.ready(),
            TaoEvent::UserEvent(Event::Ready(Which::Riven)) => overlays.riven.ready(),
            _ => {}
        }
        overlays.expire(Instant::now());
        if !matches!(*flow, ControlFlow::ExitWithCode(_)) {
            *flow = overlays.next_deadline().map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
        }
    })
}
