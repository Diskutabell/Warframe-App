//! EE.log: where it is, what the game last wrote, and the events its lines mean, handed to the
//! engine in the order they were written.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::lock;
use super::settings::SettingsStore;
use crate::eelog::{self, LogEvent, LogStatus, LogWatcher};

pub struct LogService {
    settings: Arc<SettingsStore>,
    /// Set once the watcher runs.
    status: Mutex<Option<Arc<Mutex<LogStatus>>>>,
}

impl LogService {
    pub fn new(settings: Arc<SettingsStore>) -> LogService {
        LogService { settings, status: Mutex::new(None) }
    }

    /// The log: the one set in Settings, else the one found in Steam's, Lutris' or Heroic's
    /// prefixes (searched in the background).
    pub fn path(&self) -> Option<PathBuf> {
        let configured = self.settings.get().ee_log_path;
        if configured.is_empty() { eelog::find_ee_log() } else { Some(PathBuf::from(configured)) }
    }

    /// When the log was last written, epoch seconds.
    pub fn modified(&self) -> Option<f64> {
        self.path()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|t| t.as_secs_f64())
    }

    /// The game's interface scale, from EE.cfg beside the log.
    pub fn ui_scale(&self) -> Option<f64> {
        self.path().and_then(|log| eelog::ui_scale(&log))
    }

    fn status(&self) -> Option<Arc<Mutex<LogStatus>>> {
        lock(&self.status).clone()
    }

    /// Whether the game wrote to its log in the last 10 minutes.
    pub fn game_active(&self) -> bool {
        self.status().is_some_and(|s| lock(&s).game_active())
    }

    /// The last `n` lines worth showing, oldest first.
    pub fn recent(&self, n: usize) -> Vec<String> {
        match self.status() {
            Some(s) => {
                let s = lock(&s);
                let skip = s.recent.len().saturating_sub(n);
                s.recent.iter().skip(skip).cloned().collect()
            }
            None => Vec::new(),
        }
    }

    /// Follows the log until `stop`; its events go to `events` in the order they were written.
    pub fn start(self: &Arc<Self>, events: mpsc::UnboundedSender<LogEvent>, stop: CancellationToken) {
        let me = Arc::clone(self);
        let status = LogWatcher::spawn(
            Box::new(move || me.path()),
            Box::new(move |ev| {
                let _ = events.send(ev);
            }),
            stop,
        );
        *lock(&self.status) = Some(status);
    }
}
