//! `/api/art?u=<url>&s=<px>`: item art scaled to the size the page draws it at (see `art.rs`).
//!
//! A cached picture is answered at once. One that has to be downloaded waits for one of a few
//! download slots, so a page full of new icons cannot hold up a hotkey scan, and a flood of
//! requests is turned away instead of queueing without end.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::sync::Semaphore;

use super::{Engine, Query};
use crate::art;

/// Downloads at the same time.
const DOWNLOADS: usize = 4;
/// Requests waiting for a download slot, at most; more answer 503 and the page retries.
const WAITING_CAP: usize = 128;

static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(DOWNLOADS));
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// Counts a request as waiting for as long as it lives.
struct Waiting;

impl Waiting {
    fn join() -> Option<Waiting> {
        let before = WAITING.fetch_add(1, Ordering::AcqRel);
        if before >= WAITING_CAP {
            WAITING.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Waiting)
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        WAITING.fetch_sub(1, Ordering::AcqRel);
    }
}

pub async fn serve(State(_engine): State<Engine>, q: Query) -> Response {
    let url = q.str("u").to_string();
    let size: u32 = q.0.get("s").and_then(|s| s.parse().ok()).unwrap_or(96);
    let lookup = url.clone();
    if let Ok(Some(found)) = tokio::task::spawn_blocking(move || art::cached(&lookup, size)).await {
        return answer(Ok(found));
    }
    let Some(_waiting) = Waiting::join() else {
        return text(StatusCode::SERVICE_UNAVAILABLE, "art queue busy");
    };
    let Ok(_slot) = SLOTS.acquire().await else {
        return text(StatusCode::INTERNAL_SERVER_ERROR, "art worker unavailable");
    };
    match tokio::task::spawn_blocking(move || art::get(&url, size)).await {
        Ok(found) => answer(found),
        // A panic in the download or the scaling: a bug, but only this picture is lost.
        Err(_) => text(StatusCode::NOT_FOUND, ""),
    }
}

/// A scaled image never changes, so the page may keep it for a day. A miss is a plain 404 and the
/// page hides the slot.
fn answer(found: anyhow::Result<art::Art>) -> Response {
    match found {
        Ok(found) => (
            [
                (CONTENT_TYPE, HeaderValue::from_static(found.content_type)),
                (CACHE_CONTROL, HeaderValue::from_static("max-age=86400")),
            ],
            found.bytes,
        )
            .into_response(),
        Err(_) => text(StatusCode::NOT_FOUND, ""),
    }
}

fn text(code: StatusCode, body: &'static str) -> Response {
    (code, [(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"))], body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waiting_line_is_capped() {
        let held: Vec<Waiting> = std::iter::from_fn(Waiting::join).take(WAITING_CAP + 10).collect();
        assert_eq!(held.len(), WAITING_CAP);
        assert!(Waiting::join().is_none());
        drop(held);
        assert!(Waiting::join().is_some());
    }
}
