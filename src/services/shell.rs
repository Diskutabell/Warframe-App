//! The desktop shell's inbox: what the engine asks of the window, the tray and the overlays.
//! Without a shell (`--headless`, `--browser`) these requests go nowhere.

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

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
    /// The shell is up: from now on its inbox is `tx`. False when the app is quitting already
    /// (a quit that came while the window was still being made): then the shell must not start.
    pub fn attach(&self, tx: async_channel::Sender<UiEvent>, stop: &CancellationToken) -> bool {
        let mut slot = lock(&self.tx);
        if stop.is_cancelled() {
            return false;
        }
        *slot = Some(tx);
        true
    }

    /// Quits: through the shell when there is one (it closes its window, then the engine stops),
    /// else by stopping the engine. Under the same lock as `attach`, so a quit never falls
    /// between the two and leaves a window without an engine.
    pub fn quit(&self, stop: &CancellationToken) {
        let slot = lock(&self.tx);
        match slot.as_ref() {
            Some(tx) => {
                let _ = tx.try_send(UiEvent::Quit);
            }
            None => stop.cancel(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_before_the_window_is_up_keeps_the_window_from_starting() {
        let (shell, stop) = (Shell::default(), CancellationToken::new());
        shell.quit(&stop);
        assert!(stop.is_cancelled(), "no window yet: the engine stops");
        let (tx, _rx) = async_channel::unbounded();
        assert!(!shell.attach(tx, &stop), "and the window does not start");
        // With the window up, a quit goes to it.
        let (shell, stop) = (Shell::default(), CancellationToken::new());
        let (tx, rx) = async_channel::unbounded();
        assert!(shell.attach(tx, &stop));
        shell.quit(&stop);
        assert!(!stop.is_cancelled());
        assert!(matches!(rx.try_recv(), Ok(UiEvent::Quit)));
    }
}
