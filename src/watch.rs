//! Wakeups when a file changes: the system's file notifications (inotify on Linux,
//! ReadDirectoryChangesW on Windows) on the file's folder, so a file that is replaced or created
//! anew is seen as well as one that is written to.
//!
//! Notifications can be missed (network drives, a folder that does not exist yet), so the
//! callers still look now and then on their own; with notifications that look is rare and the
//! app does no work while nothing changes.

use std::ffi::OsString;
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};

pub struct Wakeups {
    /// Kept alive for as long as the wakeups are wanted; None when the folder could not be
    /// watched.
    watcher: Option<notify::RecommendedWatcher>,
    rx: Receiver<()>,
}

impl Wakeups {
    /// Wakeups for changes to `file`.
    pub fn new(file: &Path) -> Wakeups {
        let (tx, rx) = channel();
        let name: Option<OsString> = file.file_name().map(|n| n.to_os_string());
        let watcher = file.parent().and_then(|dir| {
            let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let wanted = match &res {
                    // The folder's other files (EE.cfg, older logs) are not of interest.
                    Ok(ev) => {
                        !matches!(ev.kind, EventKind::Access(_))
                            && (ev.paths.is_empty()
                                || ev.paths.iter().any(|p| p.file_name().map(|n| n.to_os_string()) == name))
                    }
                    // An error (the event queue overflowed, say): look, to be safe.
                    Err(_) => true,
                };
                if wanted {
                    let _ = tx.send(());
                }
            })
            .ok()?;
            watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;
            Some(watcher)
        });
        Wakeups { watcher, rx }
    }

    /// Whether the system reports changes (else only the callers' own looks find them).
    pub fn watching(&self) -> bool {
        self.watcher.is_some()
    }

    /// Waits up to `timeout` for a change; true when there was one. A burst of changes (a game
    /// writing its log line by line) is taken as one: after the first, `settle` more is waited
    /// for the rest of it.
    pub fn wait(&self, timeout: Duration, settle: Duration) -> bool {
        match self.rx.recv_timeout(timeout) {
            Ok(()) => {
                std::thread::sleep(settle);
                while self.rx.try_recv().is_ok() {}
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            // Without a sender (no watcher) a wait is a plain pause.
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(timeout);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_to_the_file_wake_and_other_files_do_not() {
        let dir = std::env::temp_dir().join(format!("wfp-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("EE.log");
        std::fs::write(&file, "start\n").unwrap();
        let wakeups = Wakeups::new(&file);
        assert!(wakeups.watching());
        std::fs::write(dir.join("EE.cfg"), "x").unwrap();
        assert!(!wakeups.wait(Duration::from_millis(300), Duration::ZERO), "another file is ignored");
        std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(b"more\n").unwrap();
        assert!(wakeups.wait(Duration::from_secs(5), Duration::from_millis(10)), "a write wakes");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
