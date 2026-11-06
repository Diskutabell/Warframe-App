//! warframeproject: a Warframe companion for Linux and Windows.
//!
//! The engine (data sources, OCR, prices) serves a loopback-only HTTP API; the desktop shell shows
//! the web UI in a native window and draws the in-game overlays.

// A GUI program on Windows: no console window opens when it is started from the desktop.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod accounts;
mod art;
mod clock;
mod config;
mod difflib;
mod eelog;
mod gamedata;
mod history;
mod market;
mod memscan;
mod net;
mod notifier;
mod ocr;
mod process;
mod relics;
mod rivens;
mod server;
mod sources;
mod standing;
mod state;
mod views;
mod world;

// The parts that talk to the desktop differ per system. On Windows the same module names load
// the files in `src/windows/`, so the rest of the code calls `compositor::game_rect()` or
// `desktop::run()` without caring which system it runs on.
#[cfg(unix)]
mod compositor;
#[cfg(windows)]
#[path = "windows/compositor.rs"]
mod compositor;
#[cfg(unix)]
mod desktop;
#[cfg(windows)]
#[path = "windows/desktop.rs"]
mod desktop;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;

/// An error the user can act on: shown as-is, answered with HTTP 400 rather than 500.
#[derive(Debug)]
pub struct UserError(String);

impl UserError {
    pub fn new(msg: impl Into<String>) -> UserError {
        UserError(msg.into())
    }
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UserError {}

pub use clock::now;

#[derive(Parser)]
#[command(name = "warframeproject", version, about = "Warframe companion for Linux and Windows")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Port of the local engine (default: the one in settings, 8642).
    #[arg(long, global = true)]
    port: Option<u16>,
    /// Debugging: serve the UI to your web browser instead of opening a window.
    #[arg(long)]
    browser: bool,
    /// Run the engine only, without a window.
    #[arg(long)]
    headless: bool,
    /// Never contact any server; every source uses its cache.
    #[arg(long, global = true)]
    offline: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Open the app (the default).
    Run,
    /// Scan the relic reward screen now, in the running app (bind this to a hotkey).
    Trigger,
    /// Grade the riven on screen now, in the running app (bind this to a hotkey).
    Riven,
    /// Close the running app.
    Quit,
    /// Read a saved screenshot (the app keeps the last ones in its cache's frames folder).
    Replay {
        /// A screenshot (PNG or PPM) of the whole game screen.
        frame: PathBuf,
        /// Read it as a riven card instead of a reward screen.
        #[arg(long)]
        riven: bool,
    },
}

fn main() -> ExitCode {
    #[cfg(windows)]
    desktop::process_setup();
    let cli = Cli::parse();
    #[cfg(windows)]
    let opens_window = matches!(cli.command, None | Some(Command::Run)) && !cli.headless && !cli.browser;
    let mut settings = config::Settings::load();
    if let Some(port) = cli.port {
        settings.port = port;
    }
    let result = match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(settings, cli.browser, cli.headless, cli.offline),
        Command::Trigger => trigger(settings.port),
        Command::Riven => riven(settings.port),
        Command::Quit => local(settings.port, "quit", true, Duration::from_secs(5))
            .map(|_| println!("warframeproject is closing."))
            .with_context(|| format!("warframeproject is not running (port {})", settings.port)),
        Command::Replay { frame, riven } => {
            net::set_offline(cli.offline);
            replay(&frame, riven)
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            #[cfg(windows)]
            if opens_window {
                desktop::error_box(&format!("{e:#}"));
            }
            ExitCode::FAILURE
        }
    }
}

fn local(port: u16, path: &str, post: bool, timeout: Duration) -> Result<Value> {
    let url = format!("http://127.0.0.1:{port}/api/{path}");
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into();
    let mut resp = if post {
        agent.post(&url).header("Content-Type", "application/json").send("{}")?
    } else {
        agent.get(&url).call()?
    };
    Ok(serde_json::from_slice(&resp.body_mut().read_to_vec()?)?)
}

fn running(port: u16) -> bool {
    local(port, "settings", false, Duration::from_secs(2)).is_ok_and(|v| v.is_object())
}

fn run(settings: config::Settings, browser: bool, headless: bool, offline: bool) -> Result<()> {
    let desktop = !browser && !headless;
    // Before anything starts a thread: the display backend is chosen through the environment.
    let backend = desktop.then(|| desktop::prepare_environment(&settings));
    net::set_offline(offline);

    let port = settings.port;
    if running(port) {
        // One instance: bring the existing window forward instead.
        let _ = local(port, "window/show", true, Duration::from_secs(5));
        println!("warframeproject is already running.");
        return Ok(());
    }
    let server = server::bind(port).map_err(|e| {
        UserError::new(format!(
            "Port {port} is in use by another program ({e}). Change \"port\" in {} or pass --port.",
            config::settings_file().display()
        ))
    })?;
    let state = state::State::new(settings);
    state.start();
    server::serve(server, Arc::clone(&state), 6);
    #[cfg(windows)]
    {
        if let Err(e) = state.install_hotkeys() {
            state.event(&format!("Hotkeys not registered: {e}"), "warn");
        }
        let me = Arc::clone(&state);
        std::thread::Builder::new().name("unpack-ocr".into()).spawn(move || {
            if let Err(e) = compositor::unpack_tesseract() {
                me.event(&format!("The built-in OCR could not be unpacked: {e:#}"), "warn");
            }
        })?;
    }
    let url = format!("http://127.0.0.1:{port}/");
    println!("warframeproject {} engine on {url}", env!("CARGO_PKG_VERSION"));

    match backend {
        Some(backend) => {
            desktop::run(state, &url, backend)?;
            // Closing the window ends the app, background threads included.
            std::process::exit(0);
        }
        None => {
            if browser {
                #[cfg(unix)]
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                #[cfg(windows)]
                desktop::open_url(&url);
            }
            loop {
                std::thread::park();
            }
        }
    }
}

fn trigger(port: u16) -> Result<()> {
    let res = local(port, "rewards/scan", true, Duration::from_secs(60))
        .with_context(|| format!("warframeproject is not running (port {port})"))?;
    for it in res["items"].as_array().into_iter().flatten() {
        let price = it["price"].as_i64().map_or("?".into(), |p| p.to_string());
        let ducats = it["ducats"].as_i64().map_or("?".into(), |d| d.to_string());
        println!("{:<40} {price:>5}p  {ducats:>4}d", it["name"].as_str().unwrap_or(""));
    }
    if let Some(e) = res["error"].as_str() {
        println!("{e}");
    }
    Ok(())
}

fn riven(port: u16) -> Result<()> {
    let res = local(port, "riven/scan", true, Duration::from_secs(60))
        .with_context(|| format!("warframeproject is not running (port {port})"))?;
    if let Some(e) = res["error"].as_str() {
        println!("{e}");
        return Ok(());
    }
    let negative = if res["negatives"].as_u64().unwrap_or(0) > 0 { " + negative" } else { "" };
    let unread = match res["unread"].as_u64().unwrap_or(0) {
        0 => String::new(),
        k => format!("  ·  {k} stat(s) not readable"),
    };
    println!(
        "{}  ·  disposition {}x  ·  {} positive(s){negative}{unread}",
        res["weapon"].as_str().unwrap_or("?"),
        res["disposition"],
        res["positives"]
    );
    for g in res["stats"].as_array().into_iter().flatten() {
        let sign = if g["negative"].as_bool().unwrap_or(false) { "-" } else { "+" };
        println!(
            "  {sign}{:<8} {:<26} {:<3} ({:+.2}%)",
            g["value"].as_f64().unwrap_or(0.0).abs(),
            g["stat"].as_str().unwrap_or(""),
            g["grade"].as_str().unwrap_or(""),
            g["deviation"].as_f64().unwrap_or(0.0)
        );
    }
    Ok(())
}

/// Reads a saved frame offline: the regression check for the OCR.
fn replay(frame: &std::path::Path, riven: bool) -> Result<()> {
    #[cfg(windows)]
    compositor::unpack_tesseract()?;
    let catalog = gamedata::load(false).context("the game data is needed to know what to look for")?;
    let image = ocr::screenshot::load(frame)?;
    let started = Instant::now();
    if riven {
        let (read, text) = ocr::read_riven(&image, &catalog.riven_weapons())?;
        println!("weapon: {}", read.weapon.as_deref().unwrap_or("(none)"));
        let item = read.weapon.as_deref().and_then(|w| catalog.item_by_name(w));
        let readings: Vec<(&str, f64, bool)> = read.stats.iter().map(|s| (s.stat, s.value, s.negative)).collect();
        let counts = item.map(|it| rivens::infer_counts(it, &readings));
        if let Some((pos, neg, unread)) = counts {
            println!("rolled {pos} positive(s) + {neg} negative; {unread} stat(s) not readable");
        }
        for s in &read.stats {
            let graded =
                item.zip(counts).and_then(|(it, (p, n, _))| rivens::grade_roll(it, s.stat, s.value, p, n, s.negative));
            let grade = graded.map_or(String::new(), |g| format!("  {} ({:+.2}%)", g.grade, g.deviation));
            println!("  {}{} {}{grade}", if s.negative { "-" } else { "+" }, s.value.abs(), s.stat);
        }
        eprintln!("--- text\n{text}");
    } else {
        let names = ocr::matcher::Names::new(&catalog.reward_names());
        let (matches, text) = ocr::read_rewards(&image, &names)?;
        for m in &matches {
            println!("{:<40} {:.3}", m.name, m.score);
        }
        if matches.is_empty() {
            println!("(no reward names recognised)");
        }
        eprintln!("--- text\n{text}");
    }
    eprintln!("--- read in {:.2} s", started.elapsed().as_secs_f64());
    Ok(())
}
