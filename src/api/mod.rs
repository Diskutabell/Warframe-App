//! Loopback HTTP: the JSON API, the event stream and the web UI. Binds to 127.0.0.1 only.
//!
//! The endpoints the hotkeys call stay the same between versions, so existing binds
//! (`curl -X POST http://127.0.0.1:8642/api/rewards/scan`) keep working.

mod art;
mod events;
mod routes;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::anyhow;
use axum::Router;
use axum::body::Body;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, HOST, ORIGIN};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tower_http::catch_panic::CatchPanicLayer;

use crate::UserError;
use crate::app::App;
use crate::memscan::ScanError;
use crate::sources::ProfileRefused;

#[derive(RustEmbed)]
#[folder = "web/"]
pub(crate) struct Web;

/// What every handler gets: the engine.
pub type Engine = Arc<App>;

/// Default cap for JSON API POST bodies; only inventory/profile file imports allow large payloads.
pub const MAX_BODY: usize = 1024 * 1024;
pub const MAX_IMPORT_BODY: usize = 200 * 1024 * 1024;

/// Serves the API on `listener` until `stop` is cancelled. Requests still running then finish;
/// the event streams end at once.
pub async fn serve(listener: tokio::net::TcpListener, engine: Engine, stop: CancellationToken) -> std::io::Result<()> {
    let app = router(engine, stop.clone());
    axum::serve(listener, app).with_graceful_shutdown(stop.cancelled_owned()).await
}

pub fn router(engine: Engine, stop: CancellationToken) -> Router {
    routes::api(engine.clone(), stop)
        .fallback(static_file)
        .layer(middleware::from_fn(guard))
        // A panic in a request (a bug) answers 500 instead of dropping the connection; the panic
        // itself is printed to the log.
        .layer(CatchPanicLayer::custom(|_: Box<dyn std::any::Any + Send>| {
            json_response(StatusCode::INTERNAL_SERVER_ERROR, &json!({"error": BUG}))
        }))
        .with_state(engine)
}

// ------------------------------------------------------------------ requests

fn header(headers: &axum::http::HeaderMap, name: axum::http::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

pub(crate) fn valid_host(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix('[') {
        let Some((addr, after)) = rest.split_once(']') else { return false };
        let port_ok = after.is_empty()
            || after.strip_prefix(':').is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        return addr == "::1" && port_ok;
    }
    let name = match host.split_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        Some(_) => return false,
        None => host,
    };
    matches!(name, "127.0.0.1" | "localhost")
}

pub(crate) fn valid_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(origin) => host.is_some_and(|h| !h.is_empty() && origin == format!("http://{h}")),
    }
}

/// Guards every request: only answer requests addressed to the loopback host (DNS rebinding),
/// and only our own UI may POST (cross-site requests from other pages are refused). Answers are
/// never cached unless a handler says otherwise (item art).
async fn guard(req: Request, next: Next) -> Response {
    let host = header(req.headers(), HOST);
    if !valid_host(host.unwrap_or("")) {
        return json_response(StatusCode::FORBIDDEN, &json!({"error": "forbidden host"}));
    }
    if req.method() == Method::POST && !valid_origin(header(req.headers(), ORIGIN), host) {
        return json_response(StatusCode::FORBIDDEN, &json!({"error": "cross-origin request refused"}));
    }
    let mut resp = next.run(req).await;
    resp.headers_mut().entry(CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    resp
}

/// Percent-decoding as `urllib.parse.parse_qs` does it: `+` is a space, a bad escape stays as is.
fn decode_component(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%' && i + 2 < bytes.len())
            .then(|| Some(hex(bytes[i + 1])? * 16 + hex(bytes[i + 2])?))
            .flatten();
        match (bytes[i], escaped) {
            (_, Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (b, None) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query string's parameters; the first of repeated keys wins.
fn parse_query(query: &str) -> HashMap<String, String> {
    let mut q = HashMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        q.entry(decode_component(k)).or_insert_with(|| decode_component(v));
    }
    q
}

/// The request's query parameters.
pub struct Query(pub HashMap<String, String>);

impl Query {
    pub fn str(&self, key: &str) -> &str {
        self.0.get(key).map_or("", String::as_str)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Query {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Query(parse_query(parts.uri.query().unwrap_or(""))))
    }
}

/// A JSON body of at most `MAX` bytes. An empty body is `{}`; anything else that is not JSON is
/// the caller's mistake (400), as is a body over the limit.
pub struct JsonBody<const MAX: usize>(pub Value);

impl<S: Send + Sync, const MAX: usize> FromRequest<S> for JsonBody<MAX> {
    type Rejection = Response;

    async fn from_request(req: Request, _: &S) -> Result<Self, Self::Rejection> {
        let raw = axum::body::to_bytes(req.into_body(), MAX)
            .await
            .map_err(|_| json_response(StatusCode::BAD_REQUEST, &json!({"error": "Upload too large"})))?;
        if raw.iter().all(u8::is_ascii_whitespace) {
            return Ok(JsonBody(json!({})));
        }
        serde_json::from_slice(&raw)
            .map(JsonBody)
            .map_err(|e| json_response(StatusCode::BAD_REQUEST, &json!({"error": format!("Not JSON: {e}")})))
    }
}

// ------------------------------------------------------------------ answers

pub fn json_response(code: StatusCode, body: &Value) -> Response {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    (code, [(CONTENT_TYPE, HeaderValue::from_static("application/json"))], bytes).into_response()
}

/// A failed request: 400 for problems the user can act on, 500 for bugs.
pub struct ApiError(pub anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

pub fn status_for(e: &anyhow::Error) -> StatusCode {
    let users = e.downcast_ref::<UserError>().is_some()
        || e.downcast_ref::<ScanError>().is_some()
        || e.downcast_ref::<ProfileRefused>().is_some();
    if users {
        StatusCode::BAD_REQUEST
    } else {
        eprintln!("request failed: {e:#}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        json_response(status_for(&self.0), &json!({"error": self.0.to_string()}))
    }
}

pub type ApiResult = Result<Response, ApiError>;

/// What a bug (a panic) in an API call answers. The panic itself is printed to the log.
pub const BUG: &str = "The engine hit a bug answering this request; its log says where.";

/// Runs blocking engine work (views over the whole catalog, file and network access in the
/// older parts of the engine) on the blocking pool, so the server's threads stay free. A panic
/// in it is a bug and answers 500 instead of ending anything.
pub async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result.map_err(ApiError),
        Err(e) if e.is_panic() => Err(ApiError(anyhow!(BUG))),
        Err(e) => Err(ApiError(anyhow!("the request was cancelled: {e}"))),
    }
}

/// Answers with `value` as JSON (200).
pub fn ok_json(value: &Value) -> Response {
    json_response(StatusCode::OK, value)
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ttf" => "font/ttf",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The web UI's files; anything else is a 404. Only GET and HEAD.
async fn static_file(method: Method, uri: axum::http::Uri) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return json_response(StatusCode::METHOD_NOT_ALLOWED, &json!({"error": "method not allowed"}));
    }
    let name = match uri.path().trim_start_matches('/') {
        "" => "index.html",
        p => p,
    };
    match Web::get(name).filter(|_| !name.split('/').any(|seg| seg == "..")) {
        Some(file) => {
            ([(CONTENT_TYPE, HeaderValue::from_static(content_type(name)))], Body::from(file.data.into_owned()))
                .into_response()
        }
        None => json_response(StatusCode::NOT_FOUND, &json!({"error": "not found"})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_decode_like_parse_qs() {
        let q = parse_query("weapon=Kuva%20Bramma&pos=2&x=a+b&bad=%zz&pos=3");
        assert_eq!(q["weapon"], "Kuva Bramma");
        assert_eq!(q["pos"], "2", "the first of repeated keys wins");
        assert_eq!(q["x"], "a b");
        assert_eq!(q["bad"], "%zz");
    }

    #[test]
    fn the_ui_is_embedded() {
        assert!(Web::get("index.html").is_some());
        assert!(Web::get("js/main.js").is_some());
        assert!(Web::get("js/pages/settings.js").is_some());
        assert!(Web::get("../Cargo.toml").is_none());
    }

    #[test]
    fn a_refused_profile_is_the_users_problem_not_a_bug() {
        assert_eq!(status_for(&ProfileRefused::new(403, "refused".into()).into()), StatusCode::BAD_REQUEST);
        assert_eq!(status_for(&anyhow!("boom")), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn hosts_and_origins_are_checked() {
        assert!(valid_host("127.0.0.1:8642"));
        assert!(valid_host("127.0.0.1"));
        assert!(valid_host("localhost:8642"));
        assert!(valid_host("[::1]:8642"));
        assert!(valid_host("[::1]"));
        assert!(!valid_host("[::1]:abc"));
        assert!(!valid_host("example.com:8642"));
        assert!(!valid_host("127.0.0.1.evil.example"));
        assert!(!valid_host(""));

        assert!(valid_origin(None, Some("127.0.0.1:8642")));
        assert!(valid_origin(Some("http://127.0.0.1:8642"), Some("127.0.0.1:8642")));
        assert!(!valid_origin(Some("https://127.0.0.1:8642"), Some("127.0.0.1:8642")));
        assert!(!valid_origin(Some("file://127.0.0.1:8642"), Some("127.0.0.1:8642")));
        assert!(!valid_origin(Some("http://evil.example//127.0.0.1:8642"), Some("127.0.0.1:8642")));
    }

    #[tokio::test]
    async fn a_panicking_call_answers_with_an_error_instead_of_ending_anything() {
        let failed = blocking(|| -> anyhow::Result<()> { panic!("a bug") }).await;
        let resp = failed.expect_err("a panic is an error").into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(blocking(|| Ok(1)).await.ok(), Some(1));
    }

    #[tokio::test]
    async fn bodies_over_the_limit_and_non_json_are_refused() {
        let req = |body: &'static str| Request::builder().method("POST").body(Body::from(body)).unwrap();
        let JsonBody(v) = JsonBody::<16>::from_request(req("  "), &()).await.ok().unwrap();
        assert_eq!(v, json!({}));
        let JsonBody(v) = JsonBody::<16>::from_request(req(r#"{"a": 1}"#), &()).await.ok().unwrap();
        assert_eq!(v, json!({"a": 1}));
        let big = JsonBody::<16>::from_request(req(r#"{"a": "0123456789abcdef"}"#), &()).await.err().unwrap();
        assert_eq!(big.status(), StatusCode::BAD_REQUEST);
        let bad = JsonBody::<16>::from_request(req("{nope"), &()).await.err().unwrap();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    }
}
