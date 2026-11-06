//! Desktop notifications: the optional reward summary, watched fissures, Baro's arrival and finished
//! foundry builds. Everything here is off unless switched on in Settings.

use std::collections::HashSet;

#[cfg(unix)]
use crate::compositor::which;
use crate::{clock, now, world};

/// Shows a desktop notification through `notify-send`, if it is installed.
#[cfg(unix)]
pub fn send(title: &str, body: &str) {
    if which("notify-send").is_some() {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "warframeproject", "-t", "15000", title, body])
            .stdin(std::process::Stdio::null())
            .spawn();
    }
}

/// Shows a Windows toast notification.
#[cfg(windows)]
pub fn send(title: &str, body: &str) {
    use tauri_winrt_notification::{Duration, Toast};
    // Toasts need the id of an installed app to show under; PowerShell's is always there.
    let toast = Toast::new(Toast::POWERSHELL_APP_ID).title(title).text1(body).duration(Duration::Long);
    if let Err(e) = toast.show() {
        eprintln!("could not show a notification: {e}");
    }
}

fn minutes_left(expiry: &str) -> Option<i64> {
    clock::parse_iso(expiry).map(|e| ((e - now()) / 60.0).round() as i64)
}

/// Remembers what was already announced, so each fissure, Baro visit and build is announced once.
#[derive(Default)]
pub struct Notifier {
    fissures: HashSet<String>,
    baro: Option<String>,
    foundry: HashSet<String>,
}

/// A foundry build that is done: (recipe uid, display name, finished at).
pub type Build = (String, String, f64);

impl Notifier {
    pub fn fissures(&mut self, platform: crate::config::Platform, watch: &[String]) {
        if watch.is_empty() {
            return;
        }
        let (fissures, _) = world::fissures(platform);
        for f in fissures {
            let id = f["id"].as_str().unwrap_or_default().to_string();
            if id.is_empty() || !watch.contains(&world::watch_key(&f)) || !self.fissures.insert(id) {
                continue;
            }
            let kind = if f["storm"] == true {
                " (Void Storm)"
            } else if f["hard"] == true {
                " (Steel Path)"
            } else {
                ""
            };
            let left =
                f["expiry"].as_str().and_then(minutes_left).map_or(String::new(), |m| format!(" · {m} min left"));
            send(
                &format!(
                    "{} {} fissure{kind}",
                    f["tier"].as_str().unwrap_or("?"),
                    f["mission"].as_str().unwrap_or("?")
                ),
                &format!("{} · {}{left}", f["node"].as_str().unwrap_or("?"), f["enemy"].as_str().unwrap_or("?")),
            );
        }
    }

    pub fn baro(&mut self, baro: &serde_json::Value) {
        if baro["active"] != true {
            return;
        }
        let visit = baro["activation"].as_str().unwrap_or_default().to_string();
        if self.baro.as_ref() == Some(&visit) {
            return;
        }
        self.baro = Some(visit);
        let hours =
            baro["expiry"].as_str().and_then(minutes_left).map_or(String::new(), |m| format!(" for {} h", m / 60));
        send("Baro Ki'Teer has arrived", &format!("{}{hours}", baro["location"].as_str().unwrap_or("A relay")));
    }

    pub fn foundry(&mut self, done: &[Build]) {
        for (uid, name, at) in done {
            if *at <= now() && self.foundry.insert(format!("{uid}@{at}")) {
                send("Foundry", &format!("{name} is ready to claim"));
            }
        }
    }
}
