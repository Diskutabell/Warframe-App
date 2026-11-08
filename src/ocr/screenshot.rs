//! Taking a screenshot of the game.

use std::fs;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};
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

#[cfg(unix)]
/// Auto-detection order: wlroots-style compositors, KDE, GNOME, then X11 tools.
const AUTO: [ScreenshotTool; 6] = [
    ScreenshotTool::Grim,
    ScreenshotTool::Spectacle,
    ScreenshotTool::GnomeScreenshot,
    ScreenshotTool::Maim,
    ScreenshotTool::Scrot,
    ScreenshotTool::Import,
];

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
        ScreenshotTool::Auto => AUTO.into_iter().find(|t| which(program(*t)).is_some()),
        t => which(program(t)).map(|_| t),
    }
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
        // PPM straight to stdout: PNG compression of a 2560x1440 frame alone took ~1 s.
        let mut cmd = Command::new("grim");
        cmd.args(["-t", "ppm"]);
        if let Some(output) = compositor::game_output() {
            cmd.args(["-o", &output]);
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
