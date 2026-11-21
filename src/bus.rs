//! The engine's event bus: what changed, pushed to the UI over Server-Sent Events.
//!
//! Everything the window shows used to be polled every 5 s, which recomputed the whole summary
//! (mastery, collection, foundry, farming board, reward history) whether or not anything had
//! changed. Now each change publishes a topic on this bus and `/api/events` streams it to the
//! page, which redraws only what the topic touches.
//!
//! The summary itself (`/api/state`) changes in many small ways (an activity line, a memory-scan
//! progress tick, an account linked). Those only mark it dirty (`state_changed`); `run_summary`
//! coalesces the marks and pushes one fresh summary at most every `SUMMARY_COALESCE`, and only
//! while a page is listening.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

/// How many events a slow page may fall behind before it is told to fetch everything again.
const CAPACITY: usize = 256;
/// Summary pushes are at least this far apart: a burst of changes (a memory scan's progress, a
/// profile filed under another account) becomes one push. Well inside the 200 ms the UI allows.
const SUMMARY_COALESCE: Duration = Duration::from_millis(120);
/// A summary is pushed at least this often while a page listens, for what changes with the clock
/// alone (the game counted idle 10 minutes after the log's last line, a rate-limit pause ending).
const SUMMARY_HEARTBEAT: Duration = Duration::from_secs(60);

/// What an event is about. The page decides what to redraw from the topic alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Topic {
    /// The summary (`/api/state`); the data is the summary itself.
    State,
    /// Another account is shown: everything on the page belongs to it.
    Account,
    /// The shown account's inventory, profile or hand-set data changed.
    Inventory,
    /// The game data finished loading or was reloaded.
    Catalog,
    /// A fresh worldstate (fissures, cycles, vendors).
    World,
    /// warframe.market prices arrived; the data lists the slugs.
    Prices,
    /// The farming board changed.
    Tasks,
    /// A reward screen was read (the data is the result, prices included once known).
    Rewards,
    /// A riven was read and graded.
    Riven,
    /// A reward screen was added to the history.
    History,
    /// Settings were changed (by any window, or the tray).
    Settings,
    /// The client fell behind and lost events: fetch everything again.
    Resync,
}

impl Topic {
    pub fn name(self) -> &'static str {
        match self {
            Topic::State => "state",
            Topic::Account => "account",
            Topic::Inventory => "inventory",
            Topic::Catalog => "catalog",
            Topic::World => "world",
            Topic::Prices => "prices",
            Topic::Tasks => "tasks",
            Topic::Rewards => "rewards",
            Topic::Riven => "riven",
            Topic::History => "history",
            Topic::Settings => "settings",
            Topic::Resync => "resync",
        }
    }
}

/// One published event: its topic and its data, serialised once for every listener.
#[derive(Debug)]
pub struct Message {
    pub topic: Topic,
    /// JSON text.
    pub data: String,
    /// Increases with every message; a page that reconnects can tell old from new.
    pub id: u64,
}

#[derive(Clone)]
pub struct Bus {
    inner: Arc<Inner>,
}

struct Inner {
    tx: broadcast::Sender<Arc<Message>>,
    summary_dirty: Notify,
    next_id: AtomicU64,
}

impl Default for Bus {
    fn default() -> Self {
        Bus::new()
    }
}

impl Bus {
    pub fn new() -> Bus {
        let (tx, _) = broadcast::channel(CAPACITY);
        Bus { inner: Arc::new(Inner { tx, summary_dirty: Notify::new(), next_id: AtomicU64::new(1) }) }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Message>> {
        self.inner.tx.subscribe()
    }

    /// Whether any page is listening: nothing is worth computing for nobody.
    pub fn has_listeners(&self) -> bool {
        self.inner.tx.receiver_count() > 0
    }

    /// Publishes `data` (serialised now) under `topic`. Without listeners this costs nothing.
    pub fn publish<T: Serialize + ?Sized>(&self, topic: Topic, data: &T) {
        if !self.has_listeners() {
            return;
        }
        match serde_json::to_string(data) {
            Ok(data) => self.publish_raw(topic, data),
            Err(e) => eprintln!("could not serialise a {} event: {e}", topic.name()),
        }
    }

    /// Publishes a topic that carries no data of its own.
    pub fn signal(&self, topic: Topic) {
        if self.has_listeners() {
            self.publish_raw(topic, "{}".to_string());
        }
    }

    fn publish_raw(&self, topic: Topic, data: String) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        // An error only means nobody listens any more.
        let _ = self.inner.tx.send(Arc::new(Message { topic, data, id }));
    }

    /// Marks the summary dirty; `run_summary` pushes a fresh one shortly.
    pub fn state_changed(&self) {
        self.inner.summary_dirty.notify_one();
    }

    /// Pushes the summary whenever it was marked dirty (coalesced), and once a minute regardless,
    /// until `stop` is cancelled. `compute` builds the summary; it runs on a blocking thread
    /// because it walks the whole catalog.
    pub async fn run_summary<F>(self, stop: CancellationToken, compute: F)
    where
        F: Fn() -> serde_json::Value + Send + Sync + 'static,
    {
        let compute = Arc::new(compute);
        loop {
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = self.inner.summary_dirty.notified() => {}
                _ = tokio::time::sleep(SUMMARY_HEARTBEAT) => {}
            }
            // Let the rest of a burst arrive before computing once for all of it.
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(SUMMARY_COALESCE) => {}
            }
            if !self.has_listeners() {
                continue;
            }
            let compute = Arc::clone(&compute);
            match tokio::task::spawn_blocking(move || compute()).await {
                Ok(summary) => self.publish(Topic::State, &summary),
                Err(e) => eprintln!("the summary could not be computed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_reach_every_listener_in_order() {
        let bus = Bus::new();
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        bus.publish(Topic::Prices, &serde_json::json!({"slugs": ["ash_prime_set"]}));
        bus.signal(Topic::Catalog);
        for rx in [&mut a, &mut b] {
            let first = rx.recv().await.unwrap();
            assert_eq!(first.topic, Topic::Prices);
            assert_eq!(first.data, r#"{"slugs":["ash_prime_set"]}"#);
            let second = rx.recv().await.unwrap();
            assert_eq!(second.topic, Topic::Catalog);
            assert!(second.id > first.id);
        }
    }

    #[tokio::test]
    async fn nothing_is_serialised_without_listeners() {
        struct Panics;
        impl Serialize for Panics {
            fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                panic!("serialised without a listener")
            }
        }
        let bus = Bus::new();
        bus.publish(Topic::World, &Panics);
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_changes_becomes_one_summary() {
        let bus = Bus::new();
        let mut rx = bus.subscribe();
        let stop = CancellationToken::new();
        let computed = Arc::new(AtomicU64::new(0));
        let count = Arc::clone(&computed);
        let task = tokio::spawn(
            bus.clone()
                .run_summary(stop.clone(), move || serde_json::json!({"n": count.fetch_add(1, Ordering::Relaxed) + 1})),
        );
        for _ in 0..50 {
            bus.state_changed();
        }
        let msg = rx.recv().await.unwrap();
        assert_eq!((msg.topic, msg.data.as_str()), (Topic::State, r#"{"n":1}"#));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(computed.load(Ordering::Relaxed), 1, "the burst was computed once");
        stop.cancel();
        task.await.unwrap();
    }
}
