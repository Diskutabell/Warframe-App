//! Asking the compositor where the game is, and installing the scan hotkeys.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::{UserError, process};

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

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs a query command and parses its stdout as JSON; None on any failure.
fn query_json(cmd: &str, args: &[&str]) -> Option<Value> {
    which(cmd)?;
    let out = process::run(Command::new(cmd).args(args), None, QUERY_TIMEOUT).ok()?;
    serde_json::from_slice(&out.stdout).ok()
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
pub fn game_rect() -> Option<Rect> {
    if running()? == Running::Hyprland {
        let clients = query_json("hyprctl", &["clients", "-j"])?;
        for c in clients.as_array().into_iter().flatten() {
            if is_game(c["class"].as_str().unwrap_or(""), c["title"].as_str().unwrap_or("")) {
                let at = |v: &Value, i| v[i].as_i64().unwrap_or(0) as i32;
                let r = Rect {
                    x: at(&c["at"], 0),
                    y: at(&c["at"], 1),
                    width: at(&c["size"], 0),
                    height: at(&c["size"], 1),
                };
                if r.width > 0 && r.height > 0 {
                    return Some(r);
                }
            }
        }
        return None;
    }
    let tree = query_json("swaymsg", &["-t", "get_tree"])?;
    fn walk(n: &Value) -> Option<Rect> {
        let class = n["app_id"].as_str().or_else(|| n["window_properties"]["class"].as_str()).unwrap_or("");
        if is_game(class, n["name"].as_str().unwrap_or("")) {
            let g = &n["rect"];
            let r = Rect {
                x: g["x"].as_i64()? as i32,
                y: g["y"].as_i64()? as i32,
                width: g["width"].as_i64()? as i32,
                height: g["height"].as_i64()? as i32,
            };
            if r.width > 0 && r.height > 0 {
                return Some(r);
            }
        }
        n["nodes"].as_array().into_iter().chain(n["floating_nodes"].as_array()).flatten().find_map(walk)
    }
    walk(&tree)
}

/// The output (monitor) Warframe is on, by name, e.g. "DP-1".
///
/// `grim` with no -o stitches every monitor into one image, so on a multi-monitor desk the
/// centred-16:9 crop lands hundreds of pixels off. Capturing only the game's output avoids that.
pub fn game_output() -> Option<String> {
    if running()? == Running::Hyprland {
        let clients = query_json("hyprctl", &["clients", "-j"])?;
        let hit = clients
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| is_game(c["class"].as_str().unwrap_or(""), c["title"].as_str().unwrap_or("")))?;
        let monitors = query_json("hyprctl", &["monitors", "-j"])?;
        return monitors
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["id"] == hit["monitor"])
            .and_then(|m| m["name"].as_str().map(str::to_owned));
    }
    let tree = query_json("swaymsg", &["-t", "get_tree"])?;
    fn walk(n: &Value, output: Option<&str>) -> Option<String> {
        let output = if n["type"] == "output" { n["name"].as_str() } else { output };
        let class = n["app_id"].as_str().or_else(|| n["window_properties"]["class"].as_str()).unwrap_or("");
        let title = if n["type"] == "con" { n["name"].as_str().unwrap_or("") } else { "" };
        if is_game(class, title) {
            return output.map(str::to_owned);
        }
        n["nodes"].as_array().into_iter().chain(n["floating_nodes"].as_array()).flatten().find_map(|c| walk(c, output))
    }
    walk(&tree, None)
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
        let Some(list) = query_json("hyprctl", &["monitors", "-j"]) else { return Vec::new() };
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
    if let Some(list) = query_json("swaymsg", &["-t", "get_outputs"]) {
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

/// A key the compositor config can bind: letters, digits, '_', '+', '-' and spaces only.
///
/// The value ends up inside a compositor config file, which runs commands; anything else
/// (quotes, commas, newlines) could turn a key name into a config line of its own.
pub fn validate_key(key: &str) -> Result<()> {
    let ok = key.len() <= 40 && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | ' '));
    if ok { Ok(()) } else { Err(UserError::new(format!("\"{key}\" is not a key name a compositor can bind.")).into()) }
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
    process::run(Command::new("hyprctl").arg("systeminfo"), None, QUERY_TIMEOUT)
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("configProvider: lua"))
}

pub fn detect_flavour() -> Option<Flavour> {
    match running()? {
        Running::Hyprland => Some(if hyprland_uses_lua() { Flavour::HyprlandLua } else { Flavour::Hyprland }),
        Running::Sway => Some(Flavour::Sway),
    }
}

/// The config lines for the given (key, API path) pairs.
pub fn hotkey_lines(flavour: Option<Flavour>, port: u16, keys: &[(&str, &str)]) -> Vec<String> {
    let cmd = |p: &str| format!("curl -s -X POST http://127.0.0.1:{port}/api/{p}");
    keys.iter()
        .filter(|(k, _)| !k.trim().is_empty() && validate_key(k).is_ok())
        .map(|(k, p)| match flavour {
            Some(Flavour::HyprlandLua) => format!("hl.bind(\"{k}\", hl.dsp.exec_cmd(\"{}\"))", cmd(p)),
            Some(Flavour::Hyprland) => format!("bind = , {k}, exec, {}", cmd(p)),
            Some(Flavour::Sway) => format!("bindsym {k} exec {}", cmd(p)),
            None => format!("{k} -> {}", cmd(p)),
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

fn reload(cmd: &str) -> Result<()> {
    let out = process::run(Command::new(cmd).arg("reload"), None, Duration::from_secs(15))?;
    if !out.success {
        let msg =
            String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string();
        return Err(
            UserError::new(format!("{cmd} reload failed: {}", msg.chars().take(160).collect::<String>())).into()
        );
    }
    // hyprctl reports some failures on stdout with status 0.
    let said = String::from_utf8_lossy(&out.stdout).to_lowercase();
    if said.contains("error") {
        return Err(UserError::new(format!("{cmd} reload reported: {}", said.trim())).into());
    }
    Ok(())
}

/// Writes the binds to the compositor config and reloads it. Nothing of the user's own config is
/// changed beyond one managed block (Lua) or one include line (hyprland.conf, sway).
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
            crate::config::write_atomic(&target, text.as_bytes())?;
            reload("hyprctl")?;
            Ok(Installed { file: target, sourced: added })
        }
        Some(f @ (Flavour::Hyprland | Flavour::Sway)) => {
            let (dir, main, sub, reload_cmd) = if f == Flavour::Hyprland {
                (home.join(".config/hypr"), "hyprland.conf", "config", "hyprctl")
            } else {
                (home.join(".config/sway"), "config", "config.d", "swaymsg")
            };
            let main = dir.join(main);
            if !main.is_file() {
                return Err(UserError::new(format!("Could not find {}.", main.display())).into());
            }
            let target = if dir.join(sub).is_dir() { dir.join(sub) } else { dir.clone() }.join("warframeproject.conf");
            fs::write(&target, format!("# Written by warframeproject. Safe to delete.\n{}\n", lines.join("\n")))?;
            let include = if f == Flavour::Hyprland {
                format!("source = {}", target.display())
            } else {
                format!("include {}", target.display())
            };
            let text = fs::read_to_string(&main)?;
            let added = !text.contains(&target.display().to_string());
            if added {
                fs::write(&main, format!("{}\n\n# warframeproject hotkeys\n{include}\n", text.trim_end_matches('\n')))?;
            }
            reload(reload_cmd)?;
            Ok(Installed { file: target, sourced: added })
        }
        None => Err(UserError::new("No supported compositor found (hyprctl or swaymsg).").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_cannot_smuggle_config() {
        assert!(validate_key("F9").is_ok());
        assert!(validate_key("SUPER + F9").is_ok());
        assert!(validate_key("").is_ok());
        assert!(validate_key("F9\"), os.execute(\"rm").is_err());
        assert!(validate_key("F9\nexec-once = x").is_err());
        assert!(validate_key("F9, exec, x").is_err());
    }

    #[test]
    fn lines_per_flavour() {
        let keys = [("F9", "rewards/scan"), ("", "riven/scan")];
        assert_eq!(
            hotkey_lines(Some(Flavour::HyprlandLua), 8642, &keys),
            ["hl.bind(\"F9\", hl.dsp.exec_cmd(\"curl -s -X POST http://127.0.0.1:8642/api/rewards/scan\"))"]
        );
        assert_eq!(
            hotkey_lines(Some(Flavour::Sway), 1, &keys),
            ["bindsym F9 exec curl -s -X POST http://127.0.0.1:1/api/rewards/scan"]
        );
    }
}
