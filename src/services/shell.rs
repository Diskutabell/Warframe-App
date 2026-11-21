//! The desktop shell's inbox: what the engine asks of the window, the tray and the overlays.
//! Without a shell (`--headless`, `--browser`) these requests go nowhere.

use std::sync::{Arc, Mutex};

use super::lock;
use super::scans::{RewardResult, RivenResult};

/// What the engine asks of the desktop shell, if one is running.
pub enum UiEvent {
    ShowWindow,
    HideWindow,
    ToggleWindow,
    TrayEnabled(bool),
    /// New rewards; `fresh` is false when only the prices changed.
    Rewards {
        result: Arc<RewardResult>,
        fresh: bool,
    },
    HideRewards,
    /// A riven read; `fresh` is false when only its market listings arrived.
    Riven {
        result: Arc<RivenResult>,
        fresh: bool,
    },
    TestOverlay,
    /// Hide the overlays before a screenshot, so a scan never reads our own text; acknowledged
    /// on the sender once they are gone.
    HideForCapture(tokio::sync::oneshot::Sender<()>),
    Quit,
}

#[derive(Default)]
pub struct Shell {
    tx: Mutex<Option<async_channel::Sender<UiEvent>>>,
}

impl Shell {
    /// The shell is up: from now on its inbox is `tx`.
    pub fn attach(&self, tx: async_channel::Sender<UiEvent>) {
        *lock(&self.tx) = Some(tx);
    }

    /// Whether a desktop shell is running.
    pub fn present(&self) -> bool {
        lock(&self.tx).is_some()
    }

    /// Hands `ev` to the shell. The inbox is unbounded, so this never waits.
    pub fn send(&self, ev: UiEvent) {
        if let Some(tx) = lock(&self.tx).as_ref() {
            let _ = tx.try_send(ev);
        }
    }
}
