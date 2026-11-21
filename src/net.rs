//! Outgoing HTTP: one shared async client, a global off switch, and errors that keep the status
//! code.
//!
//! Every request goes out over HTTPS (TLS by rustls with the Mozilla root list, the same on Linux
//! and Windows), names the app in its User-Agent, and reads at most a given number of bytes, so a
//! misbehaving server cannot make the app hold a huge answer in memory. `--offline` stops every
//! request before it leaves the machine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::redirect::Policy;
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

/// The request was not sent: the app runs with `--offline`.
#[derive(Debug)]
pub struct Offline;

impl std::fmt::Display for Offline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("offline mode")
    }
}

impl std::error::Error for Offline {}

/// Default response cap: generous enough for WFCD's ~9 MB relic list and game exports, tighter
/// than reading unbounded responses.
pub const DEFAULT_LIMIT: u64 = 32 * 1024 * 1024;
/// Redirects followed at most, and only to HTTPS addresses.
const MAX_REDIRECTS: usize = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// TLS as the app has always done it: rustls with ring and the bundled Mozilla root list, so no
/// system library or certificate store is needed and Linux and Windows behave the same.
fn tls() -> rustls::ClientConfig {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

fn build(redirects: Policy) -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(tls())
        .user_agent(USER_AGENT)
        .https_only(true)
        .redirect(redirects)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(Duration::from_secs(30))
        .build()
        .expect("the HTTP client's fixed configuration is valid")
}

/// Which redirects a request follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redirects {
    /// Any HTTPS address, up to `MAX_REDIRECTS` hops.
    Follow,
    /// Only to addresses the item art may come from (`art::allowed`): an art host must not be able
    /// to steer the fetch anywhere else. (WFCD's image CDN answers every picture with a redirect to
    /// GitHub, which is an art source too.)
    ArtSources,
}

/// The shared client for a redirect policy.
fn client(redirects: Redirects) -> &'static reqwest::Client {
    static FOLLOWING: OnceLock<reqwest::Client> = OnceLock::new();
    static ART: OnceLock<reqwest::Client> = OnceLock::new();
    let follow = |only_art: bool| {
        Policy::custom(move |attempt| {
            let url = attempt.url().as_str();
            let allowed = attempt.url().scheme() == "https" && (!only_art || crate::art::allowed(url));
            if attempt.previous().len() >= MAX_REDIRECTS || !allowed { attempt.stop() } else { attempt.follow() }
        })
    };
    match redirects {
        Redirects::Follow => FOLLOWING.get_or_init(|| build(follow(false))),
        Redirects::ArtSources => ART.get_or_init(|| build(follow(true))),
    }
}

pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
}

/// GET with extra headers. Any status comes back as a `Response`; only transport failures error.
pub async fn get(url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Response> {
    get_with(url, headers, timeout, DEFAULT_LIMIT, Redirects::Follow).await
}

/// GET with an explicit body-size limit and redirect policy. A redirect the policy does not
/// follow comes back as the 3xx response itself.
pub async fn get_with(
    url: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
    max_bytes: u64,
    redirects: Redirects,
) -> Result<Response> {
    if offline() {
        return Err(Offline.into());
    }
    let mut req = client(redirects).get(url).timeout(timeout);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut resp = req.send().await.with_context(|| format!("GET {}", without_query(url)))?;
    let status = resp.status().as_u16();
    let etag = resp.headers().get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_owned);
    if resp.content_length().is_some_and(|n| n > max_bytes) {
        bail!("the answer from {} is larger than {max_bytes} bytes", without_query(url));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.with_context(|| format!("reading {}", without_query(url)))? {
        if body.len() as u64 + chunk.len() as u64 > max_bytes {
            bail!("the answer from {} is larger than {max_bytes} bytes", without_query(url));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Response { status, etag, body })
}

/// GET a JSON document; a non-2xx status is an `HttpStatus` error.
pub async fn get_json(url: &str, headers: &[(&str, &str)], timeout: Duration) -> Result<Value> {
    let mut all = vec![("Accept", "application/json")];
    all.extend_from_slice(headers);
    let resp = get(url, &all, timeout).await?;
    if !(200..300).contains(&resp.status) {
        return Err(HttpStatus(resp.status).into());
    }
    Ok(serde_json::from_slice(&resp.body)?)
}

/// The URL without its query string, for error messages (queries can carry ids).
fn without_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
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

/// Talks to the app's own loopback server from the command line (`warframeproject trigger`,
/// `quit`, the single-instance check) without starting an async runtime: plain HTTP/1.1 over a
/// TCP socket, one request per connection.
pub mod local {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow, bail};
    use serde_json::Value;

    /// Sends `GET /api/<path>`, or `POST` with an empty JSON object, and reads the JSON answer.
    pub fn call(port: u16, path: &str, post: bool, timeout: Duration) -> Result<Value> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&addr, timeout.min(Duration::from_secs(5)))
            .with_context(|| format!("nothing answers on port {port}"))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        let (method, body) = if post { ("POST", "{}") } else { ("GET", "") };
        let request = format!(
            "{method} /api/{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes())?;
        let mut raw = Vec::new();
        stream.take(64 * 1024 * 1024).read_to_end(&mut raw).context("reading the answer")?;
        parse(&raw)
    }

    /// The JSON body of a raw HTTP/1.1 answer (fixed length or chunked).
    pub(super) fn parse(raw: &[u8]) -> Result<Value> {
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| anyhow!("not an HTTP answer"))?;
        let head = String::from_utf8_lossy(&raw[..split]);
        let mut body = &raw[split + 4..];
        let status: u16 = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow!("not an HTTP answer"))?;
        let chunked = head.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("transfer-encoding:") && l.contains("chunked")
        });
        let decoded;
        if chunked {
            decoded = dechunk(body)?;
            body = &decoded;
        }
        let value: Value = serde_json::from_slice(body).context("the answer is not JSON")?;
        if !(200..300).contains(&status) {
            match value["error"].as_str() {
                Some(e) => bail!("{e}"),
                None => bail!("HTTP {status}"),
            }
        }
        Ok(value)
    }

    fn dechunk(mut body: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let line_end =
                body.windows(2).position(|w| w == b"\r\n").ok_or_else(|| anyhow!("broken chunked answer"))?;
            let size_text = String::from_utf8_lossy(&body[..line_end]);
            let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
                .map_err(|_| anyhow!("broken chunked answer"))?;
            body = &body[line_end + 2..];
            if size == 0 {
                return Ok(out);
            }
            if body.len() < size + 2 {
                bail!("broken chunked answer");
            }
            out.extend_from_slice(&body[..size]);
            body = &body[size + 2..];
        }
    }
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

    #[test]
    fn local_answers_are_read_plain_and_chunked() {
        let plain = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 11\r\n\r\n{\"ok\":true}";
        assert_eq!(local::parse(plain).unwrap(), serde_json::json!({"ok": true}));
        let chunked = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\n{\"ok\"\r\n6\r\n:true}\r\n0\r\n\r\n";
        assert_eq!(local::parse(chunked).unwrap(), serde_json::json!({"ok": true}));
        let refused = b"HTTP/1.1 400 Bad Request\r\ncontent-length: 18\r\n\r\n{\"error\":\"nope!\"}\n";
        assert_eq!(local::parse(refused).unwrap_err().to_string(), "nope!");
    }

    #[test]
    fn errors_name_the_address_without_its_query() {
        assert_eq!(without_query("https://x.example/a/b?playerId=123#f"), "https://x.example/a/b");
    }
}
