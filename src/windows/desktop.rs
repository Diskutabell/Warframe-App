//! Windows: the main window (the web UI in Edge's WebView2) and the overlays over the game.
//!
//! This stands in for `desktop/` on Windows (see `main.rs`). tao makes the windows and runs the
//! event loop, wry puts the WebView2 page in the main one. The overlays show what
//! `crate::overlay` draws, in layered, always-on-top windows that never take focus from the game
//! and let every click through. Like the GTK overlays they are hidden and re-shown, never destroyed.

#[path = "tray.rs"]
mod tray;

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use tao::dpi::LogicalSize;
use tao::event::{Event as TaoEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use windows_sys::Win32::Foundation::{HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, IsWindowVisible, MB_ICONERROR, MB_OK, MessageBoxW, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow,
};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

use crate::app::App;
use crate::compositor;
use crate::config::{self, PanelPosition, Settings};
use crate::overlay::{self, Drawn, Screen};
use crate::services::scans::{RewardResult, RivenResult};
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

/// One overlay window. It is layered: Windows shows exactly the pixels handed to it, with their
/// transparency, and it never paints itself.
struct Overlay {
    window: Window,
    hide_at: Cell<Option<Instant>>,
}

impl Overlay {
    fn new(target: &EventLoopWindowTarget<Event>) -> Result<Overlay> {
        let window = WindowBuilder::new()
            .with_title("warframeproject overlay")
            .with_decorations(false)
            .with_undecorated_shadow(false)
            .with_always_on_top(true)
            .with_resizable(false)
            .with_visible(false)
            // Never take keyboard focus from the game (tao sets WS_EX_NOACTIVATE for this).
            .with_focused(false)
            .with_focusable(false)
            .with_skip_taskbar(true)
            .with_inner_size(LogicalSize::new(1.0, 1.0))
            .build(target)?;
        // The panels sit over the game (the reward screen while the cards are being picked, the
        // riven screen beside the card): every click goes through to it. This also makes the
        // window layered (WS_EX_TRANSPARENT | WS_EX_LAYERED), which `present` relies on.
        window.set_ignore_cursor_events(true)?;
        Ok(Overlay { window, hide_at: Cell::new(None) })
    }

    fn hwnd(&self) -> HWND {
        self.window.hwnd() as HWND
    }

    fn visible(&self) -> bool {
        // SAFETY: the window lives as long as `self`.
        unsafe { IsWindowVisible(self.hwnd()) != 0 }
    }

    /// Shows `drawn` for `secs` seconds.
    fn show(&self, drawn: &Drawn, secs: u32) {
        self.hide_at.set(Some(Instant::now() + Duration::from_secs(secs as u64)));
        self.present(drawn);
    }

    /// Puts `drawn` on screen at its place, above everything, without taking focus.
    fn present(&self, drawn: &Drawn) {
        let (w, h) = (drawn.pixels.width() as i32, drawn.pixels.height() as i32);
        // SAFETY: every handle is checked, used on this thread only and released below; the DIB
        // is written within its w x h x 4 bytes.
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative: rows top to bottom, as the pixels are.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if !screen.is_null() && !dc.is_null() && !bitmap.is_null() && !bits.is_null() {
                // Premultiplied BGRA, as UpdateLayeredWindow wants it.
                let out = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
                for (o, p) in out.chunks_exact_mut(4).zip(drawn.pixels.pixels()) {
                    o.copy_from_slice(&[p.blue(), p.green(), p.red(), p.alpha()]);
                }
                let old = SelectObject(dc, bitmap);
                let at = POINT { x: drawn.rect.x, y: drawn.rect.y };
                let size = SIZE { cx: w, cy: h };
                let from = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                if UpdateLayeredWindow(self.hwnd(), screen, &at, &size, dc, &from, 0, &blend, ULW_ALPHA) == 0 {
                    eprintln!("could not draw the overlay: {}", std::io::Error::last_os_error());
                }
                SelectObject(dc, old);
                SetWindowPos(
                    self.hwnd(),
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !dc.is_null() {
                DeleteDC(dc);
            }
            if !screen.is_null() {
                ReleaseDC(std::ptr::null_mut(), screen);
            }
        }
    }

    fn hide(&self) {
        self.hide_at.set(None);
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
    state: Arc<App>,
    strip: Overlay,
    riven: Overlay,
}

impl Overlays {
    /// The game's window (screen pixels) and the desktop's scale; the primary monitor when the
    /// game is not up.
    fn screen(&self, ui_scale: f64) -> Screen {
        let game = compositor::game_rect().unwrap_or_else(compositor::game_monitor);
        Screen { game, display: 1.0, dpi: compositor::game_scale(), ui_scale }
    }

    fn show_rewards(&self, r: &RewardResult, fresh: bool) {
        let s = self.state.settings();
        if !s.overlay_enabled || (r.items.is_empty() && r.error.is_none()) {
            return;
        }
        // The rest of the names or the prices: update panels that are up, never bring back ones
        // taken down with their screen.
        if !fresh && !self.strip.visible() {
            return;
        }
        let drawn = self.rewards(r);
        if fresh {
            self.strip.show(&drawn, s.overlay_seconds.max(4));
        } else {
            self.strip.present(&drawn);
        }
    }

    /// The price panels, under or above each card as the game draws the cards at the interface
    /// scale they were found at.
    fn rewards(&self, r: &RewardResult) -> Drawn {
        let ui_scale = r.scale.or_else(|| self.state.ui_scales().first().copied()).unwrap_or(1.0);
        let above = self.state.settings().overlay_position == PanelPosition::Above;
        overlay::rewards(r, self.screen(ui_scale), above)
    }

    fn show_test(&self) {
        // The test panels always show, even with the reward overlay switched off.
        let s = self.state.settings();
        let sample = RewardResult::sample(self.state.ui_scales().first().copied().unwrap_or(1.0));
        self.strip.show(&self.rewards(&sample), s.overlay_seconds.max(4));
    }

    /// The grades beside the riven card in the middle of the screen, sized with the card like the
    /// price panels. `fresh` is false when only the market listings arrived.
    fn show_riven(&self, r: &RivenResult, fresh: bool) {
        if !fresh && !self.riven.visible() {
            return;
        }
        let s = self.state.settings();
        let drawn = overlay::riven(r, self.screen(self.state.ui_scales().first().copied().unwrap_or(1.0)));
        if fresh {
            self.riven.show(&drawn, s.riven_overlay_seconds.max(5));
        } else {
            self.riven.present(&drawn);
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

    let overlays = Overlays { strip: Overlay::new(&event_loop)?, riven: Overlay::new(&event_loop)?, state };

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
                UiEvent::HideRewards => overlays.strip.hide(),
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
        overlays.expire(Instant::now());
        if !matches!(*flow, ControlFlow::ExitWithCode(_)) {
            *flow = overlays.next_deadline().map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
        }
    });
    Ok(())
}
