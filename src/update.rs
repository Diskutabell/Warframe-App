//! Whether a newer version is out: one look at the project's latest GitHub release when the app
//! starts. The window then says so in a popup; nothing is downloaded or installed.

use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use crate::net;

/// The newest published release. GitHub never counts drafts or pre-releases as "latest".
const LATEST: &str = "https://api.github.com/repos/Diskutabell/Warframe-App/releases/latest";
const REPO_URL: &str = "https://github.com/Diskutabell/Warframe-App/";

/// "v0.10.2-rc1" -> ([0, 10, 2], Some("rc1")). Build metadata after '+' is ignored.
fn parse_version(version: &str) -> Option<(Vec<u64>, Option<&str>)> {
    let trimmed = version.trim().trim_start_matches(['v', 'V']);
    let without_build = trimmed.split('+').next()?;
    let (core, pre) = match without_build.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (without_build, None),
    };
    let nums: Vec<u64> = core.split('.').map(|n| n.parse().ok()).collect::<Option<_>>()?;
    (!nums.is_empty()).then_some((nums, pre))
}

/// Whether `latest` is a later version than `running`; false when either cannot be read.
pub fn is_newer(latest: &str, running: &str) -> bool {
    let (Some((l_nums, l_pre)), Some((r_nums, r_pre))) = (parse_version(latest), parse_version(running)) else {
        return false;
    };
    let len = l_nums.len().max(r_nums.len());
    for i in 0..len {
        let l = l_nums.get(i).copied().unwrap_or(0);
        let r = r_nums.get(i).copied().unwrap_or(0);
        if l != r {
            return l > r;
        }
    }
    // Same numeric version: a final release is newer than a pre-release.
    l_pre.is_none() && r_pre.is_some()
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
    let url = release["html_url"]
        .as_str()
        .filter(|u| u.starts_with(REPO_URL) && !u.chars().any(|c| c.is_control() || c.is_whitespace()));
    is_newer(tag, running).then(|| {
        json!({
            "version": tag.trim_start_matches(['v', 'V']), "current": running,
            "name": release["name"], "url": url, "notes": release["body"],
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
        assert!(!is_newer("0.9.0", "0.9"), "shorter versions zero-pad when comparing");
        assert!(!is_newer("0.9", "0.9.0"));
        assert!(!is_newer("0.7.9", "0.8.0"), "an older release is no update");
        assert!(is_newer("0.9.0", "0.9.0-rc1"), "final release is newer than pre-release");
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

        let untrusted = json!({"tag_name": "v0.9.0", "html_url": "https://evil.example/releases/v0.9.0"});
        assert!(newer_release(&untrusted, "0.8.0").unwrap()["url"].is_null());
    }
}
