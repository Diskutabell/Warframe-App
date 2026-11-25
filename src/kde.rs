//! KDE Plasma over D-Bus: KWin says where the game's window is, and KDE's global shortcut
//! service (kglobalaccel, part of KWin since Plasma 6) holds the hotkeys.
//!
//! The hotkeys work like an ordinary KDE application's: the app registers its actions with
//! kglobalaccel, which grabs the keys and sends `globalShortcutPressed` to the app while it runs.
//! kglobalaccel keeps them in `~/.config/kglobalshortcutsrc`, so they show up under System
//! Settings > Keyboard > Shortcuts, and a later start finds them there and activates them again.
//! Nothing else of the user's configuration is touched.
//!
//! Method names, signatures and flag values are the ones the installed kglobalacceld 6.8 and
//! KWin 6.8 export (checked by introspection and against `kglobalacceld.h`).

use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use gtk::gio;
use gtk::glib::{self, ToVariant, Variant};

use crate::UserError;
use crate::compositor::Rect;

const ACCEL: &str = "org.kde.kglobalaccel";
const ACCEL_PATH: &str = "/kglobalaccel";
const ACCEL_IFACE: &str = "org.kde.KGlobalAccel";
const COMPONENT_IFACE: &str = "org.kde.kglobalaccel.Component";
const KWIN: &str = "org.kde.KWin";

/// The component's unique name: the name of the app's desktop file, so System Settings shows the
/// shortcuts under the app's own name and icon.
const COMPONENT: &str = "warframeproject";

/// `KGlobalAccelD::SetShortcutFlag`: the action is live (its keys trigger it), and the keys given
/// replace any kept from an earlier run instead of being replaced by them.
const SET_PRESENT: u32 = 2;
const NO_AUTOLOADING: u32 = 4;

/// KWin answers in milliseconds; this only guards against a compositor that is stuck.
const TIMEOUT_MS: i32 = 2000;

/// A Plasma session (Wayland or X11), from what the session tells its programs.
pub fn is_session() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|part| part.eq_ignore_ascii_case("kde")))
}

fn session_bus() -> Result<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|e| anyhow!("session D-Bus unavailable: {e}"))
}

fn call(
    conn: &gio::DBusConnection,
    dest: &str,
    path: &str,
    iface: &str,
    method: &str,
    args: Variant,
) -> Result<Variant> {
    conn.call_sync(
        Some(dest),
        path,
        iface,
        method,
        Some(&args),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        TIMEOUT_MS,
        gio::Cancellable::NONE,
    )
    .map_err(|e| anyhow!("{method}: {}", e.message()))
}

// ------------------------------------------------------------------ the game's window

/// Warframe's window in KWin's (layout) coordinates.
///
/// KWin has no call that lists windows, but its window search for KRunner (`/WindowsRunner`)
/// answers with the ids of the windows whose title matches, and `getWindowInfo` describes one by
/// id: its class, title, geometry and state. Both only read. The search finds browser tabs and
/// chats that name the game too, so each hit is checked with `is_game`.
pub fn game_window(is_game: impl Fn(&str, &str) -> bool) -> Option<Rect> {
    let conn = session_bus().ok()?;
    let hits = call(&conn, KWIN, "/WindowsRunner", "org.kde.krunner1", "Match", ("warframe",).to_variant()).ok()?;
    // a(sssida{sv}): match id "0_{uuid}", text, icon, type, relevance, properties.
    hits.child_value(0).iter().find_map(|hit| {
        let id = hit.try_child_value(0)?.str()?.to_owned();
        let (_, uuid) = id.split_once('_')?;
        let info = call(&conn, KWIN, "/KWin", KWIN, "getWindowInfo", (uuid,).to_variant()).ok()?.child_value(0);
        window_rect(&info, &is_game)
    })
}

/// The geometry from a `getWindowInfo` answer (`a{sv}`), when it describes a shown game window.
fn window_rect(info: &Variant, is_game: &impl Fn(&str, &str) -> bool) -> Option<Rect> {
    let info = glib::VariantDict::new(Some(info));
    let text = |key: &str| info.lookup_value(key, None).and_then(|v| v.str().map(str::to_owned)).unwrap_or_default();
    let number = |key: &str| info.lookup_value(key, None).and_then(|v| v.get::<f64>());
    if !is_game(&text("resourceClass"), &text("caption")) {
        return None;
    }
    if info.lookup_value("minimized", None).and_then(|v| v.get::<bool>()) == Some(true) {
        return None;
    }
    let rect = Rect {
        x: number("x")?.round() as i32,
        y: number("y")?.round() as i32,
        width: number("width")?.round() as i32,
        height: number("height")?.round() as i32,
    };
    (rect.width > 0 && rect.height > 0).then_some(rect)
}

// ------------------------------------------------------------------ hotkeys

/// A key as Qt names it (`Qt::Key` | `Qt::KeyboardModifier`), which is what kglobalaccel takes.
///
/// The key names are the ones the other platforms take (validated by `compositor::validate_key`):
/// F1-F35, letters, digits, and a few named keys, with any of Super, Ctrl, Alt and Shift.
pub fn qt_key(raw: &str) -> Option<i32> {
    const SHIFT: i32 = 0x0200_0000;
    const CONTROL: i32 = 0x0400_0000;
    const ALT: i32 = 0x0800_0000;
    const META: i32 = 0x1000_0000;
    const KEYPAD: i32 = 0x2000_0000;
    let mut mods = 0;
    let mut key = None;
    for part in raw.split(['+', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        let up = part.to_ascii_uppercase();
        let modifier = match up.as_str() {
            "SUPER" | "WIN" | "WINDOWS" | "META" | "MOD4" => Some(META),
            "CTRL" | "CONTROL" => Some(CONTROL),
            "ALT" | "MOD1" => Some(ALT),
            "SHIFT" => Some(SHIFT),
            _ => None,
        };
        if let Some(m) = modifier {
            mods |= m;
            continue;
        }
        if key.is_some() {
            return None;
        }
        let function = up.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()).filter(|n| (1..=35).contains(n));
        let keypad = up.strip_prefix("KP_").filter(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit());
        key = Some(match up.as_str() {
            _ if function.is_some() => 0x0100_0030 + function? - 1,
            _ if keypad.is_some() => KEYPAD | keypad?.as_bytes()[0] as i32,
            c if c.len() == 1 && c.as_bytes()[0].is_ascii_alphanumeric() => c.as_bytes()[0] as i32,
            "ESCAPE" | "ESC" => 0x0100_0000,
            "TAB" => 0x0100_0001,
            "BACKSPACE" => 0x0100_0003,
            "RETURN" | "ENTER" => 0x0100_0004,
            "INSERT" => 0x0100_0006,
            "DELETE" => 0x0100_0007,
            "PAUSE" => 0x0100_0008,
            "PRINT" => 0x0100_0009,
            "HOME" => 0x0100_0010,
            "END" => 0x0100_0011,
            "LEFT" => 0x0100_0012,
            "UP" => 0x0100_0013,
            "RIGHT" => 0x0100_0014,
            "DOWN" => 0x0100_0015,
            "PAGEUP" | "PAGE_UP" | "PRIOR" => 0x0100_0016,
            "PAGEDOWN" | "PAGE_DOWN" | "NEXT" => 0x0100_0017,
            "SCROLLLOCK" | "SCROLL_LOCK" | "SCROLL" => 0x0100_0026,
            "MENU" => 0x0100_0055,
            "SPACE" => 0x20,
            "MINUS" => 0x2d,
            "EQUAL" => 0x3d,
            "GRAVE" => 0x60,
            _ => return None,
        });
    }
    key.map(|k| k | mods)
}

/// kglobalaccel's id of an action: component, action, and their names for people. A list of
/// strings, not a `Variant`: a `Variant` inside a tuple would be sent boxed (`v`), not as `as`.
fn action_id(action: &str, text: &str) -> Vec<String> {
    [COMPONENT, action, COMPONENT, text].map(str::to_owned).to_vec()
}

/// One key as a `QKeySequence` on the bus: a struct of the up to four keys of a sequence.
fn sequence(key: i32) -> (Vec<i32>,) {
    (vec![key, 0, 0, 0],)
}

/// The action of an API path: `rewards/scan` -> `rewards-scan`.
fn action_name(path: &str) -> String {
    path.replace('/', "-")
}

/// One hotkey to register: the key (empty to remove it), the API path it calls, what it does.
pub struct Shortcut<'a> {
    pub key: &'a str,
    pub path: &'a str,
    pub text: &'a str,
}

/// The actions this run holds live, to let go of when the app quits.
static LIVE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Whether the shortcuts were registered once (in this or an earlier run): kglobalaccel still
/// knows the component. A start only activates them again then, as Hyprland and Sway keep an
/// installed bind until it is removed.
pub fn installed() -> bool {
    let Ok(conn) = session_bus() else { return false };
    let Ok(all) = call(&conn, ACCEL, ACCEL_PATH, ACCEL_IFACE, "allMainComponents", ().to_variant()) else {
        return false;
    };
    // aas: per component [unique name, friendly name, ...].
    all.child_value(0).iter().any(|c| c.try_child_value(0).is_some_and(|name| name.str() == Some(COMPONENT)))
}

/// Registers the hotkeys with KDE's global shortcuts, replacing the ones registered before, and
/// has a press call the app's API on `port` (the one this run serves).
///
/// Every key is checked before anything changes. A key some other shortcut already has is not
/// taken from it - kglobalaccel itself would hand it out twice (seen with KWin's Meta+W): the
/// others are registered, and the error names what has it, as System Settings would.
pub fn register(port: u16, shortcuts: &[Shortcut]) -> Result<()> {
    let mut wanted = Vec::new();
    for s in shortcuts {
        let key = s.key.trim();
        let code =
            if key.is_empty() {
                None
            } else {
                Some(qt_key(key).ok_or_else(|| {
                    UserError::new(format!("\"{key}\" is not a key KDE can bind (try F9 or ctrl+F9)."))
                })?)
            };
        wanted.push((action_name(s.path), code, s));
    }
    if wanted.iter().all(|(_, code, _)| code.is_none()) {
        return Err(UserError::new("Set at least one key first.").into());
    }
    let conn = session_bus()?;
    let accel = |method: &str, args: Variant| call(&conn, ACCEL, ACCEL_PATH, ACCEL_IFACE, method, args);

    let mut taken = Vec::new();
    for (_, code, s) in &mut wanted {
        let Some(key) = *code else { continue };
        if let Some(holder) = holder(&accel, key) {
            taken.push(format!("{} is already the KDE shortcut for {holder}", s.key.trim()));
            *code = None;
        }
    }
    // Our own keys go first, so that two actions can swap keys without colliding with each other.
    for (action, code, s) in &wanted {
        if code.is_some() {
            accel("doRegister", (action_id(action, s.text),).to_variant())?;
            accel(
                "setShortcutKeys",
                (action_id(action, s.text), Vec::<(Vec<i32>,)>::new(), SET_PRESENT | NO_AUTOLOADING).to_variant(),
            )?;
        } else {
            // A cleared or taken key: the action leaves KDE's list altogether.
            accel("unregister", (COMPONENT, action.as_str()).to_variant())?;
        }
    }
    let mut live = Vec::new();
    for (action, code, s) in &wanted {
        let Some(code) = *code else { continue };
        let set = accel(
            "setShortcutKeys",
            (action_id(action, s.text), vec![sequence(code)], SET_PRESENT | NO_AUTOLOADING).to_variant(),
        )?;
        // a(ai): the keys it now has; kglobalaccel leaves out one that a live shortcut uses.
        let got = set
            .child_value(0)
            .iter()
            .any(|seq| seq.child_value(0).try_child_value(0).and_then(|k| k.get::<i32>()) == Some(code));
        if got {
            live.push(action.clone());
        } else {
            taken.push(format!("{} is already a KDE shortcut", s.key.trim()));
        }
    }
    *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = live;
    listen(&conn, port)?;
    if !taken.is_empty() {
        return Err(UserError::new(format!(
            "{}. Pick another key, or free it in System Settings > Keyboard > Shortcuts.",
            taken.join("; ")
        ))
        .into());
    }
    Ok(())
}

/// Which other program's shortcut has `key`, as "KWin: Toggle Overview", from every shortcut
/// kglobalaccel knows (also those of programs not running now). None also when kglobalaccel is
/// too old to be asked (before KDE Frameworks 5.90): the keys it hands back still tell.
fn holder(accel: &impl Fn(&str, Variant) -> Result<Variant>, key: i32) -> Option<String> {
    // KGlobalAccel::MatchType::Equal is 0, sent as a struct.
    let found = accel("globalShortcutsByKey", (sequence(key), (0i32,)).to_variant()).ok()?;
    // a(ssssssaiai): action, its name, component, its name, context, its name, keys, defaults.
    found.child_value(0).iter().find_map(|info| {
        let field = |i: usize| info.try_child_value(i).and_then(|v| v.str().map(str::to_owned));
        (field(2)? != COMPONENT).then(|| format!("{}: {}", field(3).unwrap_or_default(), field(1).unwrap_or_default()))
    })
}

/// Lets go of the keys when the app quits: kglobalaccel keeps grabbing a live action's keys until
/// told otherwise, also after the program that registered it is gone. They stay in KDE's list and
/// come back on the next start.
pub fn release() {
    let actions = std::mem::take(&mut *LIVE.lock().unwrap_or_else(|e| e.into_inner()));
    if actions.is_empty() {
        return;
    }
    let Ok(conn) = session_bus() else { return };
    for action in actions {
        let _ = call(&conn, ACCEL, ACCEL_PATH, ACCEL_IFACE, "setInactive", (action_id(&action, ""),).to_variant());
    }
}

/// Starts (once) the thread that answers presses: kglobalaccel sends `globalShortcutPressed` on
/// the component's object, and a press calls the API path of its action, as the hotkeys on the
/// other desktops do.
fn listen(conn: &gio::DBusConnection, port: u16) -> Result<()> {
    // Held until the thread listens: the start's registration and an Install pressed at the same
    // moment must not start two listeners (each press would then scan twice).
    static STARTED: Mutex<bool> = Mutex::new(false);
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    if *started {
        return Ok(());
    }
    let path = call(conn, ACCEL, ACCEL_PATH, ACCEL_IFACE, "getComponent", (COMPONENT,).to_variant())?
        .child_value(0)
        .str()
        .map(str::to_owned)
        .context("kglobalaccel named no object for the shortcuts")?;
    let conn = conn.clone();
    let (ready, wait) = std::sync::mpsc::channel();
    std::thread::Builder::new().name("kde-hotkeys".into()).spawn(move || {
        // Signals are delivered on the main context of the thread that subscribed: this thread
        // has its own, so presses arrive whether or not the GTK window runs.
        let context = glib::MainContext::new();
        let run = context.with_thread_default(|| {
            conn.signal_subscribe(
                Some(ACCEL),
                Some(COMPONENT_IFACE),
                Some("globalShortcutPressed"),
                Some(&path),
                None,
                gio::DBusSignalFlags::NONE,
                move |_, _, _, _, _, params| {
                    let Some((component, action, _)) = params.get::<(String, String, i64)>() else { return };
                    if component != COMPONENT {
                        return;
                    }
                    let path = action.replacen('-', "/", 1);
                    // Off this thread: a scan takes a second, and other presses must still arrive.
                    std::thread::spawn(move || {
                        if let Err(e) = crate::net::local::call(port, &path, true, Duration::from_secs(60)) {
                            eprintln!("hotkey request failed: {e:#}");
                        }
                    });
                },
            );
            let _ = ready.send(());
            glib::MainLoop::new(Some(&context), false).run();
        });
        if run.is_err() {
            eprintln!("the KDE hotkeys could not start listening");
        }
    })?;
    wait.recv_timeout(Duration::from_secs(5)).context("the KDE hotkey listener did not start")?;
    *started = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_as_qt_names_them() {
        assert_eq!(qt_key("F9"), Some(0x0100_0038));
        assert_eq!(qt_key("f1"), Some(0x0100_0030));
        assert_eq!(qt_key("F35"), Some(0x0100_0052));
        assert_eq!(qt_key("SUPER + F9"), Some(0x1000_0000 | 0x0100_0038));
        assert_eq!(qt_key("ctrl + shift + F10"), Some(0x0400_0000 | 0x0200_0000 | 0x0100_0039));
        assert_eq!(qt_key("alt+a"), Some(0x0800_0000 | 0x41));
        assert_eq!(qt_key("Page_Down"), Some(0x0100_0017));
        assert_eq!(qt_key("KP_5"), Some(0x2000_0000 | 0x35));
        assert_eq!(qt_key("F36"), None);
        assert_eq!(qt_key("F9 + F10"), None, "two keys");
        assert_eq!(qt_key("SUPER"), None, "no key");
        assert_eq!(qt_key("Hyper_L"), None, "a name not handled");
    }

    #[test]
    fn actions_are_api_paths() {
        assert_eq!(action_name("rewards/scan"), "rewards-scan");
        assert_eq!(action_name("window/toggle"), "window-toggle");
        assert_eq!("window-toggle".replacen('-', "/", 1), "window/toggle");
    }

    #[test]
    fn a_window_is_the_game_only_when_it_says_so() {
        let is_game = |class: &str, title: &str| class == "steam_app_230410" || title == "Warframe";
        let info = |class: &str, title: &str, minimized: bool| {
            let dict = glib::VariantDict::new(None);
            dict.insert_value("resourceClass", &class.to_variant());
            dict.insert_value("caption", &title.to_variant());
            dict.insert_value("minimized", &minimized.to_variant());
            dict.insert_value("x", &1920.0f64.to_variant());
            dict.insert_value("y", &0.0f64.to_variant());
            dict.insert_value("width", &2560.0f64.to_variant());
            dict.insert_value("height", &1440.0f64.to_variant());
            dict.end()
        };
        assert_eq!(
            window_rect(&info("steam_app_230410", "Warframe", false), &is_game),
            Some(Rect { x: 1920, y: 0, width: 2560, height: 1440 })
        );
        assert_eq!(window_rect(&info("firefox", "Glaive Riven Mods | Warframe Market", false), &is_game), None);
        assert_eq!(window_rect(&info("steam_app_230410", "Warframe", true), &is_game), None, "minimized");
    }
}
