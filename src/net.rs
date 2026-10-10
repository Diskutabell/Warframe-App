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

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
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
    if offline() {
        return Err(Offline.into());
    }
    let mut req = agent().get(url).config().timeout_global(Some(timeout)).build();
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.call()?;
    let status = resp.status().as_u16();
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned);
    // WFCD's relic list alone is ~9 MB, and ureq's default cap is 10 MB.
    let body = resp.body_mut().with_config().limit(128 * 1024 * 1024).read_to_vec()?;
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
