//! Whether a newer version is out: one look at the project's latest GitHub release when the app
//! starts. The window then says so in a popup; nothing is downloaded or installed.

use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use crate::net;

/// The newest published release. GitHub never counts drafts or pre-releases as "latest".
const LATEST: &str = "https://api.github.com/repos/Diskutabell/Warframe-App/releases/latest";

/// "v0.10.2" -> [0, 10, 2]. Whatever follows a '-' or '+' is not compared.
fn numbers(version: &str) -> Option<Vec<u64>> {
    let core = version.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    core.split('.').map(|n| n.parse().ok()).collect()
}

/// Whether `latest` is a later version than `running`; false when either cannot be read.
pub fn is_newer(latest: &str, running: &str) -> bool {
    match (numbers(latest), numbers(running)) {
        // Vectors compare number by number: [0, 10, 0] > [0, 9, 3].
        (Some(latest), Some(running)) => latest > running,
        _ => false,
    }
}

/// The latest release, when it is newer than this build: `{version, current, name, url, notes,
/// published}`. `Ok(None)` when this build is up to date.
pub fn check() -> Result<Option<Value>> {
    let release = net::get_json(LATEST, &[], Duration::from_secs(20))?;
    Ok(newer_release(&release, env!("CARGO_PKG_VERSION")))
}

/// What the popup needs from GitHub's answer, if its release is newer than `running`.
fn newer_release(release: &Value, running: &str) -> Option<Value> {
    let tag = release["tag_name"].as_str().unwrap_or("");
    is_newer(tag, running).then(|| {
        json!({
            "version": tag.trim_start_matches(['v', 'V']), "current": running,
            "name": release["name"], "url": release["html_url"], "notes": release["body"],
            "published": release["published_at"],
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_number_by_number() {
        assert!(is_newer("v0.9.0", "0.8.0"));
        assert!(is_newer("0.10.0", "0.9.3"), "10 is more than 9, though it sorts first as text");
        assert!(is_newer("v1.0", "0.99.99"));
        assert!(is_newer("0.8.1", "0.8.0"));
        assert!(!is_newer("v0.8.0", "0.8.0"));
        assert!(!is_newer("0.7.9", "0.8.0"), "an older release is no update");
        assert!(!is_newer("v0.9.0-rc1", "0.9.0"));
        assert!(!is_newer("", "0.8.0"));
        assert!(!is_newer("nightly", "0.8.0"));
    }

    #[test]
    fn a_newer_release_brings_its_page_and_notes() {
        // The fields GitHub's /releases/latest answers with, shortened.
        let release = json!({"tag_name": "v0.9.0", "name": "warframeproject 0.9.0",
            "html_url": "https://github.com/Diskutabell/Warframe-App/releases/tag/v0.9.0",
            "body": "### Added\n\n- **Arbitrations**", "published_at": "2026-11-10T12:00:00Z", "assets": []});
        let found = newer_release(&release, "0.8.0").unwrap();
        assert_eq!((found["version"].as_str(), found["current"].as_str()), (Some("0.9.0"), Some("0.8.0")));
        assert_eq!(found["url"], release["html_url"]);
        assert_eq!(found["notes"], release["body"]);
        assert_eq!(newer_release(&release, "0.9.0"), None);
        assert_eq!(newer_release(&json!({"message": "API rate limit exceeded"}), "0.8.0"), None);
    }
}
