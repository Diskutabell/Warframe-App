//! Desktop notifications: the optional reward summary, watched fissures, Baro's arrival and finished
//! foundry builds. Everything here is off unless switched on in Settings.

use std::collections::HashSet;

#[cfg(unix)]
use crate::compositor::which;
use crate::{clock, now, world};

/// Arguments passed to `notify-send`: `--` keeps titles or bodies beginning with `-` from being
/// parsed as options.
#[cfg(unix)]
fn notify_send_args<'a>(title: &'a str, body: &'a str) -> [&'a str; 7] {
    ["-a", "warframeproject", "-t", "15000", "--", title, body]
}

/// Shows a desktop notification through `notify-send`, if it is installed.
#[cfg(unix)]
pub fn send(title: &str, body: &str) {
    if which("notify-send").is_none() {
        return;
    }
    let (title, body) = (title.to_string(), body.to_string());
    // Wait on a background thread so notify-send is always reaped instead of leaking zombies,
    // without blocking the caller if the notification daemon is slow.
    let _ = std::thread::Builder::new().name("notify".into()).spawn(move || {
        let mut cmd = std::process::Command::new("notify-send");
        cmd.args(notify_send_args(&title, &body));
        let _ = crate::process::run(&mut cmd, None, std::time::Duration::from_secs(5));
    });
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
    /// Announces watched fissures that opened since the last look. `raw` is the worldstate;
    /// `err` says the copy is stale (then fissures it lacks are not forgotten yet).
    pub fn fissures(&mut self, raw: Option<&serde_json::Value>, err: Option<&str>, watch: &[String]) {
        if watch.is_empty() {
            return;
        }
        let fissures = world::fissures(raw);
        if err.is_none() && raw.is_some() {
            let active: HashSet<&str> = fissures.iter().filter_map(|f| f.id.as_deref()).collect();
            self.fissures.retain(|id| active.contains(id.as_str()));
        }
        for f in fissures {
            let id = f.id.clone().unwrap_or_default();
            if id.is_empty() || !watch.contains(&world::watch_key(&f)) || !self.fissures.insert(id) {
                continue;
            }
            let kind = if f.storm {
                " (Void Storm)"
            } else if f.hard {
                " (Steel Path)"
            } else {
                ""
            };
            let left = f.expiry.as_deref().and_then(minutes_left).map_or(String::new(), |m| format!(" · {m} min left"));
            let or_unknown = |v: &Option<String>| v.clone().unwrap_or_else(|| "?".into());
            send(
                &format!("{} {} fissure{kind}", or_unknown(&f.tier), or_unknown(&f.mission)),
                &format!("{} · {}{left}", or_unknown(&f.node), or_unknown(&f.enemy)),
            );
        }
    }

    pub fn baro(&mut self, baro: &world::Baro) {
        if !baro.active {
            return;
        }
        let visit = baro.activation.clone().unwrap_or_default();
        if self.baro.as_ref() == Some(&visit) {
            return;
        }
        self.baro = Some(visit);
        let hours =
            baro.expiry.as_deref().and_then(minutes_left).map_or(String::new(), |m| format!(" for {} h", m / 60));
        send("Baro Ki'Teer has arrived", &format!("{}{hours}", baro.location.as_deref().unwrap_or("A relay")));
    }

    pub fn foundry(&mut self, done: &[Build]) {
        let active: HashSet<String> = done.iter().map(|(uid, _, at)| format!("{uid}@{at}")).collect();
        self.foundry.retain(|k| active.contains(k));
        for (uid, name, at) in done {
            if *at <= now() && self.foundry.insert(format!("{uid}@{at}")) {
                send("Foundry", &format!("{name} is ready to claim"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn notify_send_uses_option_terminator() {
        assert_eq!(
            notify_send_args("-urgent", "-body"),
            ["-a", "warframeproject", "-t", "15000", "--", "-urgent", "-body"]
        );
    }

    #[test]
    fn foundry_prunes_claimed_builds() {
        let mut n = Notifier::default();
        let future = now() + 3600.0;
        n.foundry.insert("old@1".into());
        n.foundry.insert(format!("keep@{future}"));
        n.foundry(&[("keep".into(), "Ash".into(), future)]);
        assert!(!n.foundry.contains("old@1"));
        assert!(n.foundry.contains(&format!("keep@{future}")));
    }
}
