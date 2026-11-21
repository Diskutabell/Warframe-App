//! `/api/events`: what changed in the engine, as Server-Sent Events.
//!
//! The first event is the summary as it is now, so a page that connects (or reconnects after the
//! engine restarted) is current at once. After that every bus message follows in order. A page
//! that falls too far behind is sent `resync` and fetches everything again.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{Stream, StreamExt, stream};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_util::sync::CancellationToken;

use super::Engine;
use crate::bus::Topic;

/// How long the browser waits before reconnecting when the stream breaks.
const RETRY: Duration = Duration::from_secs(2);
/// A comment line this often keeps idle connections from being dropped as dead.
const KEEP_ALIVE: Duration = Duration::from_secs(15);

pub async fn stream(
    State(engine): State<Engine>,
    stop: CancellationToken,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe before computing the first summary: a change made meanwhile is then sent too,
    // never lost between the two.
    let rx = engine.events().subscribe();
    let summary = {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || engine.summary()).await.ok()
    };
    let first = summary.map(|s| Event::default().event(Topic::State.name()).retry(RETRY).data(s.to_string()));
    let rest = BroadcastStream::new(rx).map(|msg| match msg {
        Ok(msg) => Event::default().event(msg.topic.name()).id(msg.id.to_string()).data(&msg.data),
        Err(BroadcastStreamRecvError::Lagged(_)) => Event::default().event(Topic::Resync.name()).data("{}"),
    });
    let events = stream::iter(first).chain(rest).map(Ok).take_until(stop.cancelled_owned());
    Sse::new(events).keep_alive(KeepAlive::new().interval(KEEP_ALIVE).text("ping"))
}
