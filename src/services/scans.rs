//! Reading the screen: the relic reward screen (from the log's open line, or the hotkey) and
//! riven cards (the hotkey), the overlays that show what was read, and the reward history.
//!
//! Capturing and OCR are blocking work and run on the blocking pool, one at a time; prices and
//! riven listings come from the network while the overlays already show the names.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::accounts::AccountService;
use super::catalog::CatalogStore;
use super::gamelog::LogService;
use super::lock;
use super::settings::SettingsStore;
use super::shell::{Shell, UiEvent};
use crate::bus::{Bus, Topic};
use crate::db::Db;
use crate::market::Market;
use crate::ocr::image::Rgb;
use crate::ocr::matcher::RewardMatch;
use crate::ocr::{self, reward, screenshot};
use crate::rivens::{self, Graded};
use crate::{history, notifier, now};

/// A reward scan started this recently already covers the screen: the F9 hotkey and the
/// automatic trigger routinely fire for the same reward screen within a second of each other.
const COALESCE: f64 = 5.0;
/// How long a scan looks for the reward cards: from the log's open line they turn over after a
/// 5 s pause showing the relics, or at once; the hotkey is pressed with them in view.
const REWARD_WAIT: Duration = Duration::from_secs(12);
const HOTKEY_WAIT: Duration = Duration::from_secs(1);
/// A caller joining a running scan waits for it this long at most.
const JOIN_WAIT: Duration = Duration::from_secs(15);
/// Between two looks at the screen while waiting for the cards. A capture with grim takes 20-40 ms;
/// the other screenshot tools start a whole program each time.
const REWARD_POLL: Duration = Duration::from_millis(120);
const SLOW_POLL: Duration = Duration::from_secs(1);
/// Readings of one screen that found names, at most, while a card is still unread or unsure: one
/// that stays so is not worth more.
const REWARD_READS: usize = 3;
/// A card read at this score or better is not read again. On real screens the wrong names (a
/// sibling part on a hovered or tinted card) scored 0.77-0.85, and the same card read 1.00 in
/// the frames before and after.
const SURE: f64 = 0.9;
/// Readings of one screen at most, with or without names: what looked like cards may be none.
const REWARD_ATTEMPTS: usize = 6;
/// While the prices are up, the screen is checked this often for the cards still being there.
const CARDS_CHECK: Duration = Duration::from_millis(400);
/// How long prices are waited for once the names are read: the choice timer is 15 s.
const PRICES_WAIT: Duration = Duration::from_secs(6);

#[derive(Clone, Debug, Serialize)]
pub struct RewardItem {
    pub name: String,
    pub uid: String,
    pub score: f64,
    pub slug: Option<String>,
    pub have: i64,
    pub needed: i64,
    pub vaulted: Option<bool>,
    pub ducats: Option<i64>,
    pub price: Option<i64>,
    /// The card it is on, 0 = leftmost; None when the names were searched for without cards.
    pub slot: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RewardResult {
    pub t: f64,
    pub items: Vec<RewardItem>,
    pub best: Option<String>,
    pub ocr: String,
    pub error: Option<String>,
    /// How many cards the screen showed, when it was recognised as the reward screen.
    pub cards: Option<usize>,
    /// The interface scale the cards were found at, which places the price panels under them.
    pub scale: Option<f64>,
}

impl RewardResult {
    /// What Settings' "show a test overlay" shows: four cards at interface scale `scale`.
    pub fn sample(scale: f64) -> RewardResult {
        let item = |slot, name: &str, price: Option<i64>, ducats: Option<i64>, have, needed, vaulted| RewardItem {
            name: name.into(),
            uid: String::new(),
            score: 1.0,
            slug: None,
            have,
            needed,
            vaulted: Some(vaulted),
            ducats,
            price,
            slot: Some(slot),
        };
        RewardResult {
            t: crate::now(),
            items: vec![
                item(0, "Ash Prime Systems Blueprint", Some(25), Some(65), 0, 1, false),
                item(1, "Braton Prime Barrel", Some(4), Some(15), 2, 0, false),
                item(2, "Forma Blueprint", None, None, 3, 0, false),
                item(3, "Fragor Prime Head", Some(6), Some(45), 0, 0, true),
            ],
            best: Some("Ash Prime Systems Blueprint".into()),
            cards: Some(4),
            scale: Some(scale),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RivenResult {
    pub t: f64,
    pub weapon: Option<String>,
    /// The weapon's picture.
    pub image: Option<String>,
    pub positives: Option<u8>,
    pub negatives: Option<u8>,
    /// Stats the card has but OCR did not read (their count is inferred from the values).
    pub unread: u8,
    pub stats: Vec<Graded>,
    /// The grade of the average roll over all stats.
    pub overall: Option<&'static str>,
    /// What this weapon's rivens sold for lately (api.warframestat.us: `unrolled` and `rerolled`,
    /// each with `median`, `min`, `max`, `pop`).
    pub traded: Option<Value>,
    /// Rivens with the same positives listed on warframe.market; filled in after the grades are
    /// shown, since it is one more request.
    pub market: Option<crate::market::RivenListings>,
    #[serde(rename = "marketError")]
    pub market_error: Option<String>,
    pub ocr: String,
    pub error: Option<String>,
}

/// The pick worth the most platinum, the leftmost card on a tie. With no price known there is no
/// best to claim.
fn best_pick(items: &[RewardItem]) -> Option<String> {
    items
        .iter()
        .filter(|i| i.price.is_some_and(|p| p > 0))
        .fold(None::<&RewardItem>, |best, i| if best.is_none_or(|b| i.price > b.price) { Some(i) } else { best })
        .map(|i| i.name.clone())
}

/// What a look for the reward cards found.
enum Sighting {
    /// The screen closed first.
    Cancelled,
    /// The cards, the best name read on each, the frame and the words they came from.
    Cards { frame: Rgb, layout: reward::Layout, names: Vec<Option<RewardMatch>>, text: String },
    /// No reward cards before the time ran out; the last frame.
    NoCards(Rgb),
}

struct RewardScan {
    started: f64,
    done: bool,
    generation: u64,
    screen_generation: u64,
    result: Option<Arc<RewardResult>>,
}

impl RewardScan {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.screen_generation = self.screen_generation.wrapping_add(1);
        self.started = f64::NEG_INFINITY;
        self.done = true;
        self.result = None;
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    fn finish(&mut self, generation: u64) {
        if self.is_current(generation) {
            self.done = true;
        }
    }

    /// The log opened a new reward screen at `t`; its number. A scan that is reading it already
    /// (the hotkey, pressed before the log line reached the disk) carries on and is joined, as is
    /// one that just read its cards; anything else belongs to an older screen and is called off.
    fn new_screen(&mut self, t: f64) -> u64 {
        let read_cards = self.result.as_ref().is_some_and(|r| r.cards.is_some());
        if !self.done || (t - self.started < COALESCE && read_cards) {
            self.screen_generation = self.screen_generation.wrapping_add(1);
        } else {
            self.invalidate();
        }
        self.screen_generation
    }

    /// Whose turn it is: a scan still running (or just finished) for the same screen is shared,
    /// prices included; otherwise this caller scans. `screen` is the log's screen the caller reads
    /// (None for the hotkey); `joined` says it has waited for a running scan already.
    fn turn(&mut self, screen: Option<u64>, joined: bool, may_wait: bool, t: f64) -> Turn {
        if screen.is_some_and(|screen| screen != self.screen_generation) {
            return Turn::Cancelled;
        }
        let recent = t - self.started < COALESCE;
        if self.done && (joined || recent) {
            // The log's screen wants the cards read; a hotkey caller takes whatever was read.
            if let Some(r) = self.result.as_ref().filter(|r| screen.is_none() || r.cards.is_some()) {
                return Turn::Share(Arc::clone(r));
            }
        }
        if !self.done && may_wait {
            return Turn::Wait;
        }
        self.started = t;
        self.done = false;
        self.generation = self.generation.wrapping_add(1);
        self.result = None;
        Turn::Scan(self.generation)
    }
}

/// What a caller asking for a reward scan does: share a result, give up, wait, or scan.
enum Turn {
    Share(Arc<RewardResult>),
    Cancelled,
    Wait,
    Scan(u64),
}

pub struct ScanService {
    db: Arc<Db>,
    settings: Arc<SettingsStore>,
    catalog: Arc<CatalogStore>,
    accounts: Arc<AccountService>,
    market: Arc<Market>,
    shell: Arc<Shell>,
    log: Arc<LogService>,
    bus: Bus,
    rewards: Mutex<Option<Arc<RewardResult>>>,
    riven: Mutex<Option<Arc<RivenResult>>>,
    /// One capture and OCR at a time, rewards and rivens alike.
    scan_lock: tokio::sync::Mutex<()>,
    reward_scan: Mutex<RewardScan>,
    /// Bumped whenever a reward scan ends or is called off, waking the callers sharing it.
    reward_done: watch::Sender<u64>,
    /// The interface scale the reward cards were last found at when it was not EE.cfg's.
    found_scale: Mutex<Option<f64>>,
}

/// Runs blocking work (screen capture, OCR, files) on the blocking pool.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|e| anyhow!("the scan hit a bug (see the app's log): {e}"))
}

/// Marks the running reward scan finished, however it ends, and wakes callers sharing it.
struct ScanFinished<'a>(&'a ScanService, u64);

impl Drop for ScanFinished<'_> {
    fn drop(&mut self) {
        lock(&self.0.reward_scan).finish(self.1);
        self.0.reward_done.send_modify(|n| *n = n.wrapping_add(1));
    }
}

fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

fn matches_json(r: &RewardResult) -> Value {
    let items: Vec<Value> =
        r.items.iter().map(|i| json!({"name": i.name, "uid": i.uid, "score": i.score, "slot": i.slot})).collect();
    json!({"cards": r.cards, "scale": r.scale, "items": items})
}

impl ScanService {
    // The scans read the screen for the accounts, price for the market and draw through the
    // shell: every one of these is a service of its own.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<Db>,
        settings: Arc<SettingsStore>,
        catalog: Arc<CatalogStore>,
        accounts: Arc<AccountService>,
        market: Arc<Market>,
        shell: Arc<Shell>,
        log: Arc<LogService>,
        bus: &Bus,
    ) -> ScanService {
        ScanService {
            db,
            settings,
            catalog,
            accounts,
            market,
            shell,
            log,
            bus: bus.clone(),
            rewards: Mutex::new(None),
            riven: Mutex::new(None),
            scan_lock: tokio::sync::Mutex::new(()),
            reward_scan: Mutex::new(RewardScan {
                started: f64::NEG_INFINITY,
                done: true,
                generation: 0,
                screen_generation: 0,
                result: None,
            }),
            reward_done: watch::channel(0).0,
            found_scale: Mutex::new(None),
        }
    }

    pub fn rewards(&self) -> Option<Arc<RewardResult>> {
        lock(&self.rewards).clone()
    }

    pub fn riven(&self) -> Option<Arc<RivenResult>> {
        lock(&self.riven).clone()
    }

    /// The interface scales to look for the reward cards at: EE.cfg's, then the one they were
    /// last found at if that differs (the game may write a changed scale only when it exits).
    /// Empty when neither is known: then every scale is tried.
    pub fn ui_scales(&self) -> Vec<f64> {
        let cfg = self.log.ui_scale();
        let found = *lock(&self.found_scale);
        cfg.into_iter().chain(found.filter(|f| cfg.is_none_or(|c| (c - f).abs() > 1e-6))).collect()
    }

    // ------------------------------------------------------------ the log's reward screen

    /// The log opened a reward screen: read it once its cards are up.
    ///
    /// The log reaches the disk late (up to seconds), so the hotkey may have read this very screen
    /// already, or be reading it. Such a scan is not called off: the automatic one joins it and
    /// shares its result when that found the cards, and reads the screen itself when not (the
    /// hotkey was pressed before the cards turned over).
    pub fn on_reward_screen(self: &Arc<Self>) {
        if !self.settings.get().ocr_enabled {
            return;
        }
        let screen = lock(&self.reward_scan).new_screen(now());
        self.reward_done.send_modify(|n| *n = n.wrapping_add(1));
        let me = Arc::clone(self);
        tokio::spawn(async move {
            me.scan_rewards_for(REWARD_WAIT, Some(screen)).await;
        });
    }

    /// The log closed the reward screen: its prices go, and a scan still looking gives up.
    pub fn on_reward_screen_closed(&self) {
        lock(&self.reward_scan).invalidate();
        self.shell.send(UiEvent::HideRewards);
        self.reward_done.send_modify(|n| *n = n.wrapping_add(1));
    }

    // ------------------------------------------------------------ reward scans

    /// Hides our overlays and waits until they are off screen, so a capture never reads them.
    async fn hide_overlays_for_capture(&self) {
        if !self.shell.present() {
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.shell.send(UiEvent::HideForCapture(tx));
        if tokio::time::timeout(Duration::from_millis(300), rx).await.is_ok_and(|r| r.is_ok()) {
            // The compositor needs a frame to take the surface down.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn publish_rewards(&self, result: RewardResult, fresh: bool, generation: u64) -> Arc<RewardResult> {
        let result = Arc::new(result);
        let mut scan = lock(&self.reward_scan);
        if scan.is_current(generation) {
            scan.result = Some(Arc::clone(&result));
            *lock(&self.rewards) = Some(Arc::clone(&result));
            self.shell.send(UiEvent::Rewards { result: Arc::clone(&result), fresh });
            self.bus.publish(Topic::Rewards, &*result);
        }
        result
    }

    /// Screenshots the reward screen, reads the names, and prices them (the hotkey and the API).
    pub async fn scan_rewards(self: &Arc<Self>) -> Arc<RewardResult> {
        match self.scan_rewards_for(HOTKEY_WAIT, None).await {
            Some(result) => result,
            // A hotkey scan has no screen to be called off with; only a bug ends it without one.
            None => {
                Arc::new(RewardResult { t: now(), error: Some("The scan was cancelled.".into()), ..Default::default() })
            }
        }
    }

    /// Whose turn it is: a scan still running (or just finished) for the same screen is shared,
    /// prices included; otherwise this caller scans.
    fn turn(&self, screen: Option<u64>, joined: bool, deadline: Instant) -> Turn {
        lock(&self.reward_scan).turn(screen, joined, Instant::now() < deadline, now())
    }

    /// Reads the reward screen: `screen` is the log's reward screen this scan belongs to (None for
    /// the hotkey), and the scan gives up quietly once that screen has closed.
    async fn scan_rewards_for(self: &Arc<Self>, wait: Duration, screen: Option<u64>) -> Option<Arc<RewardResult>> {
        let deadline = Instant::now() + JOIN_WAIT;
        let mut done = self.reward_done.subscribe();
        let mut joined = false;
        let generation = loop {
            match self.turn(screen, joined, deadline) {
                Turn::Share(result) => return Some(result),
                Turn::Cancelled => return None,
                Turn::Scan(generation) => break generation,
                Turn::Wait => {
                    joined = true;
                    let _ = tokio::time::timeout_at(deadline.into(), done.changed()).await;
                }
            }
        };
        let _finished = ScanFinished(self, generation);
        let capture = self.scan_lock.lock().await;
        let notify = self.settings.get().notify;
        self.hide_overlays_for_capture().await;

        let me = Arc::clone(self);
        let looked = blocking(move || me.find_cards(wait, screen, generation)).await;
        let (sighting, shown) = match looked {
            Ok((Ok(sighting), shown)) => (sighting, shown),
            Ok((Err(e), _)) => return Some(self.reward_scan_failed(&e, generation)),
            Err(e) => return Some(self.reward_scan_failed(&e, generation)),
        };
        let (frame, layout, names, text) = match sighting {
            Sighting::Cancelled => return None,
            Sighting::Cards { frame, layout, names, text } if names.iter().any(Option::is_some) => {
                (frame, Some(layout), names, text)
            }
            Sighting::Cards { frame, .. } | Sighting::NoCards(frame) if screen.is_some() => {
                // The log announced the rewards, but no card with a readable name showed up:
                // there is nothing to put over the game. The frame is kept to find out why.
                self.accounts.event("Reward screen: no reward card could be read on screen", "warn");
                drop(capture);
                ocr::archive("reward", frame, String::new(), json!({"cards": null, "items": []}));
                return None;
            }
            Sighting::Cards { frame, .. } | Sighting::NoCards(frame) => {
                // The hotkey on a screen without readable cards (another theme, a scale they
                // were not found at): look for names anywhere in the middle of the screen.
                let names = self.catalog.names();
                let read = blocking(move || {
                    let found = ocr::read_rewards(&frame, &names);
                    (frame, found)
                })
                .await;
                match read {
                    Ok((frame, Ok((found, text)))) => (frame, None, found.into_iter().map(Some).collect(), text),
                    Ok((_, Err(e))) | Err(e) => return Some(self.reward_scan_failed(&e, generation)),
                }
            }
        };
        if self.cancelled(screen, generation) {
            return None;
        }
        if let Some(layout) = &layout {
            if !self.ui_scales().iter().any(|s| (s - layout.ui_scale).abs() < 1e-6) {
                *lock(&self.found_scale) = Some(layout.ui_scale);
            }
        }
        let me = Arc::clone(self);
        let ocr_text = tail(&text, 800);
        let (account, base) = blocking(move || me.reward_result(&names, layout.as_ref(), ocr_text)).await.ok()?;
        self.publish_rewards(base.clone(), !shown, generation);
        // The screen is read; a riven scan may run while the prices load.
        drop(capture);
        ocr::archive("reward", frame, text, matches_json(&base));
        if let Some(layout) = layout {
            self.watch_cards(layout, generation);
        }

        let mut items = base.items.clone();
        let slugs: Vec<Option<String>> = items.iter().map(|i| i.slug.clone()).collect();
        let prices = self.market.fetch_now(&slugs, PRICES_WAIT).await;
        for it in &mut items {
            if let Some(sell) = it.slug.as_ref().and_then(|s| prices.get(s)).and_then(|p| p.sell) {
                it.price = Some(sell);
            }
        }
        let best = best_pick(&items);
        // The history keeps what a reward screen offered, even when the screen has closed in the
        // meantime; names found without cards may be another screen's, so they are not kept.
        if layout.is_some() && !items.is_empty() {
            let entry = history::Entry {
                t: now(),
                drops: items
                    .iter()
                    .map(|i| history::Reward {
                        name: i.name.clone(),
                        uid: i.uid.clone(),
                        price: i.price,
                        ducats: i.ducats,
                        needed: i.needed > i.have,
                    })
                    .collect(),
                best: best.clone(),
                account,
            };
            let db = Arc::clone(&self.db);
            if let Ok(Err(e)) = blocking(move || db.record_reward(&entry)).await {
                eprintln!("could not record the reward screen: {e:#}");
            }
            self.bus.signal(Topic::History);
            self.bus.state_changed();
        }
        if self.cancelled(screen, generation) {
            return None;
        }
        if !items.is_empty() && notify {
            let lines: Vec<String> = items
                .iter()
                .map(|i| {
                    let star = if best.as_deref() == Some(i.name.as_str()) { "★ " } else { "" };
                    let price = i.price.map_or("?".to_string(), |p| p.to_string());
                    let need = if i.needed > i.have { " · need" } else { "" };
                    format!("{star}{}: {price}p{need}", i.name)
                })
                .collect();
            notifier::send("Relic rewards", &lines.join("\n"));
        }
        Some(self.publish_rewards(RewardResult { items, best, ..base }, false, generation))
    }

    /// Whether the log's reward screen this scan belongs to has closed (or a newer scan took over).
    fn cancelled(&self, screen: Option<u64>, generation: u64) -> bool {
        screen.is_some() && !lock(&self.reward_scan).is_current(generation)
    }

    fn reward_scan_failed(&self, e: &anyhow::Error, generation: u64) -> Arc<RewardResult> {
        self.accounts.event(&format!("Reward scan failed: {e}"), "warn");
        self.publish_rewards(
            RewardResult { t: now(), error: Some(e.to_string()), ..Default::default() },
            true,
            generation,
        )
    }

    /// Looks at the screen until the reward cards are up and read, or `wait` runs out. Blocking:
    /// runs on the blocking pool.
    ///
    /// The scan starts at the log's open line, seconds before the cards turn over, so the screen is
    /// captured every `REWARD_POLL` until the cards' rarity bars show. A card may still be fading
    /// in then: what is read is published at once and the screen read again, up to
    /// `REWARD_READS` times, each card keeping the best name any reading gave it. The second value
    /// says whether a partial reading was published.
    fn find_cards(&self, wait: Duration, screen: Option<u64>, generation: u64) -> (Result<Sighting>, bool) {
        let mut shown = false;
        let found = self.look_for_cards(wait, screen, generation, &mut shown);
        (found, shown)
    }

    fn look_for_cards(
        &self,
        wait: Duration,
        screen: Option<u64>,
        generation: u64,
        shown: &mut bool,
    ) -> Result<Sighting> {
        let tool = self.settings.get().screenshot_tool;
        let names = self.catalog.names();
        let scales = self.ui_scales();
        let deadline = Instant::now() + wait;
        let poll = if screenshot::cheap(tool) { REWARD_POLL } else { SLOW_POLL };
        let (mut reads, mut attempts) = (0, 0);
        let mut seen: Option<(Rgb, reward::Layout, Vec<Option<RewardMatch>>, String)> = None;
        loop {
            if self.cancelled(screen, generation) {
                return Ok(Sighting::Cancelled);
            }
            let frame = screenshot::capture(tool)?;
            let late = Instant::now() >= deadline;
            // A scale that EE.cfg does not know, or knows wrong: every scale, at the end.
            let layout = reward::find_layout(&frame, &scales)
                .or_else(|| if late && !scales.is_empty() { reward::find_layout(&frame, &[]) } else { None });
            let Some(layout) = layout else {
                if late {
                    return Ok(match seen {
                        Some((frame, layout, names, text)) => Sighting::Cards { frame, layout, names, text },
                        None => Sighting::NoCards(frame),
                    });
                }
                std::thread::sleep(poll);
                continue;
            };
            let read = reward::read(&frame, &layout, &names)?;
            attempts += 1;
            if read.names.iter().any(Option::is_some) {
                reads += 1;
            }
            let (mut merged, before) = match seen.take() {
                Some((_, l, names, _)) if l.cards == layout.cards => (names.clone(), names),
                _ => (vec![None; layout.cards], vec![None; layout.cards]),
            };
            for (slot, new) in merged.iter_mut().zip(read.names) {
                if let Some(new) = new {
                    if slot.as_ref().is_none_or(|old| reward::better(&new, old)) {
                        *slot = Some(new);
                    }
                }
            }
            let sure = merged.iter().all(|m| m.as_ref().is_some_and(|m| m.score >= SURE));
            if late || sure || reads >= REWARD_READS || attempts >= REWARD_ATTEMPTS {
                return Ok(Sighting::Cards { frame, layout, names: merged, text: read.text });
            }
            if merged != before {
                // Names are shown as soon as a card is read; the rest of them, and the prices,
                // follow in place. The choice timer is 15 s and warframe.market can take seconds.
                let (_, result) = self.reward_result(&merged, Some(&layout), String::new());
                self.publish_rewards(result, !*shown, generation);
                *shown = true;
            }
            seen = Some((frame, layout, merged, read.text));
            std::thread::sleep(poll);
        }
    }

    /// The reward items for what the cards say, with everything known without asking the
    /// market: counts, needs, vaulted, ducats and the cached price. Owned and needed are the
    /// playing account's, whichever account the window shows; that account is returned too.
    fn reward_result(
        &self,
        names: &[Option<RewardMatch>],
        layout: Option<&reward::Layout>,
        ocr: String,
    ) -> (Option<String>, RewardResult) {
        let (account, v) = self.accounts.game_view();
        let needed = v.needed_parts();
        let items: Vec<RewardItem> = names
            .iter()
            .enumerate()
            .filter_map(|(k, m)| {
                let m = m.as_ref()?;
                let part = v.cat.parts.get(v.cat.part_alias.get(&m.uid).unwrap_or(&m.uid));
                let slug = self.market.slug_for(Some(&m.uid), part.and_then(|p| p.slug.as_deref()), Some(&m.name));
                Some(RewardItem {
                    name: m.name.clone(),
                    uid: m.uid.clone(),
                    score: m.score,
                    have: v.count(&m.uid),
                    needed: needed.get(&m.uid).copied().unwrap_or(0),
                    vaulted: part.map(|p| p.vaulted),
                    ducats: self.market.ducats(slug.as_deref()),
                    price: slug.as_deref().and_then(|s| self.market.cached(s)).and_then(|p| p.sell),
                    slug,
                    slot: layout.map(|_| k),
                })
            })
            .collect();
        let result = RewardResult {
            t: now(),
            best: best_pick(&items),
            ocr,
            error: items.is_empty().then(|| "No reward names recognized.".to_string()),
            cards: layout.map(|l| l.cards),
            scale: layout.map(|l| l.ui_scale),
            items,
        };
        (account, result)
    }

    /// Hides the prices as soon as the cards leave the screen. The log says so too ("Relic reward
    /// screen shut down"), but a little later, and a line can be missed; this looks for the cards'
    /// bars every `CARDS_CHECK` and hides after two looks without them. Only with a screenshot
    /// tool that is quick and silent.
    fn watch_cards(self: &Arc<Self>, layout: reward::Layout, generation: u64) {
        let tool = self.settings.get().screenshot_tool;
        if !screenshot::cheap(tool) {
            return;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let until = Instant::now() + Duration::from_secs(40);
            let mut misses = 0;
            let layout = Arc::new(layout);
            while Instant::now() < until {
                tokio::time::sleep(CARDS_CHECK).await;
                if !lock(&me.reward_scan).is_current(generation) {
                    return;
                }
                let shape = Arc::clone(&layout);
                let up =
                    blocking(move || screenshot::capture(tool).map(|frame| reward::still_up(&frame, &shape))).await;
                let Ok(Ok(up)) = up else { return };
                misses = if up { 0 } else { misses + 1 };
                if misses >= 2 {
                    let mut scan = lock(&me.reward_scan);
                    if scan.is_current(generation) {
                        scan.invalidate();
                        drop(scan);
                        me.reward_done.send_modify(|n| *n = n.wrapping_add(1));
                        me.shell.send(UiEvent::HideRewards);
                    }
                    return;
                }
            }
        });
    }

    // ------------------------------------------------------------ rivens

    fn riven_failed(&self, weapon: Option<String>, text: &str, error: String) -> Arc<RivenResult> {
        let r =
            Arc::new(RivenResult { t: now(), weapon, ocr: tail(text, 600), error: Some(error), ..Default::default() });
        *lock(&self.riven) = Some(Arc::clone(&r));
        self.bus.publish(Topic::Riven, &*r);
        if self.settings.get().riven_overlay_enabled {
            self.shell.send(UiEvent::Riven { result: Arc::clone(&r), fresh: true });
        }
        r
    }

    /// Reads the riven card currently on screen and grades it.
    pub async fn scan_riven(self: &Arc<Self>) -> Arc<RivenResult> {
        let _one_scan_at_a_time = self.scan_lock.lock().await;
        let settings = self.settings.get();
        self.hide_overlays_for_capture().await;
        let cat = self.catalog.get();
        let scales = self.ui_scales();
        let weapons = cat.riven_weapons();
        let read = blocking(move || {
            screenshot::capture(settings.screenshot_tool).and_then(|frame| {
                let (r, text) = ocr::read_riven(&frame, &weapons, &scales)?;
                Ok((frame, r, text))
            })
        })
        .await
        .and_then(|r| r);
        let (frame, read, text) = match read {
            Ok(r) => r,
            Err(e) => return self.riven_failed(None, "", e.to_string()),
        };
        ocr::archive("riven", frame, text.clone(), json!({"weapon": read.weapon, "stats": read.stats}));
        let Some(weapon) = read.weapon.clone() else {
            return self.riven_failed(
                None,
                &text,
                "No weapon name recognised. Open the riven so its card is visible.".into(),
            );
        };
        if read.stats.is_empty() {
            return self.riven_failed(Some(weapon), &text, "No stat lines recognised on screen.".into());
        }
        let Some(it) = cat.item_by_name(&weapon).filter(|i| i.disposition.is_some_and(|d| d != 0.0)) else {
            return self.riven_failed(Some(weapon.clone()), &text, format!("No riven data for {weapon}."));
        };
        // 2-3 positives and 0-1 negative; a stat line OCR lost is recovered from the values.
        let readings: Vec<(&str, f64, bool)> = read.stats.iter().map(|s| (s.stat, s.value, s.negative)).collect();
        let (pos, neg, unread) = rivens::infer_counts(it, &readings);
        let stats: Vec<Graded> =
            read.stats.iter().filter_map(|s| rivens::grade_roll(it, s.stat, s.value, pos, neg, s.negative)).collect();
        let overall = (!stats.is_empty())
            .then(|| rivens::grade_for(stats.iter().map(|g| g.deviation).sum::<f64>() / stats.len() as f64));
        // From the cache, read off the async threads; a stale copy is refreshed behind it.
        let (name, platform) = (it.name.clone(), settings.platform);
        let traded = blocking(move || rivens::traded(&name, platform)).await.ok().flatten();
        let r = Arc::new(RivenResult {
            t: now(),
            weapon: Some(it.name.clone()),
            image: crate::gamedata::image_url(it.image.as_deref()),
            positives: Some(pos),
            negatives: Some(neg),
            unread,
            stats,
            overall,
            traded,
            ocr: tail(&text, 600),
            ..Default::default()
        });
        *lock(&self.riven) = Some(Arc::clone(&r));
        self.bus.publish(Topic::Riven, &*r);
        if settings.riven_overlay_enabled {
            self.shell.send(UiEvent::Riven { result: Arc::clone(&r), fresh: true });
        }
        self.find_riven_listings(&r, it.uid.clone());
        r
    }

    /// Looks up rivens like `r` on warframe.market in the background and updates the result (and
    /// the panel, if it is still up) when the answer comes.
    fn find_riven_listings(self: &Arc<Self>, r: &Arc<RivenResult>, uid: String) {
        let positives: Vec<&'static str> =
            r.stats.iter().filter(|g| !g.negative).filter_map(|g| rivens::market_slug(g.stat)).collect();
        let (Some(weapon), false) = (r.weapon.clone(), positives.is_empty()) else { return };
        let (me, r) = (Arc::clone(self), Arc::clone(r));
        tokio::spawn(async move {
            // The panel shows the weapon's picture only from the art cache.
            if let Some(url) = r.image.clone() {
                let _ = crate::art::get(&url, 128).await;
            }
            let mut updated = (*r).clone();
            match me.market.riven_listings(&uid, &weapon, &positives).await {
                Ok(found) => updated.market = Some(found),
                Err(e) => updated.market_error = Some(format!("{e:#}")),
            }
            let updated = Arc::new(updated);
            {
                let mut current = lock(&me.riven);
                // A newer riven was read meanwhile: this answer belongs to the old one.
                if current.as_ref().is_none_or(|c| c.t != r.t) {
                    return;
                }
                *current = Some(Arc::clone(&updated));
            }
            me.bus.publish(Topic::Riven, &*updated);
            if me.settings.get().riven_overlay_enabled {
                me.shell.send(UiEvent::Riven { result: updated, fresh: false });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_reward_scan_cannot_finish_the_newer_scan() {
        let mut scan = RewardScan { started: 10.0, done: false, generation: 2, screen_generation: 0, result: None };
        assert!(!scan.is_current(1));
        scan.finish(1);
        assert!(!scan.done);
        assert!(scan.is_current(2));
        scan.finish(2);
        assert!(scan.done);
    }

    fn idle() -> RewardScan {
        RewardScan { started: f64::NEG_INFINITY, done: true, generation: 0, screen_generation: 0, result: None }
    }

    fn found(cards: Option<usize>) -> Arc<RewardResult> {
        Arc::new(RewardResult { cards, ..RewardResult::default() })
    }

    #[test]
    fn the_logs_screen_joins_a_hotkey_scan_that_read_the_cards() {
        let mut scan = idle();
        // F9 at t=100, before the log's open line reached the disk.
        let Turn::Scan(hotkey) = scan.turn(None, false, true, 100.0) else { panic!("the hotkey scans") };
        // The open line arrives at t=101 while it reads: it is not called off.
        let screen = scan.new_screen(101.0);
        assert!(scan.is_current(hotkey));
        assert!(matches!(scan.turn(Some(screen), false, true, 101.0), Turn::Wait));
        // It finishes with the cards read: the log's scan shares it.
        scan.result = Some(found(Some(4)));
        scan.finish(hotkey);
        assert!(matches!(scan.turn(Some(screen), true, true, 108.0), Turn::Share(r) if r.cards == Some(4)));
    }

    #[test]
    fn a_hotkey_pressed_before_the_cards_does_not_stand_in_for_them() {
        let mut scan = idle();
        let Turn::Scan(hotkey) = scan.turn(None, false, true, 100.0) else { panic!("the hotkey scans") };
        let screen = scan.new_screen(100.5);
        // The hotkey looked before the cards turned over and found none.
        scan.result = Some(found(None));
        scan.finish(hotkey);
        assert!(matches!(scan.turn(Some(screen), true, true, 102.0), Turn::Scan(_)), "the log's scan reads them");
        // A screen opening long after a scan is a new screen: the old scan is called off.
        let mut old = idle();
        let Turn::Scan(g) = old.turn(None, false, true, 100.0) else { panic!() };
        old.result = Some(found(Some(4)));
        old.finish(g);
        let screen = old.new_screen(400.0);
        assert!(matches!(old.turn(Some(screen), false, true, 400.0), Turn::Scan(_)));
    }

    #[test]
    fn closing_reward_screen_invalidates_pending_and_running_scans() {
        let mut scan = RewardScan { started: 10.0, done: false, generation: 2, screen_generation: 5, result: None };
        scan.invalidate();
        assert_ne!(scan.screen_generation, 5);
        assert!(!scan.is_current(2));
        assert!(scan.done);
        assert_eq!(scan.started, f64::NEG_INFINITY);
        scan.finish(2);
        assert!(scan.done);
    }

    #[test]
    fn the_best_pick_is_the_most_platinum_and_the_leftmost_on_a_tie() {
        let item = |name: &str, price| RewardItem {
            name: name.into(),
            uid: String::new(),
            score: 1.0,
            slug: None,
            have: 0,
            needed: 0,
            vaulted: None,
            ducats: None,
            price,
            slot: None,
        };
        assert_eq!(best_pick(&[item("a", Some(5)), item("b", Some(9)), item("c", Some(9))]).as_deref(), Some("b"));
        assert_eq!(best_pick(&[item("a", None), item("b", Some(0))]), None);
    }
}
