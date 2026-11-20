//! Outgoing HTTP: one shared client, a global off switch, and errors that keep the status code.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::config::USER_AGENT;

static OFFLINE: AtomicBool = AtomicBool::new(false);

/// `--offline`: no request leaves the machine; every source falls back to its cache.
pub fn set_offline(offline: bool) {
    OFFLINE.store(offline, Ordering::Relaxed);
}

pub fn offline() -> bool {
    OFFLINE.load(Ordering::Relaxed)
}

/// A response with a non-2xx status.
#[derive(Debug)]
pub struct HttpStatus(pub u16);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

#[derive(Debug)]
struct Offline;

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("offline mode")
    }
}

impl std::error::Error for Offline {}

/// Default response cap: generous enough for WFCD's ~9 MB relic list and game exports, tighter
/// than reading unbounded responses.
pub const DEFAULT_LIMIT: u64 = 32 * 1024 * 1024;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .https_only(true)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}

pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

/// GET with extra headers. Any status comes back as a `Response`; only transport failures error.
pub fn get(url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Response> {
    get_with(url, headers, timeout, DEFAULT_LIMIT, true)
}

/// GET with explicit body-size limit and redirect policy.
pub fn get_with(
    url: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
    max_bytes: u64,
    follow_redirects: bool,
) -> Result<Response> {
    if offline() {
        return Err(Offline.into());
    }
    let mut cfg = agent().get(url).config().timeout_global(Some(timeout));
    if !follow_redirects {
        cfg = cfg.max_redirects(0);
    }
    let mut req = cfg.build();
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.call()?;
    let status = resp.status().as_u16();
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let body = resp.body_mut().with_config().limit(max_bytes).read_to_vec()?;
    Ok(Response { status, etag, body })
}

/// GET a JSON document; a non-2xx status is an `HttpStatus` error.
pub fn get_json(url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Value> {
    let mut all = vec![("Accept", "application/json")];
    all.extend_from_slice(headers);
    let resp = get(url, &all, timeout)?;
    if !(200..300).contains(&resp.status) {
        return Err(HttpStatus(resp.status).into());
    }
    Ok(serde_json::from_slice(&resp.body)?)
}

/// Whether `uri` is a plain `http://` or `https://` web URL safe to hand to the OS browser
/// launcher: rejects `file://`, UNC paths, custom URI handlers, whitespace and control bytes.
pub fn is_http_url(uri: &str) -> bool {
    let Some(rest) = uri.strip_prefix("https://").or_else(|| uri.strip_prefix("http://")) else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !uri.chars().any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_validation_allows_only_http_and_https() {
        assert!(is_http_url("https://warframe.market/items/ash_prime_set"));
        assert!(is_http_url("http://127.0.0.1:8642/"));
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("file://C:/Windows/System32/calc.exe"));
        assert!(!is_http_url("\\\\server\\share\\payload.exe"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("https://"));
        assert!(!is_http_url("https://warframe.market/foo bar"));
        assert!(!is_http_url("https://warframe.market/\nfoo"));
    }
}
