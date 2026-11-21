//! Asking the compositor where the game is, floating the main window, and installing the scan
//! hotkeys.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::{UserError, ipc};

/// Warframe under Proton: the window class is stable, the title is not localised.
const GAME_CLASSES: [&str; 3] = ["steam_app_230410", "Warframe.x64.exe", "Warframe.exe"];

fn is_game(class: &str, title: &str) -> bool {
    GAME_CLASSES.contains(&class) || title.trim().eq_ignore_ascii_case("warframe")
}

pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(program)).find(|p| {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

/// Hyprland's answer to a JSON query (`clients`, `monitors`); None on any failure.
fn hypr_json(what: &str) -> Option<Value> {
    ipc::hyprland_json(&format!("j/{what}")).ok()
}

/// Sway's answer to a query; None on any failure.
fn sway_json(kind: ipc::SwayMessage) -> Option<Value> {
    ipc::sway(kind, "").ok()
}

/// The compositor this session runs, from the environment it gives its clients. Asking which
/// tools are installed is not enough: a machine can have both hyprctl and swaymsg.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Running {
    Hyprland,
    Sway,
}

fn running() -> Option<Running> {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("HYPRLAND_INSTANCE_SIGNATURE") {
        Some(Running::Hyprland)
    } else if set("SWAYSOCK") {
        Some(Running::Sway)
    } else {
        None
    }
}

/// A compositor that speaks wlr-layer-shell, so the overlays can sit above a fullscreen game.
pub fn has_layer_shell() -> bool {
    let env = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if !env("WAYLAND_DISPLAY") {
        return false;
    }
    if env("HYPRLAND_INSTANCE_SIGNATURE") || env("SWAYSOCK") || env("NIRI_SOCKET") {
        return true;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    ["kde", "hyprland", "sway", "river", "wayfire", "labwc", "niri", "cosmic"].iter().any(|d| desktop.contains(d))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Warframe's window in compositor (layout) coordinates.
/// The game's window among Hyprland's clients: a mapped window of some size. Proton also keeps
/// unmapped 0x0 helper windows of the same class (on monitor -1), which must not be taken for it.
fn hypr_game(clients: &Value) -> Option<&Value> {
    clients.as_array()?.iter().find(|c| {
        let size = |i: usize| c["size"][i].as_i64().unwrap_or(0);
        c["mapped"].as_bool() != Some(false)
            && size(0) > 0
            && size(1) > 0
            && is_game(c["class"].as_str().unwrap_or(""), c["title"].as_str().unwrap_or(""))
    })
}

/// The game's window in Sway's tree, with the output it is on. Only windows count (tiled or
/// floating): a workspace may be called "Warframe" too.
fn sway_game<'a>(n: &'a Value, output: Option<&'a str>) -> Option<(&'a Value, Option<&'a str>)> {
    let output = if n["type"] == "output" { n["name"].as_str() } else { output };
    if matches!(n["type"].as_str(), Some("con" | "floating_con")) {
        let class = n["app_id"].as_str().or_else(|| n["window_properties"]["class"].as_str()).unwrap_or("");
        let sized = n["rect"]["width"].as_i64().unwrap_or(0) > 0 && n["rect"]["height"].as_i64().unwrap_or(0) > 0;
        if sized && is_game(class, n["name"].as_str().unwrap_or("")) {
            return Some((n, output));
        }
    }
    n["nodes"].as_array().into_iter().chain(n["floating_nodes"].as_array()).flatten().find_map(|c| sway_game(c, output))
}

pub fn game_rect() -> Option<Rect> {
    let at = |v: &Value, i: usize| v[i].as_i64().unwrap_or(0) as i32;
    if running()? == Running::Hyprland {
        let clients = hypr_json("clients")?;
        let c = hypr_game(&clients)?;
        return Some(Rect {
            x: at(&c["at"], 0),
            y: at(&c["at"], 1),
            width: at(&c["size"], 0),
            height: at(&c["size"], 1),
        });
    }
    let tree = sway_json(ipc::SwayMessage::GetTree)?;
    let (n, _) = sway_game(&tree, None)?;
    let g = &n["rect"];
    Some(Rect {
        x: g["x"].as_i64()? as i32,
        y: g["y"].as_i64()? as i32,
        width: g["width"].as_i64()? as i32,
        height: g["height"].as_i64()? as i32,
    })
}

/// The output (monitor) Warframe is on, by name, e.g. "DP-1".
///
/// `grim` with no -o stitches every monitor into one image, so on a multi-monitor desk the
/// centred-16:9 crop lands hundreds of pixels off. Capturing only the game's output avoids that.
pub fn game_output() -> Option<String> {
    // A reward scan captures every 120 ms; the game does not change monitors that fast.
    static LAST: std::sync::Mutex<Option<(std::time::Instant, Option<String>)>> = std::sync::Mutex::new(None);
    const FRESH: std::time::Duration = std::time::Duration::from_secs(2);
    if let Ok(last) = LAST.lock() {
        if let Some((_, output)) = last.as_ref().filter(|(at, _)| at.elapsed() < FRESH) {
            return output.clone();
        }
    }
    // Only a found monitor is kept: a game not up yet may be up a moment later.
    let output = ask_game_output();
    if let (Ok(mut last), Some(_)) = (LAST.lock(), &output) {
        *last = Some((std::time::Instant::now(), output.clone()));
    }
    output
}

fn ask_game_output() -> Option<String> {
    if running()? == Running::Hyprland {
        let clients = hypr_json("clients")?;
        let hit = hypr_game(&clients)?;
        let monitors = hypr_json("monitors")?;
        return monitors
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["id"] == hit["monitor"])
            .and_then(|m| m["name"].as_str().map(str::to_owned));
    }
    let tree = sway_json(ipc::SwayMessage::GetTree)?;
    sway_game(&tree, None).and_then(|(_, output)| output.map(str::to_owned))
}

/// A monitor in compositor (logical) coordinates.
#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub x: i32,
    pub y: i32,
}

pub fn monitors() -> Vec<Monitor> {
    let Some(compositor) = running() else { return Vec::new() };
    if compositor == Running::Hyprland {
        let Some(list) = hypr_json("monitors") else { return Vec::new() };
        return list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(Monitor {
                    name: m["name"].as_str()?.to_owned(),
                    x: m["x"].as_i64()? as i32,
                    y: m["y"].as_i64()? as i32,
                })
            })
            .collect();
    }
    if let Some(list) = sway_json(ipc::SwayMessage::GetOutputs) {
        return list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(Monitor {
                    name: m["name"].as_str()?.to_owned(),
                    x: m["rect"]["x"].as_i64()? as i32,
                    y: m["rect"]["y"].as_i64()? as i32,
                })
            })
            .collect();
    }
    Vec::new()
}

// ------------------------------------------------------------------ hotkeys

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mod {
    Super,
    Ctrl,
    Alt,
    Shift,
}

impl Mod {
    fn parse(token: &str) -> Option<Mod> {
        match token.to_ascii_uppercase().as_str() {
            "SUPER" | "WIN" | "WINDOWS" | "META" | "MOD4" => Some(Mod::Super),
            "CTRL" | "CONTROL" => Some(Mod::Ctrl),
            "ALT" | "MOD1" => Some(Mod::Alt),
            "SHIFT" => Some(Mod::Shift),
            _ => None,
        }
    }

    fn hypr(self) -> &'static str {
        match self {
            Mod::Super => "SUPER",
            Mod::Ctrl => "CTRL",
            Mod::Alt => "ALT",
            Mod::Shift => "SHIFT",
        }
    }

    fn sway(self) -> &'static str {
        match self {
            Mod::Super => "Mod4",
            Mod::Ctrl => "Ctrl",
            Mod::Alt => "Alt",
            Mod::Shift => "Shift",
        }
    }
}

/// Splits a hotkey like `"SUPER + shift + F9"` into `(modifiers, key)`.
fn parse_hotkey(raw: &str) -> Option<(Vec<Mod>, String)> {
    let mut mods = Vec::new();
    let mut key: Option<String> = None;
    for part in raw.split(['+', ' ']).map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(m) = Mod::parse(part) {
            if !mods.contains(&m) {
                mods.push(m);
            }
        } else if key.is_some() {
            return None;
        } else {
            key = Some(part.to_string());
        }
    }
    key.map(|k| (mods, k))
}

/// A key the compositor config can bind: letters, digits, '_', '+', '-' and spaces only.
///
/// The value ends up inside a compositor config file, which runs commands; anything else
/// (quotes, commas, newlines) could turn a key name into a config line of its own.
pub fn validate_key(key: &str) -> Result<()> {
    let chars_ok =
        key.len() <= 40 && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | ' '));
    let shape_ok = key.trim().is_empty() || parse_hotkey(key).is_some();
    if chars_ok && shape_ok {
        Ok(())
    } else {
        Err(UserError::new(format!("\"{key}\" is not a key name a compositor can bind.")).into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavour {
    HyprlandLua,
    Hyprland,
    Sway,
}

impl Flavour {
    pub fn label(self) -> &'static str {
        match self {
            Flavour::HyprlandLua => "hyprland-lua",
            Flavour::Hyprland => "hyprland",
            Flavour::Sway => "sway",
        }
    }
}

/// Hyprland can be configured by hyprland.conf or by Lua, and the bind syntax differs. A
/// Lua-driven Hyprland never reads hyprland.conf, and refuses `hyprctl keyword bind`.
fn hyprland_uses_lua() -> bool {
    // /systeminfo gathers system details first (it may run lspci) and can take a while; the
    // answer holds for the session, so it is asked until it once answers.
    static LUA: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if let Some(lua) = LUA.get() {
        return *lua;
    }
    match ipc::hyprland_within("/systeminfo", ipc::RELOAD_TIMEOUT) {
        Ok(info) => *LUA.get_or_init(|| info.contains("configProvider: lua")),
        Err(_) => false,
    }
}

pub fn detect_flavour() -> Option<Flavour> {
    match running()? {
        Running::Hyprland => Some(if hyprland_uses_lua() { Flavour::HyprlandLua } else { Flavour::Hyprland }),
        Running::Sway => Some(Flavour::Sway),
    }
}

/// The main window in Hyprland's rules (RE2 regexes): GTK's program name, set in `desktop::run`,
/// is the Wayland app id, and the X11 class too (capitalised) when GTK runs through XWayland.
const MAIN_WINDOW_CLASS: &str = "(?i)^warframeproject$";

/// The command that makes the compositor float the main window from the moment it opens. The
/// syntax is Hyprland 0.56's (checked with `Hyprland --verify-config`); older releases spell
/// rules differently and refuse this one.
/// Hyprland takes it as a request on its socket, Sway as a command.
fn float_rule(flavour: Flavour, pid: u32) -> String {
    match flavour {
        Flavour::HyprlandLua => format!(
            "/eval hl.window_rule({{ name = \"warframeproject-float\", match = {{ class = \"{MAIN_WINDOW_CLASS}\" }}, float = true }})"
        ),
        Flavour::Hyprland => format!("/keyword windowrule match:class {MAIN_WINDOW_CLASS}, float on"),
        // Sway's criteria can name the process, which covers an XWayland window as well.
        Flavour::Sway => format!("for_window [pid={pid}] floating enable"),
    }
}

/// Has a tiling compositor (Hyprland, Sway) float the main window, so it keeps its own size and
/// stays out of the tiling layout. Must run before the window is shown: a rule only applies to
/// windows that open after it, and floating the window later would first squeeze it into a tile.
/// The rule lives in the running compositor only - no config file is touched - and reloading the
/// compositor's config drops it, so every start sets it again. Other desktops float windows anyway.
pub fn float_main_window() -> Result<()> {
    let Some(flavour) = detect_flavour() else { return Ok(()) };
    let rule = float_rule(flavour, std::process::id());
    if flavour == Flavour::Sway {
        return ipc::sway_command(&rule, ipc::TIMEOUT);
    }
    ipc::hyprland_ok(&rule, ipc::TIMEOUT).context("Hyprland refused the rule")
}

/// The config lines for the given (key, API path) pairs.
pub fn hotkey_lines(flavour: Option<Flavour>, port: u16, keys: &[(&str, &str)]) -> Vec<String> {
    let cmd = |p: &str| format!("curl -s -X POST http://127.0.0.1:{port}/api/{p}");
    keys.iter()
        .filter_map(|(k, p)| {
            if k.trim().is_empty() || validate_key(k).is_err() {
                return None;
            }
            let (mods, key) = parse_hotkey(k)?;
            let line = match flavour {
                Some(Flavour::HyprlandLua) => {
                    let combo = if mods.is_empty() {
                        key
                    } else {
                        let m: Vec<&str> = mods.iter().map(|m| m.hypr()).collect();
                        format!("{}+{key}", m.join("+"))
                    };
                    format!("hl.bind(\"{combo}\", hl.dsp.exec_cmd(\"{}\"))", cmd(p))
                }
                Some(Flavour::Hyprland) => {
                    let m: Vec<&str> = mods.iter().map(|m| m.hypr()).collect();
                    format!("bind = {}, {key}, exec, {}", m.join(" "), cmd(p))
                }
                Some(Flavour::Sway) => {
                    let combo = if mods.is_empty() {
                        key
                    } else {
                        let m: Vec<&str> = mods.iter().map(|m| m.sway()).collect();
                        format!("{}+{key}", m.join("+"))
                    };
                    format!("bindsym {combo} exec {}", cmd(p))
                }
                None => format!("{k} -> {}", cmd(p)),
            };
            Some(line)
        })
        .collect()
}

/// What installing the hotkeys will touch, for the Settings page.
pub fn hotkey_note(flavour: Option<Flavour>) -> String {
    match flavour {
        Some(Flavour::HyprlandLua) => "Writes these into a marked block in ~/.config/hypr/lua/keybindings.lua \
                                       (replacing that block on later installs), then reloads Hyprland."
            .into(),
        Some(f) => format!(
            "Writes these to a file of their own and adds one line to your {} config that includes it, then reloads.",
            f.label()
        ),
        None => "No supported compositor detected - bind these commands yourself.".into(),
    }
}

pub struct Installed {
    pub file: PathBuf,
    pub sourced: bool,
}

/// Has the compositor read its config again.
fn reload(running: Running) -> Result<()> {
    let failed = |e: String| UserError::new(format!("Reloading the compositor's config failed: {e}"));
    match running {
        Running::Sway => ipc::sway_command("reload", ipc::RELOAD_TIMEOUT),
        Running::Hyprland => ipc::hyprland_ok("/reload", ipc::RELOAD_TIMEOUT),
    }
    .map_err(|e| failed(format!("{e:#}")).into())
}

/// Writes the binds to the compositor config and reloads it. Nothing of the user's own config is
/// changed beyond one managed block (Lua) or one include line (hyprland.conf, sway).
/// Replaces a compositor config file. A config that is a symlink (Home Manager, a dotfiles
/// repository) is written through, so the link stays a link; one that cannot be written (it links
/// into the read-only Nix store, say) is left alone, with what to do instead.
fn write_config(path: &Path, text: &str) -> Result<()> {
    let real = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    crate::config::write_atomic(&real, text.as_bytes()).map_err(|e| {
        if real == path {
            e
        } else {
            UserError::new(format!(
                "{} links to {}, which cannot be changed from here ({e:#}). Add the hotkey lines shown in Settings to your \
                 compositor's config yourself.",
                path.display(),
                real.display()
            ))
            .into()
        }
    })
}

pub fn install_hotkeys(flavour: Option<Flavour>, lines: &[String]) -> Result<Installed> {
    if lines.is_empty() {
        return Err(UserError::new("Set at least one key first.").into());
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    match flavour {
        Some(Flavour::HyprlandLua) => {
            let target = home.join(".config/hypr/lua/keybindings.lua");
            let text = fs::read_to_string(&target)
                .map_err(|_| UserError::new(format!("Could not find {}.", target.display())))?;
            let (head, tail) = ("-- >>> warframeproject hotkeys (managed)", "-- <<< warframeproject hotkeys");
            let block = format!("{head}\n{}\n{tail}", lines.join("\n"));
            let (text, added) = match (text.find(head), text.find(tail)) {
                (Some(a), Some(b)) if a < b => (format!("{}{block}{}", &text[..a], &text[b + tail.len()..]), false),
                _ => (format!("{}\n\n{block}\n", text.trim_end_matches('\n')), true),
            };
            write_config(&target, &text)?;
            reload(Running::Hyprland)?;
            Ok(Installed { file: target, sourced: added })
        }
        Some(f @ (Flavour::Hyprland | Flavour::Sway)) => {
            let (dir, main, sub, running) = if f == Flavour::Hyprland {
                (home.join(".config/hypr"), "hyprland.conf", "config", Running::Hyprland)
            } else {
                (home.join(".config/sway"), "config", "config.d", Running::Sway)
            };
            let main = dir.join(main);
            if !main.is_file() {
                return Err(UserError::new(format!("Could not find {}.", main.display())).into());
            }
            let target = if dir.join(sub).is_dir() { dir.join(sub) } else { dir.clone() }.join("warframeproject.conf");
            write_config(&target, &format!("# Written by warframeproject. Safe to delete.\n{}\n", lines.join("\n")))?;
            let include = if f == Flavour::Hyprland {
                format!("source = {}", target.display())
            } else {
                format!("include {}", target.display())
            };
            let text = fs::read_to_string(&main)?;
            let added = !text.contains(&target.display().to_string());
            if added {
                write_config(
                    &main,
                    &format!("{}\n\n# warframeproject hotkeys\n{include}\n", text.trim_end_matches('\n')),
                )?;
            }
            reload(running)?;
            Ok(Installed { file: target, sourced: added })
        }
        None => Err(UserError::new("No supported compositor found (Hyprland or Sway).").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_window_is_a_mapped_window_of_some_size() {
        use serde_json::json;
        let clients = json!([
            {"class": "steam_app_230410", "title": "", "mapped": false, "size": [0, 0], "at": [0, 0], "monitor": -1},
            {"class": "steam_app_230410", "title": "Warframe", "mapped": true, "size": [2560, 1440], "at": [1920, 0], "monitor": 1},
        ]);
        assert_eq!(hypr_game(&clients).unwrap()["monitor"], 1);
        let tree = json!({"type": "root", "nodes": [{"type": "output", "name": "DP-3", "nodes": [
            {"type": "workspace", "name": "Warframe", "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440}, "nodes": [],
             "floating_nodes": [{"type": "floating_con", "name": "Warframe", "app_id": null,
                                 "window_properties": {"class": "steam_app_230410"},
                                 "rect": {"x": 10, "y": 20, "width": 1920, "height": 1080}}]}
        ]}]});
        let (window, output) = sway_game(&tree, None).unwrap();
        assert_eq!(
            (window["rect"]["x"].as_i64(), output),
            (Some(10), Some("DP-3")),
            "the floating window, not the workspace"
        );
        let empty = json!({"type": "root", "nodes": [{"type": "output", "name": "DP-3", "nodes": [
            {"type": "workspace", "name": "Warframe", "rect": {"x": 0, "y": 0, "width": 2560, "height": 1440}, "nodes": []}
        ]}]});
        assert!(sway_game(&empty, None).is_none(), "a workspace named Warframe is not the game");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_stays_a_link() {
        let dir = std::env::temp_dir().join(format!("wfp-hotkeys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.lua");
        let link = dir.join("keybindings.lua");
        fs::write(&real, "-- mine\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_config(&link, "-- mine\n-- added\n").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "still a link");
        assert_eq!(fs::read_to_string(&real).unwrap(), "-- mine\n-- added\n", "written through it");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_cannot_smuggle_config() {
        assert!(validate_key("F9").is_ok());
        assert!(validate_key("SUPER + F9").is_ok());
        assert!(validate_key("").is_ok());
        assert!(validate_key("F9\"), os.execute(\"rm").is_err());
        assert!(validate_key("F9\nexec-once = x").is_err());
        assert!(validate_key("F9, exec, x").is_err());
        assert!(validate_key("SUPER").is_err());
        assert!(validate_key("F9 + F10").is_err());
    }

    #[test]
    fn lines_per_flavour() {
        let keys = [("F9", "rewards/scan"), ("", "riven/scan")];
        assert_eq!(
            hotkey_lines(Some(Flavour::HyprlandLua), 8642, &keys),
            ["hl.bind(\"F9\", hl.dsp.exec_cmd(\"curl -s -X POST http://127.0.0.1:8642/api/rewards/scan\"))"]
        );
        assert_eq!(
            hotkey_lines(Some(Flavour::Hyprland), 8642, &keys),
            ["bind = , F9, exec, curl -s -X POST http://127.0.0.1:8642/api/rewards/scan"]
        );
        assert_eq!(
            hotkey_lines(Some(Flavour::Sway), 1, &keys),
            ["bindsym F9 exec curl -s -X POST http://127.0.0.1:1/api/rewards/scan"]
        );

        let modded = [("SUPER + F9", "rewards/scan"), ("ctrl + shift + F10", "riven/scan")];
        assert_eq!(
            hotkey_lines(Some(Flavour::Hyprland), 8642, &modded),
            [
                "bind = SUPER, F9, exec, curl -s -X POST http://127.0.0.1:8642/api/rewards/scan",
                "bind = CTRL SHIFT, F10, exec, curl -s -X POST http://127.0.0.1:8642/api/riven/scan",
            ]
        );
        assert_eq!(
            hotkey_lines(Some(Flavour::Sway), 8642, &modded),
            [
                "bindsym Mod4+F9 exec curl -s -X POST http://127.0.0.1:8642/api/rewards/scan",
                "bindsym Ctrl+Shift+F10 exec curl -s -X POST http://127.0.0.1:8642/api/riven/scan",
            ]
        );
        assert_eq!(
            hotkey_lines(Some(Flavour::HyprlandLua), 8642, &modded),
            [
                "hl.bind(\"SUPER+F9\", hl.dsp.exec_cmd(\"curl -s -X POST http://127.0.0.1:8642/api/rewards/scan\"))",
                "hl.bind(\"CTRL+SHIFT+F10\", hl.dsp.exec_cmd(\"curl -s -X POST http://127.0.0.1:8642/api/riven/scan\"))",
            ]
        );
    }

    #[test]
    fn float_rules_per_flavour() {
        let rule = |f| float_rule(f, 4242);
        assert_eq!(
            rule(Flavour::HyprlandLua),
            "/eval hl.window_rule({ name = \"warframeproject-float\", \
             match = { class = \"(?i)^warframeproject$\" }, float = true })"
        );
        assert_eq!(rule(Flavour::Hyprland), "/keyword windowrule match:class (?i)^warframeproject$, float on");
        assert_eq!(rule(Flavour::Sway), "for_window [pid=4242] floating enable");
    }
}
