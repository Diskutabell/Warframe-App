//! Taking a screenshot of the game.

use std::fs;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result};

use super::image::Rgb;
#[cfg(unix)]
use crate::compositor::{self, which};
#[cfg(unix)]
use crate::config;
use crate::config::ScreenshotTool;
#[cfg(unix)]
use crate::process;

// Windows needs no screenshot program: the app copies the screen itself, and the tool setting
// does not apply.
#[cfg(windows)]
pub fn describe(tool: ScreenshotTool) -> &'static str {
    program(tool)
}

#[cfg(windows)]
pub fn program(_tool: ScreenshotTool) -> &'static str {
    "built in"
}

#[cfg(windows)]
pub fn resolve(_setting: ScreenshotTool) -> Option<ScreenshotTool> {
    Some(ScreenshotTool::Auto)
}

#[cfg(windows)]
pub fn capture(_setting: ScreenshotTool) -> Result<Rgb> {
    crate::compositor::capture()
}

/// Whether a capture is quick and quiet enough to repeat several times a second.
#[cfg(windows)]
pub fn cheap(_setting: ScreenshotTool) -> bool {
    true
}

/// Whether a capture is quick and quiet enough to repeat several times a second: grim takes
/// 20-40 ms for a 2560x1440 output. The other tools start a whole program, and some flash the
/// screen or play a shutter sound.
#[cfg(unix)]
pub fn cheap(setting: ScreenshotTool) -> bool {
    resolve(setting) == Some(ScreenshotTool::Grim)
}

#[cfg(unix)]
const TIMEOUT: Duration = Duration::from_secs(10);

/// The order Auto tries the tools in, for the session the app runs in. grim works only on
/// wlroots-style compositors (Hyprland, Sway and the like); KDE and GNOME have their own tools on
/// Wayland; an X11 session has the classic ones (grim cannot capture there at all).
#[cfg(unix)]
fn auto_order(wayland: bool, desktop: &str) -> &'static [ScreenshotTool] {
    use ScreenshotTool::*;
    let desktop = desktop.to_ascii_lowercase();
    if !wayland {
        &[Maim, Scrot, Import, Spectacle, GnomeScreenshot]
    } else if desktop.contains("kde") {
        &[Spectacle, Grim, GnomeScreenshot]
    } else if desktop.contains("gnome") {
        &[GnomeScreenshot, Spectacle, Grim]
    } else {
        &[Grim, Spectacle, GnomeScreenshot, Maim, Scrot, Import]
    }
}

#[cfg(unix)]
pub fn program(tool: ScreenshotTool) -> &'static str {
    match tool {
        ScreenshotTool::Auto | ScreenshotTool::Grim => "grim",
        ScreenshotTool::Spectacle => "spectacle",
        ScreenshotTool::GnomeScreenshot => "gnome-screenshot",
        ScreenshotTool::Maim => "maim",
        ScreenshotTool::Scrot => "scrot",
        ScreenshotTool::Import => "import",
    }
}

#[cfg(unix)]
/// The tool a capture would use, if one is installed.
pub fn resolve(setting: ScreenshotTool) -> Option<ScreenshotTool> {
    match setting {
        ScreenshotTool::Auto => {
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
            let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            auto_order(wayland, &desktop).iter().copied().find(|t| available(*t))
        }
        t => available(t).then_some(t),
    }
}

/// What a capture with `tool` uses, for the Settings page.
#[cfg(unix)]
pub fn describe(tool: ScreenshotTool) -> &'static str {
    if tool == ScreenshotTool::Grim && super::screencopy::possible() {
        return if which("grim").is_some() { "built in (grim as fallback)" } else { "built in" };
    }
    program(tool)
}

/// A tool can be used: its program is installed, or (grim) the app copies the screen itself.
#[cfg(unix)]
fn available(tool: ScreenshotTool) -> bool {
    (tool == ScreenshotTool::Grim && super::screencopy::possible()) || which(program(tool)).is_some()
}

#[cfg(unix)]
/// A file only this capture uses, in the app's own cache directory (not a guessable name in /tmp).
fn private_file() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = config::dirs().cache.join("capture");
    let _ = fs::create_dir_all(&dir);
    dir.join(format!("shot-{}-{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

#[cfg(unix)]
pub fn capture(setting: ScreenshotTool) -> Result<Rgb> {
    let Some(tool) = resolve(setting) else {
        bail!(
            "No screenshot tool found. Install grim (Hyprland, Sway), spectacle (KDE), gnome-screenshot, maim or scrot."
        );
    };
    if tool == ScreenshotTool::Grim {
        let output = compositor::game_output();
        // What grim does, without starting it; grim itself when that fails.
        match super::screencopy::capture(output.as_deref()) {
            Ok(frame) => return Ok(frame),
            Err(e) if which("grim").is_none() => {
                return Err(e.context("the screen could not be copied, and grim (the fallback) is not installed"));
            }
            Err(e) => {
                static SAID: AtomicBool = AtomicBool::new(false);
                if !SAID.swap(true, Ordering::Relaxed) {
                    eprintln!("capturing through grim instead: {e:#}");
                }
            }
        }
        // PPM straight to stdout: PNG compression of a 2560x1440 frame alone took ~1 s.
        let mut cmd = Command::new("grim");
        cmd.args(["-t", "ppm"]);
        if let Some(output) = &output {
            cmd.args(["-o", output]);
        }
        cmd.arg("-");
        let out = process::run(&mut cmd, None, TIMEOUT)?;
        if !out.success {
            bail!("grim failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        return decode(&out.stdout);
    }
    let file = private_file();
    let path = file.to_string_lossy().into_owned();
    let args: Vec<&str> = match tool {
        ScreenshotTool::Spectacle => vec!["-b", "-n", "-f", "-o", &path],
        ScreenshotTool::GnomeScreenshot => vec!["-f", &path],
        ScreenshotTool::Maim => vec![&path],
        ScreenshotTool::Scrot => vec!["-o", &path],
        ScreenshotTool::Import => vec!["-window", "root", &path],
        ScreenshotTool::Auto | ScreenshotTool::Grim => unreachable!("handled above"),
    };
    let result = process::run(Command::new(program(tool)).args(&args), None, TIMEOUT)
        .and_then(|out| {
            if !out.success {
                bail!("{} failed: {}", program(tool), String::from_utf8_lossy(&out.stderr).trim());
            }
            fs::read(&file).with_context(|| format!("{} produced no file", program(tool)))
        })
        .and_then(|bytes| decode(&bytes));
    let _ = fs::remove_file(&file);
    result
}

pub fn decode(bytes: &[u8]) -> Result<Rgb> {
    let img = image::load_from_memory(bytes).context("the screenshot could not be decoded")?;
    Ok(Rgb::from_image(img.to_rgb8()))
}

pub fn load(path: &std::path::Path) -> Result<Rgb> {
    decode(&fs::read(path).with_context(|| format!("reading {}", path.display()))?)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn auto_picks_a_tool_that_works_in_the_session() {
        use ScreenshotTool::*;
        assert_eq!(auto_order(true, "Hyprland")[0], Grim);
        assert_eq!(auto_order(true, "sway")[0], Grim);
        assert_eq!(auto_order(true, "KDE")[0], Spectacle);
        assert_eq!(auto_order(true, "ubuntu:GNOME")[0], GnomeScreenshot);
        assert!(!auto_order(false, "KDE").contains(&Grim), "grim cannot capture an X11 session");
    }
}
