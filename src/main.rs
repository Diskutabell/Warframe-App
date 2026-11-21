//! warframeproject: a Warframe companion for Linux and Windows.
//!
//! The engine (data sources, OCR, prices) serves a loopback-only HTTP API; the desktop shell shows
//! the web UI in a native window and draws the in-game overlays.

// A GUI program on Windows: no console window opens when it is started from the desktop.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod accounts;
mod api;
mod app;
mod arbitration;
mod art;
mod bundle;
mod bus;
mod clock;
mod config;
mod db;
mod difflib;
mod drops;
mod eelog;
mod gamedata;
mod history;
mod inventory;
mod market;
mod memscan;
mod net;
mod notifier;
mod ocr;
mod process;
mod relics;
mod rivens;
mod services;
mod sources;
mod standing;
mod tasks;
mod tray_icon;
mod update;
mod views;
mod watch;
mod world;

// The parts that talk to the desktop differ per system. On Windows the same module names load
// the files in `src/windows/`, so the rest of the code calls `compositor::game_rect()` or
// `desktop::run()` without caring which system it runs on.
#[cfg(unix)]
mod compositor;
#[cfg(unix)]
mod ipc;
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
    /// Show or hide the window of the running app (bind this to a hotkey).
    Toggle,
    /// Close the running app.
    Quit,
    /// Read a saved screenshot (the app keeps the last ones in its cache's frames folder).
    Replay {
        /// Screenshots (PNG or PPM) of the whole game screen. Several give one line each.
        #[arg(required = true)]
        frames: Vec<PathBuf>,
        /// Read it as a riven card instead of a reward screen.
        #[arg(long)]
        riven: bool,
        /// The game's interface scale in percent when the frame was taken (default: the one in
        /// the game's EE.cfg now, then any).
        #[arg(long)]
        scale: Option<f64>,
    },
    /// Download every game data source, check it, and write the prepared bundle and its
    /// manifest into a folder (what CI publishes on the `catalog` release).
    BuildCatalog {
        /// Where the bundle and manifest.json go.
        #[arg(long, default_value = "catalog")]
        out: PathBuf,
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
        Command::Toggle => local(settings.port, "window/toggle", true, Duration::from_secs(5))
            .map(|_| ())
            .with_context(|| format!("warframeproject is not running (port {})", settings.port)),
        Command::Quit => local(settings.port, "quit", true, Duration::from_secs(5))
            .map(|_| println!("warframeproject is closing."))
            .with_context(|| format!("warframeproject is not running (port {})", settings.port)),
        Command::Replay { frames, riven, scale } => {
            net::set_offline(cli.offline);
            replay(&frames, riven, scale)
        }
        Command::BuildCatalog { out } => {
            net::set_offline(cli.offline);
            build_catalog(&out)
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

/// Calls the running app's own API (`trigger`, `quit`, the single-instance check).
fn local(port: u16, path: &str, post: bool, timeout: Duration) -> Result<Value> {
    net::local::call(port, path, post, timeout)
}

/// Whether this app (not some other program on the port) answers there: its settings carry keys
/// nothing else would.
fn running(port: u16) -> bool {
    local(port, "settings", false, Duration::from_secs(2))
        .is_ok_and(|v| v.get("hotkey_rewards").is_some() && v.get("ee_log_path").is_some())
}

/// How long the engine gets, once the window is closed, to finish requests and background work
/// and to write what is not on disk yet.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

fn run(settings: config::Settings, browser: bool, headless: bool, offline: bool) -> Result<()> {
    let desktop = !browser && !headless;
    // Before anything starts a thread: the display backend is chosen through the environment,
    // which may only be changed while the process is single-threaded.
    let backend = desktop.then(|| desktop::prepare_environment(&settings));
    net::set_offline(offline);

    let port = settings.port;
    if running(port) {
        // One instance: bring the existing window forward instead.
        let _ = local(port, "window/show", true, Duration::from_secs(5));
        println!("warframeproject is already running.");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("engine")
        .build()
        .context("the engine's runtime could not start")?;
    let listener = runtime.block_on(tokio::net::TcpListener::bind(("127.0.0.1", port))).map_err(|e| {
        UserError::new(format!(
            "Port {port} is in use by another program ({e}). Change \"port\" in {} or pass --port.",
            config::settings_file().display()
        ))
    })?;
    // `--port 0` binds any free port: from here on the port is the one bound.
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    // The engine's background tasks are spawned onto the runtime from here on, also from the
    // desktop shell's callbacks on this thread.
    let _runtime = runtime.enter();
    let state = app::App::new(settings, port)?;
    state.start();
    let server = runtime.spawn(api::serve(listener, Arc::clone(&state), state.stop.clone()));
    runtime.spawn(quit_on_signal(Arc::clone(&state)));
    #[cfg(windows)]
    {
        if let Err(e) = state.install_hotkeys() {
            state.accounts.event(&format!("Hotkeys not registered: {e}"), "warn");
        }
        let me = Arc::clone(&state);
        std::thread::Builder::new().name("unpack-ocr".into()).spawn(move || match compositor::unpack_tesseract() {
            Ok(()) => me.tools_changed(),
            Err(e) => me.accounts.event(&format!("The built-in OCR could not be unpacked: {e:#}"), "warn"),
        })?;
    }
    let url = format!("http://127.0.0.1:{port}/");
    println!("warframeproject {} engine on {url}", env!("CARGO_PKG_VERSION"));

    let result = match backend {
        Some(backend) => desktop::run(Arc::clone(&state), &url, backend),
        None => {
            if browser {
                #[cfg(unix)]
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                #[cfg(windows)]
                desktop::open_url(&url);
            }
            // Until `/api/quit`, Ctrl+C or SIGTERM.
            runtime.block_on(state.stop.cancelled());
            Ok(())
        }
    };
    // The window is gone: stop the background work, let running requests finish, and write what
    // is not on disk yet. Returning from `main` then ends whatever is left.
    state.stop.cancel();
    // Scans looking for cards give up now, so the requests waiting on them end too.
    state.scans.stop_scans();
    runtime.block_on(async {
        if tokio::time::timeout(SHUTDOWN_GRACE, server).await.is_err() {
            eprintln!("the server did not stop in time; quitting anyway");
        }
        state.shutdown(SHUTDOWN_GRACE).await;
    });
    drop(_runtime);
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

/// Ctrl+C in the terminal, or SIGTERM from the system, quits like the window's Quit button.
async fn quit_on_signal(state: Arc<app::App>) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
            _ = state.stop.cancelled() => return,
        }
    }
    #[cfg(windows)]
    tokio::select! {
        r = tokio::signal::ctrl_c() => if r.is_err() { return },
        _ = state.stop.cancelled() => return,
    }
    state.quit();
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
        println!(
            "  {:<9} {:<26} {:<3} ({:+.2}%)",
            g["printed"].as_str().unwrap_or("").replace('\u{2212}', "-"),
            g["stat"].as_str().unwrap_or(""),
            g["grade"].as_str().unwrap_or(""),
            g["deviation"].as_f64().unwrap_or(0.0)
        );
    }
    Ok(())
}

/// Reads a saved frame offline: the regression check for the OCR.
fn build_catalog(out: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let prepared = runtime.block_on(gamedata::prepare_from_sources(true))?;
    let catalog = gamedata::build_prepared(&prepared);
    eprintln!("prepared: {}", serde_json::Value::Object(bundle::counts(&catalog)));
    let failed = bundle::check(&prepared, &catalog);
    if !failed.is_empty() {
        for f in &failed {
            eprintln!("check failed: {f}");
        }
        anyhow::bail!("the game data failed {} check(s); nothing was written", failed.len());
    }
    let (packed, manifest) = bundle::encode(&prepared, bundle::counts(&catalog))?;
    std::fs::create_dir_all(out)?;
    std::fs::write(out.join(&manifest.file), &packed)?;
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest)?)?;
    println!("{} ({} bytes): {}", manifest.file, manifest.size, serde_json::Value::Object(manifest.counts.clone()));
    Ok(())
}

fn replay(frames: &[PathBuf], riven: bool, scale: Option<f64>) -> Result<()> {
    #[cfg(windows)]
    compositor::unpack_tesseract()?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let catalog =
        runtime.block_on(gamedata::load(false)).context("the game data is needed to know what to look for")?;
    // The scale the frames were taken at: as given, else the game's own now, else any.
    let scales: Vec<f64> = match scale {
        Some(percent) => vec![percent / 100.0],
        None => eelog::find_ee_log_now().and_then(|log| eelog::ui_scale(&log)).into_iter().collect(),
    };
    if riven {
        for frame in frames {
            replay_riven(&catalog, frame, &scales)?;
        }
        return Ok(());
    }
    let names = ocr::matcher::Names::new(&catalog.reward_names());
    if let [frame] = frames {
        return replay_rewards(frame, &names, &scales);
    }
    // A folder of frames: one line each, and only the cards (no search for names without them).
    for frame in frames {
        let started = Instant::now();
        let image = ocr::screenshot::load(frame)?;
        let file = frame.file_name().map_or_else(|| frame.display().to_string(), |f| f.to_string_lossy().into_owned());
        let layout = ocr::reward::find_layout(&image, &scales).or_else(|| ocr::reward::find_layout(&image, &[]));
        match layout {
            Some(layout) => {
                let read = ocr::reward::read(&image, &layout, &names)?;
                let cards: Vec<String> = read
                    .names
                    .iter()
                    .map(|m| m.as_ref().map_or("?".to_string(), |m| format!("{} {:.2}", m.name, m.score)))
                    .collect();
                println!(
                    "{file}: {} card(s) at {:.0}%: {} ({:.2} s)",
                    layout.cards,
                    layout.ui_scale * 100.0,
                    cards.join(" | "),
                    started.elapsed().as_secs_f64()
                );
            }
            None => println!("{file}: no cards ({:.2} s)", started.elapsed().as_secs_f64()),
        }
    }
    Ok(())
}

fn replay_riven(catalog: &gamedata::Catalog, frame: &std::path::Path, scales: &[f64]) -> Result<()> {
    let image = ocr::screenshot::load(frame)?;
    let started = Instant::now();
    let (read, text) = ocr::read_riven(&image, &catalog.riven_weapons(), scales)?;
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
        // As the card prints it: the value keeps the card's sign (Recoil's good rolls are "-").
        println!("  {}{} {}{grade}", if s.value < 0.0 { "-" } else { "+" }, s.value.abs(), s.stat);
    }
    eprintln!("--- text\n{text}");
    eprintln!("--- read in {:.2} s", started.elapsed().as_secs_f64());
    Ok(())
}

fn replay_rewards(frame: &std::path::Path, names: &ocr::matcher::Names, scales: &[f64]) -> Result<()> {
    let image = ocr::screenshot::load(frame)?;
    let started = Instant::now();
    let layout = ocr::reward::find_layout(&image, scales).or_else(|| ocr::reward::find_layout(&image, &[]));
    if let Some(layout) = layout {
        println!("{} card(s) at interface scale {:.0}%", layout.cards, layout.ui_scale * 100.0);
        let read = ocr::reward::read(&image, &layout, names)?;
        for (k, m) in read.names.iter().enumerate() {
            match m {
                Some(m) => println!("  {}. {:<40} {:.3}", k + 1, m.name, m.score),
                None => println!("  {}. (not recognised)", k + 1),
            }
        }
        eprintln!("--- text\n{}", read.text);
    } else {
        // What the hotkey falls back to; the automatic scan reports nothing here.
        println!("no reward cards on this frame; names anywhere in the middle:");
        let (matches, text) = ocr::read_rewards(&image, names)?;
        for m in &matches {
            println!("  {:<40} {:.3}", m.name, m.score);
        }
        if matches.is_empty() {
            println!("  (none)");
        }
        eprintln!("--- text\n{text}");
    }
    eprintln!("--- read in {:.2} s", started.elapsed().as_secs_f64());
    Ok(())
}
