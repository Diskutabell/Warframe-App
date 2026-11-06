//! Loopback HTTP: the JSON API and the web UI. Binds to 127.0.0.1 only.
//!
//! The endpoints the hotkeys call stay the same between versions, so existing binds
//! (`curl -X POST http://127.0.0.1:8642/api/rewards/scan`) keep working.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Result, anyhow};
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::UserError;
use crate::art;
use crate::memscan::ScanError;
use crate::state::State;

#[derive(RustEmbed)]
#[folder = "web/"]
pub(crate) struct Web;

const MAX_BODY: u64 = 200 * 1024 * 1024;

pub fn bind(port: u16) -> Result<Arc<Server>> {
    Server::http(("127.0.0.1", port)).map(Arc::new).map_err(|e| anyhow!("{e}"))
}

/// Answers requests on `workers` threads. Scans block their thread for a second or two, so a
/// handful of workers keeps the UI responsive meanwhile.
pub fn serve(server: Arc<Server>, state: Arc<State>, workers: usize) {
    start_art_threads();
    for i in 0..workers {
        let (server, state) = (Arc::clone(&server), Arc::clone(&state));
        std::thread::Builder::new()
            .name(format!("http-{i}"))
            .spawn(move || {
                while let Ok(req) = server.recv() {
                    handle(&state, req);
                }
            })
            .expect("spawning an http worker");
    }
}

fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

/// Guards against DNS rebinding: only answer requests addressed to the loopback host.
fn host_ok(req: &Request) -> bool {
    let host = header(req, "Host").unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Rejects cross-site requests from other web pages: only our own UI may POST.
fn origin_ok(req: &Request) -> bool {
    match header(req, "Origin") {
        None => true,
        Some(origin) => origin.ends_with(&format!("//{}", header(req, "Host").unwrap_or(""))),
    }
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

fn split_url(url: &str) -> (String, HashMap<String, String>) {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let mut q = HashMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        q.entry(decode_component(k)).or_insert_with(|| decode_component(v));
    }
    (path.to_string(), q)
}

fn respond(req: Request, code: u16, body: Vec<u8>, ctype: &str) {
    let resp = Response::from_data(body)
        .with_status_code(code)
        .with_header(Header::from_bytes("Content-Type", ctype).expect("static header"))
        .with_header(Header::from_bytes("Cache-Control", "no-store").expect("static header"));
    let _ = req.respond(resp);
}

/// One request for art that has to be downloaded first.
struct ArtJob {
    req: Request,
    url: String,
    size: u32,
}

/// Art that is not cached yet is downloaded on threads of its own. A page full of new icons would
/// otherwise occupy every worker, and a hotkey scan would wait behind the downloads.
static ART_JOBS: OnceLock<async_channel::Sender<ArtJob>> = OnceLock::new();
const ART_THREADS: usize = 4;

fn start_art_threads() {
    let (tx, rx) = async_channel::unbounded::<ArtJob>();
    if ART_JOBS.set(tx).is_err() {
        return; // already running
    }
    for i in 0..ART_THREADS {
        let rx = rx.clone();
        std::thread::Builder::new()
            .name(format!("art-{i}"))
            .spawn(move || {
                while let Ok(job) = rx.recv_blocking() {
                    respond_art(job.req, art::get(&job.url, job.size));
                }
            })
            .expect("spawning an art thread");
    }
}

/// `/api/art?u=<url>&s=<px>`: item art scaled to the size the page draws it at (see `art.rs`).
fn serve_art(req: Request, query: &HashMap<String, String>) {
    let url = query.get("u").cloned().unwrap_or_default();
    let size = query.get("s").and_then(|s| s.parse().ok()).unwrap_or(96);
    if let Some(found) = art::cached(&url, size) {
        return respond_art(req, Ok(found));
    }
    match ART_JOBS.get() {
        // Dropping the request (the channel never closes in practice) just closes the connection.
        Some(jobs) => drop(jobs.send_blocking(ArtJob { req, url, size })),
        None => respond_art(req, art::get(&url, size)),
    }
}

/// A scaled image never changes, so the page may keep it for a day. A miss is a plain 404 and the
/// page hides the slot.
fn respond_art(req: Request, found: Result<art::Art>) {
    match found {
        Ok(found) => {
            let resp = Response::from_data(found.bytes)
                .with_header(Header::from_bytes("Content-Type", found.content_type).expect("static header"))
                .with_header(Header::from_bytes("Cache-Control", "max-age=86400").expect("static header"));
            let _ = req.respond(resp);
        }
        Err(_) => respond(req, 404, Vec::new(), "text/plain; charset=utf-8"),
    }
}

fn respond_json(req: Request, code: u16, body: &Value) {
    respond(req, code, serde_json::to_vec(body).unwrap_or_default(), "application/json");
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

/// 400 for problems the user can act on, 500 for bugs.
fn status_for(e: &anyhow::Error) -> u16 {
    if e.downcast_ref::<UserError>().is_some() || e.downcast_ref::<ScanError>().is_some() {
        400
    } else {
        eprintln!("request failed: {e:#}");
        500
    }
}

fn handle(state: &Arc<State>, mut req: Request) {
    if !host_ok(&req) {
        return respond_json(req, 403, &json!({"error": "forbidden host"}));
    }
    let (path, query) = split_url(req.url());
    match req.method() {
        Method::Get | Method::Head => {
            if path == "/api/art" {
                return serve_art(req, &query);
            }
            if let Some(api) = path.strip_prefix("/api/") {
                match get_api(state, api, &query) {
                    Ok(v) => respond_json(req, 200, &v),
                    Err(e) => {
                        let code = status_for(&e);
                        respond_json(req, code, &json!({"error": e.to_string()}))
                    }
                }
                return;
            }
            let name = match path.trim_start_matches('/') {
                "" => "index.html",
                p => p,
            };
            match Web::get(name).filter(|_| !name.split('/').any(|seg| seg == "..")) {
                Some(file) => respond(req, 200, file.data.into_owned(), content_type(name)),
                None => respond_json(req, 404, &json!({"error": "not found"})),
            }
        }
        Method::Post => {
            if !origin_ok(&req) {
                return respond_json(req, 403, &json!({"error": "cross-origin request refused"}));
            }
            let mut raw = Vec::new();
            if req.as_reader().take(MAX_BODY + 1).read_to_end(&mut raw).is_err() || raw.len() as u64 > MAX_BODY {
                return respond_json(req, 400, &json!({"error": "Upload too large"}));
            }
            let body: Value = if raw.iter().all(u8::is_ascii_whitespace) {
                json!({})
            } else {
                match serde_json::from_slice(&raw) {
                    Ok(v) => v,
                    Err(e) => return respond_json(req, 400, &json!({"error": format!("Not JSON: {e}")})),
                }
            };
            let api = path.strip_prefix("/api/").unwrap_or("");
            match post_api(state, api, &body) {
                Ok(v) => respond_json(req, 200, &v.unwrap_or_else(|| json!({"ok": true}))),
                Err(e) => {
                    let code = status_for(&e);
                    respond_json(req, code, &json!({"error": e.to_string()}))
                }
            }
        }
        _ => respond_json(req, 405, &json!({"error": "method not allowed"})),
    }
}

fn get_api(s: &Arc<State>, path: &str, q: &HashMap<String, String>) -> Result<Value> {
    if path == "state" {
        return Ok(s.summary());
    }
    if s.catalog().loaded_at.is_none() && !matches!(path, "settings" | "world" | "rewards" | "riven/result") {
        return Ok(json!({"loading": true, "error": s.catalog_error()}));
    }
    let count = |k: &str, default: u8| -> Result<u8> {
        match q.get(k).map(String::as_str) {
            None | Some("") => Ok(default),
            Some(v) => v.parse().map_err(|_| UserError::new(format!("{k} must be a number")).into()),
        }
    };
    Ok(match path {
        "mastery" => s.view().mastery(),
        "primes" => s.view().primes(),
        "foundry" => {
            let v = s.view();
            let mut items = v.craftable();
            if let Some(list) = items.as_array_mut() {
                list.extend(v.crafts());
            }
            json!({"pending": v.pending_foundry(), "items": items})
        }
        "relics" => s.view().relics(q.get("owned").is_some_and(|v| v == "1")),
        "trade" => s.trade(),
        "find" => s.view().find(q.get("q").map_or("", String::as_str)),
        "item" => s.detail(q.get("key").map_or("", String::as_str))?,
        "history" => s.history(),
        "world" => s.world(),
        "rewards" => s.rewards().map_or(json!({}), |r| serde_json::to_value(&*r).unwrap_or_default()),
        "hotkeys" => s.hotkeys(),
        "riven/result" => s.riven().map_or(json!({}), |r| serde_json::to_value(&*r).unwrap_or_default()),
        "riven" => {
            match s.riven_table(q.get("weapon").map_or("", String::as_str), count("pos", 3)?, count("neg", 1)?) {
                Ok(v) => v,
                Err(e) if e.downcast_ref::<UserError>().is_some() => json!({"error": e.to_string()}),
                Err(e) => return Err(e),
            }
        }
        "rivens" => s.rivens(),
        "fissures" => s.fissures(),
        "standing" => s.standing(),
        "syndicate" => s.syndicate(q.get("tag").map_or("", String::as_str))?,
        "settings" => serde_json::to_value(s.settings())?,
        "search" => s.search(q.get("q").map_or("", String::as_str)),
        _ => return Err(UserError::new(format!("unknown endpoint {path}")).into()),
    })
}

fn post_api(s: &Arc<State>, path: &str, b: &Value) -> Result<Option<Value>> {
    let uid = || b["uid"].as_str().map(str::to_owned).ok_or_else(|| UserError::new("uid is required"));
    match path {
        "settings" => s.update_settings(b)?,
        "account/set" => s.set_account_id(b["id"].as_str().unwrap_or(""), b["key"].as_str())?,
        "account/switch" => s.switch_account(b["key"].as_str().unwrap_or(""))?,
        "account/forget" => s.forget_account(b["key"].as_str().unwrap_or(""))?,
        "profile/refresh" => {
            let key = s
                .active_key()
                .ok_or_else(|| UserError::new("No account yet: start the game once, or enter an account id."))?;
            s.refresh_profile(&key, false)?;
        }
        "profile/import" => s.import_profile(b)?,
        "inventory/import" => {
            s.import_inventory(b, "imported file")?;
        }
        "inventory/clear" => s.clear_inventory(),
        "manual/count" => s.set_manual_count(&uid()?, b["count"].as_f64().map(|c| c as i64))?,
        "manual/pin" => s.set_pinned(&uid()?, b["pinned"].as_bool().unwrap_or(false))?,
        "manual/mastered" => s.set_manual_mastered(&uid()?, b["mastered"].as_bool().unwrap_or(false))?,
        "rewards/scan" => return Ok(Some(serde_json::to_value(&*s.scan_rewards(Duration::ZERO))?)),
        "window/show" => s.show_window(),
        "quit" => s.quit(),
        "overlay/test" => s.test_overlay()?,
        "hotkeys/register" => return Ok(Some(s.install_hotkeys()?)),
        "riven/scan" => return Ok(Some(serde_json::to_value(&*s.scan_riven())?)),
        "riven/grade" => return Ok(Some(s.grade_riven(b)?)),
        "data/reload" => {
            s.load_static(true);
            if let Some(e) = s.catalog_error() {
                return Err(UserError::new(e).into());
            }
        }
        _ => return Err(UserError::new(format!("unknown endpoint {path}")).into()),
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_decode_like_parse_qs() {
        let (path, q) = split_url("/api/riven?weapon=Kuva%20Bramma&pos=2&x=a+b&bad=%zz");
        assert_eq!(path, "/api/riven");
        assert_eq!(q["weapon"], "Kuva Bramma");
        assert_eq!(q["pos"], "2");
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
}
