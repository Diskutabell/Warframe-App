//! The desktop shell: the main window (the web UI in WebKitGTK) and the overlays over the game.

mod fonts;
mod overlay;
mod tray;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use webkit2gtk::{
    NavigationPolicyDecision, NavigationPolicyDecisionExt, PolicyDecisionExt, PolicyDecisionType, SettingsExt,
    URIRequestExt, WebContext, WebView, WebViewExt,
};

use crate::app::App;
use crate::compositor;
use crate::config::{OverlayBackend, Settings};
use crate::services::shell::UiEvent;

/// How the overlays stay above the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// wlr-layer-shell's overlay layer: above fullscreen windows, never focused, positioned in the
    /// compositor's own coordinates.
    LayerShell,
    /// An X11 popup (override-redirect) window through XWayland or a plain X session.
    X11,
}

/// Picks the display backend and sets the environment GTK and WebKit read at startup.
///
/// Must run before any thread exists: changing the environment is only sound while the process
/// is single-threaded.
pub fn prepare_environment(settings: &Settings) -> Backend {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    let backend = match settings.overlay_backend {
        OverlayBackend::LayerShell => Backend::LayerShell,
        OverlayBackend::X11 => Backend::X11,
        OverlayBackend::Auto if compositor::has_layer_shell() => Backend::LayerShell,
        OverlayBackend::Auto => Backend::X11,
    };
    let nvidia = std::fs::read_to_string("/proc/modules").is_ok_and(|m| m.lines().any(|l| l.starts_with("nvidia ")));
    // SAFETY: called first thing in main, before any other thread has been spawned.
    unsafe {
        if backend == Backend::X11 && set("WAYLAND_DISPLAY") && set("DISPLAY") {
            // Proton runs Warframe under XWayland; an X11 overlay can stack above it there.
            std::env::set_var("GDK_BACKEND", "x11");
        }
        if nvidia && !set("WEBKIT_DISABLE_DMABUF_RENDERER") {
            // WebKitGTK's DMA-BUF renderer draws blank windows on NVIDIA's proprietary driver.
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    backend
}

fn open_externally(uri: &str) {
    if !crate::net::is_http_url(uri) {
        eprintln!("refused to open non-http URI: {uri}");
        return;
    }
    if let Err(e) = gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>) {
        eprintln!("could not open {uri}: {e}");
    }
}

fn main_window(url: &str) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("warframeproject");
    window.set_default_size(1320, 860);
    window.set_size_request(900, 600);
    window.set_icon_name(Some("applications-games"));

    // Ephemeral: all state lives in the engine, so the page needs no persistent storage.
    let context = WebContext::new_ephemeral();
    let webview = WebView::with_context(&context);
    // The page's own background (#0b0c0f), so nothing flashes white while it loads.
    webview.set_background_color(&gdk::RGBA::new(11.0 / 255.0, 12.0 / 255.0, 15.0 / 255.0, 1.0));
    if let Some(settings) = WebViewExt::settings(&webview) {
        // WARFRAMEPROJECT_DEBUG: the web inspector, and the page's console messages on stdout.
        let debug = std::env::var_os("WARFRAMEPROJECT_DEBUG").is_some();
        settings.set_enable_developer_extras(debug);
        settings.set_enable_write_console_messages_to_stdout(debug);
    }
    // Links the UI opens in a new tab (warframe.market pages) belong in the user's browser.
    webview.connect_decide_policy(|_, decision, kind| {
        if kind != PolicyDecisionType::NewWindowAction {
            return false;
        }
        if let Some(nav) = decision.downcast_ref::<NavigationPolicyDecision>() {
            let uri = nav.navigation_action().and_then(|a| a.request()).and_then(|r| r.uri());
            if let Some(uri) = uri {
                open_externally(&uri);
            }
        }
        decision.ignore();
        true
    });
    webview.load_uri(url);
    window.add(&webview);
    window
}

fn show_main_window(window: &gtk::Window, tray: &RefCell<tray::Tray>) {
    if !window.is_visible() {
        // Re-apply the float rule in case Hyprland/Sway reloaded while the window was in the tray.
        let _ = compositor::float_main_window();
        window.show_all();
    }
    window.deiconify();
    window.present();
    tray.borrow().set_window_visible(true);
}

fn hide_main_window(window: &gtk::Window, tray: &RefCell<tray::Tray>) {
    window.hide();
    tray.borrow().set_window_visible(false);
}

/// Runs the GTK main loop until the main window closes.
pub fn run(state: Arc<App>, url: &str, backend: Backend) -> Result<()> {
    glib::set_prgname(Some("warframeproject"));
    glib::set_application_name("warframeproject");
    // Before GTK starts: Pango builds its font list once, and fonts added later are not in it.
    fonts::register();
    gtk::init().map_err(|e| anyhow!("GTK could not start: {e}"))?;

    let backend = if backend == Backend::LayerShell && !gtk_layer_shell::is_supported() {
        eprintln!("this compositor has no layer shell; the overlays use ordinary windows");
        Backend::X11
    } else {
        backend
    };
    let window = main_window(url);
    let tray = Rc::new(RefCell::new(tray::Tray::new(Arc::clone(&state), state.settings().minimize_to_tray, true)));
    {
        let close_state = Arc::clone(&state);
        let close_tray = Rc::clone(&tray);
        window.connect_delete_event(move |win, _| {
            if close_state.settings().minimize_to_tray {
                hide_main_window(win, &close_tray);
                glib::Propagation::Stop
            } else {
                close_tray.borrow_mut().set_enabled(false, false);
                gtk::main_quit();
                glib::Propagation::Proceed
            }
        });
    }
    // Before the window opens: a tiling compositor would otherwise fit it into its layout.
    if let Err(e) = compositor::float_main_window() {
        eprintln!("the window may open tiled: {e:#}");
    }
    window.show_all();

    let overlays = Rc::new(overlay::Overlays::new(backend, Arc::clone(&state)));
    let (tx, rx) = async_channel::unbounded::<UiEvent>();
    if !state.set_ui(tx) {
        // Told to quit while the window was being made.
        tray.borrow_mut().set_enabled(false, false);
        return Ok(());
    }
    let event_tray = Rc::clone(&tray);
    glib::MainContext::default().spawn_local(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                UiEvent::ShowWindow => show_main_window(&window, &event_tray),
                UiEvent::HideWindow => hide_main_window(&window, &event_tray),
                UiEvent::ToggleWindow => {
                    if window.is_visible() {
                        hide_main_window(&window, &event_tray);
                    } else {
                        show_main_window(&window, &event_tray);
                    }
                }
                UiEvent::TrayEnabled(enabled) => {
                    event_tray.borrow_mut().set_enabled(enabled, window.is_visible());
                }
                UiEvent::Rewards { result, fresh } => overlays.show_rewards(&result, fresh),
                UiEvent::HideRewards => overlays.hide_rewards(),
                UiEvent::Riven { result, fresh } => overlays.show_riven(&result, fresh),
                UiEvent::TestOverlay => overlays.show_test(),
                UiEvent::HideForCapture(ack) => {
                    overlays.hide_all();
                    let _ = ack.send(());
                }
                UiEvent::Quit => {
                    event_tray.borrow_mut().set_enabled(false, false);
                    gtk::main_quit();
                }
            }
        }
    });
    gtk::main();
    tray.borrow_mut().set_enabled(false, false);
    Ok(())
}
