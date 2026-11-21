//! Desktop notifications, when switched on: watched fissures, Baro Ki'Teer and finished foundry
//! builds. Each fires once.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::accounts::AccountService;
use super::lock;
use super::settings::SettingsStore;
use super::world::WorldService;
use crate::notifier::Notifier;
use crate::world;

/// How often the worldstate and the builds are looked at for something to announce.
const TICK: Duration = Duration::from_secs(60);

pub struct Notifications {
    settings: Arc<SettingsStore>,
    world: Arc<WorldService>,
    accounts: Arc<AccountService>,
    notifier: Mutex<Notifier>,
}

impl Notifications {
    pub fn new(settings: Arc<SettingsStore>, world: Arc<WorldService>, accounts: Arc<AccountService>) -> Notifications {
        Notifications { settings, world, accounts, notifier: Mutex::new(Notifier::default()) }
    }

    /// Looks once a minute while any notification is switched on; nothing is fetched for them
    /// otherwise.
    pub async fn run(self: Arc<Self>, stop: CancellationToken) {
        loop {
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(TICK) => {}
            }
            let s = self.settings.get();
            if !(s.notify_fissures || s.notify_baro || s.notify_foundry) {
                continue;
            }
            let (raw, err) = if s.notify_fissures || s.notify_baro {
                tokio::select! {
                    _ = stop.cancelled() => return,
                    snapshot = self.world.get() => snapshot,
                }
            } else {
                (None, None)
            };
            let me = Arc::clone(&self);
            // Building the list of builds walks every account's inventory: off the async threads.
            let _ = tokio::task::spawn_blocking(move || {
                let builds = if s.notify_foundry { me.accounts.builds() } else { Vec::new() };
                let mut n = lock(&me.notifier);
                if s.notify_fissures {
                    n.fissures(raw.as_deref(), err.as_deref(), &s.fissure_watch);
                }
                if s.notify_baro {
                    if let Some(raw) = raw.as_deref() {
                        n.baro(&world::baro(raw, &crate::gamedata::Catalog::default()));
                    }
                }
                if s.notify_foundry {
                    n.foundry(&builds);
                }
            })
            .await;
        }
    }
}
