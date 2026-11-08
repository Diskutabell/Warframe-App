//! Paths, persistent settings and small JSON file helpers.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::UserError;

pub const APP: &str = "warframeproject";
pub const USER_AGENT: &str = concat!("warframeproject/", env!("CARGO_PKG_VERSION"), " (+Linux Warframe companion)");

pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

/// XDG directories, created on first use. An empty or relative XDG variable is ignored, as the
/// spec says, rather than turning into a directory relative to wherever the app was started.
#[cfg(unix)]
pub fn dirs() -> &'static Dirs {
    static DIRS: OnceLock<Dirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
        let xdg = |var: &str, fallback: &str| {
            let base = std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback));
            let dir = base.join(APP);
            let _ = fs::create_dir_all(&dir);
            dir
        };
        Dirs {
            config: xdg("XDG_CONFIG_HOME", ".config"),
            data: xdg("XDG_DATA_HOME", ".local/share"),
            cache: xdg("XDG_CACHE_HOME", ".cache"),
        }
    })
}

/// Windows: settings in %APPDATA% (they roam with the account), everything else in
/// %LOCALAPPDATA%, with the cache in a folder of its own.
#[cfg(windows)]
pub fn dirs() -> &'static Dirs {
    static DIRS: OnceLock<Dirs> = OnceLock::new();
    DIRS.get_or_init(|| {
        let known = |var: &str| {
            std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(std::env::temp_dir)
        };
        let made = |dir: PathBuf| {
            let _ = fs::create_dir_all(&dir);
            dir
        };
        let local = known("LOCALAPPDATA").join(APP);
        Dirs { config: made(known("APPDATA").join(APP)), cache: made(local.join("cache")), data: made(local) }
    })
}

pub fn settings_file() -> PathBuf {
    dirs().config.join("settings.json")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    #[default]
    Pc,
    Ps4,
    Xb1,
    Swi,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Pc => "pc",
            Platform::Ps4 => "ps4",
            Platform::Xb1 => "xb1",
            Platform::Swi => "swi",
        }
    }
}

/// Which program takes screenshots. A fixed list on purpose: the old free-form command setting
/// could be changed through the local API and was then run as a program.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenshotTool {
    #[default]
    Auto,
    Grim,
    Spectacle,
    GnomeScreenshot,
    Maim,
    Scrot,
    Import,
}

/// How the overlays stay above the game.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayBackend {
    /// Layer shell where the compositor has it (Hyprland, Sway, KDE), X11 otherwise.
    #[default]
    Auto,
    LayerShell,
    X11,
}

/// Where the price panels sit on the reward screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PanelPosition {
    /// Under the cards, over the row of player names.
    #[default]
    Below,
    /// Above the cards, so the row under them stays in view: it shows what the squad picks.
    Above,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub port: u16,
    pub platform: Platform,
    /// Empty: search Steam, Flatpak, Snap and ~/Games prefixes.
    pub ee_log_path: String,
    pub profile_auto_refresh: bool,
    pub profile_refresh_minutes: u32,
    pub inventory_json_path: String,
    pub hotkey_rewards: String,
    pub hotkey_riven: String,
    pub riven_overlay_enabled: bool,
    pub riven_overlay_seconds: u32,
    pub riven_overlay_x_percent: f64,
    pub riven_overlay_y_percent: f64,
    /// Last resort; read-only, off by default. When on, the inventory is read automatically after
    /// login and after each mission (see `State::schedule_memscan`).
    pub memory_scan_enabled: bool,
    pub ocr_enabled: bool,
    pub screenshot_tool: ScreenshotTool,
    /// Extra desktop notification for each reward screen (the overlay already shows prices).
    pub notify: bool,
    /// Announce fissures matching `fissure_watch` when they open.
    pub notify_fissures: bool,
    /// Watched fissures as "tier|mission|kind", kind being normal, steel or storm.
    pub fissure_watch: Vec<String>,
    /// Announce Baro Ki'Teer's arrival.
    pub notify_baro: bool,
    /// Announce finished foundry builds (needs an inventory).
    pub notify_foundry: bool,
    pub overlay_enabled: bool,
    /// The longest the price panels stay up; they go with the reward screen, usually sooner.
    pub overlay_seconds: u32,
    pub overlay_position: PanelPosition,
    pub overlay_backend: OverlayBackend,
    /// Bulk prices; reward-screen prices refresh after 15 min regardless.
    pub price_cache_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            port: 8642,
            platform: Platform::Pc,
            ee_log_path: String::new(),
            profile_auto_refresh: false,
            profile_refresh_minutes: 30,
            inventory_json_path: String::new(),
            hotkey_rewards: "F9".into(),
            hotkey_riven: "F10".into(),
            riven_overlay_enabled: true,
            riven_overlay_seconds: 20,
            riven_overlay_x_percent: 57.0,
            riven_overlay_y_percent: 50.0,
            memory_scan_enabled: false,
            ocr_enabled: true,
            screenshot_tool: ScreenshotTool::Auto,
            notify: false,
            notify_fissures: false,
            fissure_watch: Vec::new(),
            notify_baro: false,
            notify_foundry: false,
            overlay_enabled: true,
            overlay_seconds: 15,
            overlay_position: PanelPosition::Below,
            overlay_backend: OverlayBackend::Auto,
            price_cache_minutes: 360,
        }
    }
}

impl Settings {
    /// Settings from disk. A missing or broken file gives the defaults; a field of the wrong type
    /// falls back to its default instead of discarding the whole file.
    pub fn load() -> Settings {
        let Some(raw) = read_json::<serde_json::Value>(&settings_file()) else {
            return Settings::default();
        };
        Settings::from_value(raw).unwrap_or_else(|_| Settings::default())
    }

    fn from_value(raw: serde_json::Value) -> Result<Settings> {
        let mut merged = serde_json::to_value(Settings::default())?;
        if let (Some(dst), Some(src)) = (merged.as_object_mut(), raw.as_object()) {
            for (key, value) in src {
                if let Some(slot) = dst.get_mut(key) {
                    let previous = std::mem::replace(slot, value.clone());
                    // Probe the single field: keep the default if this value does not fit.
                    if serde_json::from_value::<Settings>(serde_json::Value::Object(dst.clone())).is_err() {
                        dst.insert(key.clone(), previous);
                    }
                }
            }
        }
        Ok(serde_json::from_value(merged)?)
    }

    /// Applies a partial update from the UI. Unknown keys are ignored; a value of the wrong type
    /// or an unusable hotkey is refused with a message the UI can show.
    pub fn patched(&self, patch: &serde_json::Value) -> Result<Settings> {
        let mut current = serde_json::to_value(self)?;
        let Some(fields) = patch.as_object() else {
            return Err(UserError::new("Settings must be a JSON object.").into());
        };
        let dst = current.as_object_mut().expect("settings serialise to an object");
        for (key, value) in fields {
            if dst.contains_key(key) {
                dst.insert(key.clone(), value.clone());
            }
        }
        let updated: Settings = serde_json::from_value(current)
            .map_err(|e| UserError::new(format!("That setting has the wrong type: {e}")))?;
        for key in [&updated.hotkey_rewards, &updated.hotkey_riven] {
            crate::compositor::validate_key(key)?;
        }
        Ok(updated)
    }

    pub fn save(&self) -> Result<()> {
        write_json_pretty(&settings_file(), self)
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read(path).ok()?;
    serde_json::from_slice(&text).ok()
}

/// Writes through a temporary file and a rename, so a crash never leaves half a file behind.
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec(value)?)
}

pub fn write_json_pretty<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut file = fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_settings_load_without_losing_known_fields() {
        // A settings file from an early version: two keys that no longer exist.
        let raw = json!({"port": 9000, "screenshot_command": "", "overlay_force_x11": true,
                         "memory_scan_enabled": true, "riven_overlay_x_percent": 60});
        let s = Settings::from_value(raw).unwrap();
        assert_eq!(s.port, 9000);
        assert!(s.memory_scan_enabled);
        assert_eq!(s.riven_overlay_x_percent, 60.0);
        assert_eq!(s.screenshot_tool, ScreenshotTool::Auto);
    }

    #[test]
    fn one_bad_field_keeps_its_default_and_the_rest() {
        let s = Settings::from_value(json!({"port": "nope", "overlay_seconds": 9})).unwrap();
        assert_eq!(s.port, 8642);
        assert_eq!(s.overlay_seconds, 9);
    }

    #[test]
    fn patch_refuses_wrong_types_and_unsafe_hotkeys() {
        let s = Settings::default();
        assert!(s.patched(&json!({"overlay_seconds": "abc"})).is_err());
        assert!(s.patched(&json!({"hotkey_rewards": "F9\nexec-once = rm -rf ~"})).is_err());
        let ok = s.patched(&json!({"overlay_seconds": 20, "unknown": 1})).unwrap();
        assert_eq!(ok.overlay_seconds, 20);
    }
}
