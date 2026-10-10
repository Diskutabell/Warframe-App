//! Taking a screenshot of the game.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::image::Rgb;
use crate::compositor::{self, which};
use crate::config::{self, ScreenshotTool};
use crate::process;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Auto-detection order: wlroots-style compositors, KDE, GNOME, then X11 tools.
const AUTO: [ScreenshotTool; 6] = [
    ScreenshotTool::Grim,
    ScreenshotTool::Spectacle,
    ScreenshotTool::GnomeScreenshot,
    ScreenshotTool::Maim,
    ScreenshotTool::Scrot,
    ScreenshotTool::Import,
];

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

/// The tool a capture would use, if one is installed.
pub fn resolve(setting: ScreenshotTool) -> Option<ScreenshotTool> {
    match setting {
        ScreenshotTool::Auto => AUTO.into_iter().find(|t| which(program(*t)).is_some()),
        t => which(program(t)).map(|_| t),
    }
}

/// A file only this capture uses, in the app's own cache directory (not a guessable name in /tmp).
fn private_file() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = config::dirs().cache.join("capture");
    let _ = fs::create_dir_all(&dir);
    dir.join(format!("shot-{}-{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

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
        .and_then(|_| fs::read(&file).with_context(|| format!("{} produced no file", program(tool))))
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
