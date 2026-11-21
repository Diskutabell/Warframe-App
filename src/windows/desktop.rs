//! Windows: the main window (the web UI in Edge's WebView2) and the overlays over the game.
//!
//! This stands in for `desktop/` on Windows (see `main.rs`). tao makes the windows and runs the
//! event loop, wry puts the WebView2 page in the main one. The overlays show what
//! `crate::overlay` draws, in layered, always-on-top windows that never take focus from the game
//! and let every click through. Like the GTK overlays they are hidden and re-shown, never destroyed.

#[path = "overlay.rs"]
mod overlay_window;
#[path = "tray.rs"]
mod tray;

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tao::dpi::LogicalSize;
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

use crate::app::App;
use crate::compositor;
use crate::config::{self, Settings};
use crate::overlay::{self, Drawn, Screen};
use crate::services::shell::UiEvent;

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

/// Opens an HTTP(S) link in the default browser; rejects every other scheme and path.
pub fn open_url(url: &str) {
    if !crate::net::is_http_url(url) {
        return;
    }
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let op: Vec<u16> = "open\0".encode_utf16().collect();
    let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    // SAFETY: both strings are NUL-terminated UTF-16 and outlive the call; `url` has been
    // restricted to http(s) so ShellExecuteW hands it to the browser, never a local or UNC path.
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// What the event loop receives: the engine's events.
enum Event {
    Ui(UiEvent),
}

/// One overlay window and when it is to be taken down.
struct Overlay {
    window: overlay_window::LayeredWindow,
    hide_at: Cell<Option<Instant>>,
    state: Arc<App>,
    /// A drawing failed and was reported; said again only after one works.
    failing: Cell<bool>,
}

impl Overlay {
    fn new(state: Arc<App>) -> Result<Overlay> {
        Ok(Overlay {
            window: overlay_window::LayeredWindow::new()?,
            hide_at: Cell::new(None),
            state,
            failing: Cell::new(false),
        })
    }

    fn present(&self, drawn: &Drawn) {
        match self.window.present(drawn) {
            Ok(()) => self.failing.set(false),
            Err(e) => {
                if !self.failing.replace(true) {
                    self.state.accounts.event(&format!("The overlay could not be drawn: {e:#}"), "warn");
                }
                self.hide_at.set(None);
            }
        }
    }

    fn expire(&self, now: Instant) {
        if self.hide_at.get().is_some_and(|deadline| now >= deadline) {
            overlay::Presenter::hide(self);
        }
    }
}

impl overlay::Presenter for Overlay {
    fn visible(&self) -> bool {
        self.window.visible()
    }

    fn show(&self, drawn: &Drawn, secs: u32) {
        self.hide_at.set(Some(Instant::now() + Duration::from_secs(secs as u64)));
        self.present(drawn);
    }

    fn update(&self, drawn: &Drawn) {
        self.present(drawn);
    }

    fn hide(&self) {
        self.hide_at.set(None);
        self.window.hide();
    }
}

type Overlays = overlay::Overlays<Overlay>;

fn next_deadline(o: &Overlays) -> Option<Instant> {
    [o.strip.hide_at.get(), o.riven.hide_at.get()].into_iter().flatten().min()
}

/// The game's window (screen pixels) and the desktop's scale; the primary monitor when the game
/// is not up.
fn screen(ui_scale: f64) -> Screen {
    let game = compositor::game_rect().unwrap_or_else(compositor::game_monitor);
    Screen { game, display: 1.0, dpi: compositor::game_scale(), ui_scale }
}

/// Runs the event loop until the main window closes or the app is told to quit.
pub fn run(state: Arc<App>, url: &str, _backend: Backend) -> Result<()> {
    let mut event_loop = EventLoopBuilder::<Event>::with_user_event().build();

    // The engine sends its events on a channel; a small thread hands them to the event loop.
    let (tx, rx) = async_channel::unbounded::<UiEvent>();
    if !state.set_ui(tx) {
        // Told to quit before the window was made.
        return Ok(());
    }
    let forward = event_loop.create_proxy();
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
        // The page's own background (#0b0c0f), so nothing flashes white while it loads.
        .with_background_color((11, 12, 15, 255))
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

    let mut tray = tray::Tray::new(Arc::clone(&state), state.settings().minimize_to_tray, true);
    let mut visible = true;

    // Made on this thread, so tao's loop hands them their messages.
    let overlays = Overlays {
        strip: Overlay::new(Arc::clone(&state))?,
        riven: Overlay::new(Arc::clone(&state))?,
        state,
        screen: Box::new(screen),
    };

    // `run_return` hands control back to `main` when the loop ends, which then shuts the engine
    // down properly; `run` would end the process from inside the loop.
    event_loop.run_return(|event, _, flow| {
        // Owned by the loop, so they live as long as it runs.
        let _ = (&webview, &context);
        let show = |window: &Window, tray: &tray::Tray, visible: &mut bool| {
            window.set_minimized(false);
            window.set_visible(true);
            window.set_focus();
            *visible = true;
            tray.set_window_visible(true);
        };
        let hide = |window: &Window, tray: &tray::Tray, visible: &mut bool| {
            window.set_visible(false);
            *visible = false;
            tray.set_window_visible(false);
        };
        match event {
            TaoEvent::WindowEvent { event: WindowEvent::CloseRequested, window_id, .. } if window_id == window.id() => {
                if overlays.state.settings().minimize_to_tray {
                    hide(&window, &tray, &mut visible);
                } else {
                    tray.set_enabled(false, false);
                    *flow = ControlFlow::Exit;
                }
            }
            TaoEvent::UserEvent(Event::Ui(ev)) => match ev {
                UiEvent::ShowWindow => show(&window, &tray, &mut visible),
                UiEvent::HideWindow => hide(&window, &tray, &mut visible),
                UiEvent::ToggleWindow => {
                    if visible && !window.is_minimized() {
                        hide(&window, &tray, &mut visible);
                    } else {
                        show(&window, &tray, &mut visible);
                    }
                }
                UiEvent::TrayEnabled(enabled) => tray.set_enabled(enabled, visible),
                UiEvent::Rewards { result, fresh } => overlays.show_rewards(&result, fresh),
                UiEvent::HideRewards => overlays.hide_rewards(),
                UiEvent::Riven { result, fresh } => overlays.show_riven(&result, fresh),
                UiEvent::TestOverlay => overlays.show_test(),
                UiEvent::HideForCapture(ack) => {
                    overlays.hide_all();
                    let _ = ack.send(());
                }
                UiEvent::Quit => {
                    tray.set_enabled(false, false);
                    *flow = ControlFlow::Exit;
                }
            },
            _ => {}
        }
        let now = Instant::now();
        overlays.strip.expire(now);
        overlays.riven.expire(now);
        if !matches!(*flow, ControlFlow::ExitWithCode(_)) {
            *flow = next_deadline(&overlays).map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
        }
    });
    Ok(())
}
