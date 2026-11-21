//! The game data (`gamedata::Catalog`): loaded at startup, reloaded on request, and the names the
//! reward-screen reader looks for.

use std::sync::{Arc, Mutex, RwLock};

use super::lock;
use crate::bus::{Bus, Topic};
use crate::gamedata::{self, Catalog};
use crate::ocr::matcher::Names;

pub struct CatalogStore {
    catalog: RwLock<Arc<Catalog>>,
    /// Why the last load failed, while the game data is not there.
    error: Mutex<Option<String>>,
    /// The reward names, built once per catalog.
    names: Mutex<Option<(Arc<Catalog>, Arc<Names>)>>,
    /// One load at a time: a reload asked for during the startup load waits for it.
    loading: tokio::sync::Mutex<()>,
    bus: Bus,
}

impl CatalogStore {
    pub fn new(bus: &Bus) -> CatalogStore {
        CatalogStore {
            catalog: RwLock::new(Arc::new(Catalog::default())),
            error: Mutex::new(None),
            names: Mutex::new(None),
            loading: tokio::sync::Mutex::new(()),
            bus: bus.clone(),
        }
    }

    /// The current game data; empty (`loaded_at` None) until the first load is done.
    pub fn get(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog.read().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn ready(&self) -> bool {
        self.get().loaded_at.is_some()
    }

    pub fn error(&self) -> Option<String> {
        lock(&self.error).clone()
    }

    /// Downloads (or revalidates) the game data and swaps it in. A failure keeps the data there
    /// was and says why.
    pub async fn load(&self, force: bool) -> Result<(), String> {
        let _one_at_a_time = self.loading.lock().await;
        let result = match gamedata::load(force).await {
            Ok(c) => {
                *self.catalog.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(c);
                *lock(&self.error) = None;
                self.bus.signal(Topic::Catalog);
                Ok(())
            }
            Err(e) => {
                let why = format!("{e:#}");
                *lock(&self.error) = Some(why.clone());
                Err(why)
            }
        };
        self.bus.state_changed();
        result
    }

    /// The names a reward card can carry, matched by the OCR.
    pub fn names(&self) -> Arc<Names> {
        let cat = self.get();
        let mut cache = lock(&self.names);
        if let Some((c, n)) = cache.as_ref() {
            if Arc::ptr_eq(c, &cat) {
                return Arc::clone(n);
            }
        }
        let names = Arc::new(Names::new(&cat.reward_names()));
        *cache = Some((cat, Arc::clone(&names)));
        names
    }
}
