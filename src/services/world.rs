//! The live world: the community worldstate API (api.warframestat.us), fetched at most once a
//! minute and shared by every page, the notifications and the reward overlays.
//!
//! A copy that is out of date is still answered at once while a fresh one is fetched; only a
//! cold cache (first start, another platform) makes the caller wait. Each fresh copy is announced
//! on the bus, so the pages showing it redraw without asking.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::lock;
use super::settings::SettingsStore;
use crate::bus::{Bus, Topic};
use crate::config::Platform;
use crate::{net, now, world};

/// A copy this young is used as it is.
const MAX_AGE: f64 = 60.0;
/// After a failed fetch the source is asked again this much sooner than after a good one.
const ERROR_RETRY_AFTER: f64 = 15.0;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
struct Cache {
    platform: Option<Platform>,
    raw: Option<Arc<Value>>,
    error: Option<String>,
    /// When it was last fetched (or last failed, moved back so a retry comes sooner).
    t: f64,
}

impl Cache {
    fn fresh_for(&self, platform: Platform, t: f64) -> bool {
        self.platform == Some(platform) && t - self.t < MAX_AGE
    }
}

/// Fetches the worldstate of a platform: the API, or a test's stand-in.
type Fetch = Box<dyn Fn(Platform) -> BoxFuture<'static, anyhow::Result<Value>> + Send + Sync>;

pub struct WorldService {
    cache: Mutex<Cache>,
    /// One fetch at a time; callers that find the cache cold wait for it.
    fetching: tokio::sync::Mutex<()>,
    fetch: Fetch,
    settings: Arc<SettingsStore>,
    bus: Bus,
}

/// The worldstate as some caller got it: the document, and what went wrong if it is missing or
/// old.
pub type Snapshot = (Option<Arc<Value>>, Option<String>);

impl WorldService {
    pub fn new(settings: Arc<SettingsStore>, bus: &Bus) -> WorldService {
        let fetch: Fetch = Box::new(|platform| {
            Box::pin(async move {
                let url = world::API.replace("{platform}", platform.as_str());
                net::get_json(&url, &[], FETCH_TIMEOUT).await
            })
        });
        WorldService::with_fetch(settings, bus, fetch)
    }

    fn with_fetch(settings: Arc<SettingsStore>, bus: &Bus, fetch: Fetch) -> WorldService {
        WorldService {
            cache: Mutex::new(Cache { t: f64::NEG_INFINITY, ..Cache::default() }),
            fetching: tokio::sync::Mutex::new(()),
            fetch,
            settings,
            bus: bus.clone(),
        }
    }

    /// The worldstate of the platform in the settings: fresh, or the last copy while a new one is
    /// fetched in the background.
    pub async fn get(self: &Arc<Self>) -> Snapshot {
        let platform = self.settings.get().platform;
        {
            let c = lock(&self.cache);
            if c.fresh_for(platform, now()) {
                return (c.raw.clone(), c.error.clone());
            }
            if c.platform == Some(platform) && c.raw.is_some() {
                // Out of date but usable: answer now, fetch for the next caller.
                let me = Arc::clone(self);
                tokio::spawn(async move { me.refresh(platform).await });
                return (c.raw.clone(), c.error.clone());
            }
        }
        self.refresh(platform).await
    }

    /// Fetches the worldstate unless another caller has just done so, and keeps it when the
    /// platform is still the one asked for.
    async fn refresh(&self, platform: Platform) -> Snapshot {
        let _one_fetch = self.fetching.lock().await;
        {
            let c = lock(&self.cache);
            if c.fresh_for(platform, now()) {
                return (c.raw.clone(), c.error.clone());
            }
        }
        let fetched = (self.fetch)(platform).await;
        let mut c = lock(&self.cache);
        if self.settings.get().platform != platform {
            // The platform changed while this was in flight: hand the caller what it asked for,
            // but keep it out of the cache of the platform shown now.
            return match fetched {
                Ok(raw) => (Some(Arc::new(raw)), None),
                Err(e) => (None, Some(format!("World state unavailable: {e}"))),
            };
        }
        if c.platform != Some(platform) {
            // Another platform's world is no stand-in.
            *c = Cache { platform: Some(platform), t: f64::NEG_INFINITY, ..Cache::default() };
        }
        match fetched {
            Ok(raw) => {
                c.raw = Some(Arc::new(raw));
                c.error = None;
                c.t = now();
                self.bus.signal(Topic::World);
            }
            Err(e) => {
                c.error = Some(format!("World state unavailable: {e}"));
                // A transient outage recovers soon without asking every second.
                c.t = now() - (MAX_AGE - ERROR_RETRY_AFTER).max(0.0);
            }
        }
        (c.raw.clone(), c.error.clone())
    }

    /// Keeps the worldstate current while a page listens, so fissures and timers redraw on their
    /// own. Nobody listening, nothing is fetched.
    pub async fn run(self: Arc<Self>, stop: CancellationToken) {
        loop {
            // Until the copy held is due: a minute after it was fetched (sooner after an error).
            let due = (lock(&self.cache).t + MAX_AGE - now()).clamp(1.0, MAX_AGE);
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs_f64(due)) => {}
            }
            if self.bus.has_listeners() {
                let platform = self.settings.get().platform;
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = self.refresh(platform) => {}
                }
            }
        }
    }

    /// The relic eras with a fissure open right now ("Omnia" opens them all); storms are
    /// Railjack's.
    pub async fn open_eras(self: &Arc<Self>) -> std::collections::HashSet<String> {
        let (raw, _) = self.get().await;
        world::fissures(raw.as_deref())
            .iter()
            .filter(|f| f["storm"] != true)
            .filter_map(|f| f["tier"].as_str().map(str::to_owned))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn service(calls: Arc<AtomicUsize>, delay_ms: u64) -> (Arc<WorldService>, Arc<SettingsStore>) {
        let bus = Bus::new();
        let settings = Arc::new(SettingsStore::new(Settings::default(), &bus));
        let fetch: Fetch = Box::new(move |platform| {
            let calls = Arc::clone(&calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                Ok(json!({"platform": platform.as_str()}))
            })
        });
        (Arc::new(WorldService::with_fetch(Arc::clone(&settings), &bus, fetch)), settings)
    }

    #[tokio::test]
    async fn cold_callers_share_one_fetch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (world, _) = service(Arc::clone(&calls), 40);
        let answers = futures_util::future::join_all((0..4).map(|_| world.get())).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one fetch for every cold caller");
        for (raw, err) in answers {
            assert!(err.is_none());
            assert_eq!(raw.unwrap()["platform"], "pc");
        }
        // Fresh: answered from the cache.
        let (raw, _) = world.get().await;
        assert_eq!(raw.unwrap()["platform"], "pc");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_stale_copy_is_answered_at_once_and_refreshed_behind_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (world, _) = service(Arc::clone(&calls), 0);
        world.get().await;
        lock(&world.cache).t -= MAX_AGE + 1.0;
        let (raw, _) = world.get().await;
        assert!(raw.is_some(), "the old copy is answered");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "and a new one fetched behind it");
        assert!(lock(&world.cache).fresh_for(Platform::Pc, now()));
    }

    #[tokio::test]
    async fn another_platforms_world_is_never_shown() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (world, settings) = service(Arc::clone(&calls), 0);
        world.get().await;
        settings.set_in_memory(Settings { platform: Platform::Ps4, ..Settings::default() });
        let (raw, _) = world.get().await;
        assert_eq!(raw.unwrap()["platform"], "ps4");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
