//! The API's routes. GET answers read the engine; POST changes it. Every answer is JSON.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use super::{
    ApiError, ApiResult, Engine, ErrorAnswer, JsonBody, MAX_BODY, MAX_IMPORT_BODY, Query, Typed, blocking,
    json_response, ok_json,
};
use crate::UserError;

type Body = JsonBody<MAX_BODY>;
type Import = JsonBody<MAX_IMPORT_BODY>;

pub fn api(engine: Engine, stop: CancellationToken) -> Router<Engine> {
    // What needs the game data: while it is still downloading, these say so instead.
    let catalog = Router::new()
        .route("/api/mastery", get(|s: State<Engine>| view(s, |e| Ok(e.accounts.view().mastery()))))
        .route("/api/primes", get(|s: State<Engine>| view(s, |e| Ok(e.accounts.view().primes()))))
        .route("/api/inventory", get(|s: State<Engine>| view(s, |e| Ok(e.accounts.view().inventory()))))
        .route("/api/foundry", get(|s: State<Engine>| view(s, |e| Ok(e.accounts.view().foundry()))))
        .route("/api/relics", get(relics))
        .route("/api/trade", get(|s: State<Engine>| view(s, |e| Ok(e.trade()))))
        .route("/api/find", get(find))
        .route("/api/find-task", get(find_task))
        .route("/api/tasks", get(|s: State<Engine>| view(s, |e| Ok(crate::tasks::board(&e.accounts.view())))))
        .route("/api/task", get(task))
        .route("/api/item", get(item))
        .route("/api/history", get(|s: State<Engine>| view(s, |e| Ok(e.history()))))
        .route("/api/riven", get(riven_table))
        .route("/api/rivens", get(|State(e): State<Engine>| async move { answer(e.rivens().await) }))
        .route("/api/fissures", get(|State(e): State<Engine>| async move { answer(e.fissures().await) }))
        .route("/api/standing", get(|State(e): State<Engine>| async move { answer(e.standing().await) }))
        .route("/api/syndicate", get(syndicate))
        .route("/api/search", get(search))
        .route("/api/price-history", get(price_history))
        .route_layer(middleware::from_fn_with_state(engine, loading_guard));

    let events_stop = stop.clone();
    Router::new()
        .merge(catalog)
        .route("/api/state", get(|s: State<Engine>| view(s, |e| Ok(e.summary()))))
        .route("/api/settings", get(|s: State<Engine>| view(s, |e| Ok(e.settings()))))
        .route("/api/world", get(|State(e): State<Engine>| async move { answer(e.world_view().await) }))
        // Settings shows the hotkeys before the game data is there.
        .route("/api/hotkeys", get(|s: State<Engine>| view(s, |e| Ok(e.hotkeys()))))
        // The last reward screen and riven read; null before the first.
        .route("/api/rewards", get(|s: State<Engine>| view(s, |e| Ok(e.rewards().map(|r| (*r).clone())))))
        .route("/api/riven/result", get(|s: State<Engine>| view(s, |e| Ok(e.riven().map(|r| (*r).clone())))))
        .route("/api/art", get(super::art::serve))
        .route(
            "/api/events",
            get(move |s: State<Engine>, headers: axum::http::HeaderMap| {
                super::events::stream(s, headers, events_stop.clone())
            }),
        )
        // Changes.
        .route(
            "/api/settings",
            post(|State(e): State<Engine>, JsonBody(b): Body| async move { done(e.update_settings(b).await) }),
        )
        .route(
            "/api/account/set",
            post(|s: State<Engine>, Typed(b): Typed<AccountId>| {
                change(s, move |e| e.accounts.set_account_id(&b.id, b.key.as_deref()))
            }),
        )
        .route(
            "/api/account/switch",
            post(|s: State<Engine>, Typed(b): Typed<AccountKey>| change(s, move |e| e.accounts.switch_account(&b.key))),
        )
        .route(
            "/api/account/forget",
            post(|s: State<Engine>, Typed(b): Typed<AccountKey>| change(s, move |e| e.accounts.forget_account(&b.key))),
        )
        .route("/api/profile/refresh", post(profile_refresh))
        .route(
            "/api/profile/import",
            post(|s: State<Engine>, JsonBody(b): Import| change(s, move |e| e.accounts.import_profile(&b))),
        )
        .route(
            "/api/inventory/import",
            post(|s: State<Engine>, JsonBody(b): Import| {
                change(s, move |e| e.accounts.import_inventory(&b, "imported file").map(|_| ()))
            }),
        )
        .route(
            "/api/inventory/clear",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.accounts.clear_inventory();
                    Ok(())
                })
            }),
        )
        .route(
            "/api/manual/count",
            post(|s: State<Engine>, Typed(b): Typed<SetCount>| {
                change(s, move |e| e.accounts.set_manual_count(&b.uid, b.count.map(|c| c as i64)))
            }),
        )
        .route(
            "/api/manual/pin",
            post(|s: State<Engine>, Typed(b): Typed<SetPinned>| {
                change(s, move |e| e.accounts.set_pinned(&b.uid, b.pinned))
            }),
        )
        .route("/api/manual/task", post(manual_task))
        .route(
            "/api/history/pick",
            post(|s: State<Engine>, Typed(b): Typed<Pick>| {
                change(s, move |e| e.accounts.pick_reward(b.id, b.name.as_deref()))
            }),
        )
        .route(
            "/api/foundry/built",
            post(|s: State<Engine>, Typed(b): Typed<ByUid>| change(s, move |e| e.accounts.mark_built(&b.uid))),
        )
        .route(
            "/api/manual/mastered",
            post(|s: State<Engine>, Typed(b): Typed<SetMastered>| {
                change(s, move |e| e.accounts.set_manual_mastered(&b.uid, b.mastered))
            }),
        )
        .route(
            "/api/tasks/clear",
            post(|s: State<Engine>, _: Body| view(s, |e| Ok(Removed { removed: e.accounts.clear_done_tasks()? }))),
        )
        .route(
            "/api/rewards/scan",
            post(|State(e): State<Engine>, _: Body| async move { answer(Ok((*e.scans.scan_rewards().await).clone())) }),
        )
        .route(
            "/api/riven/scan",
            post(|State(e): State<Engine>, _: Body| async move { answer(Ok((*e.scans.scan_riven().await).clone())) }),
        )
        .route(
            "/api/riven/grade",
            post(|s: State<Engine>, Typed(b): Typed<crate::app::GradeRequest>| view(s, move |e| e.grade_riven(&b))),
        )
        .route(
            "/api/window/show",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.show_window();
                    Ok(())
                })
            }),
        )
        .route(
            "/api/window/hide",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.hide_window();
                    Ok(())
                })
            }),
        )
        .route(
            "/api/window/toggle",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.toggle_window();
                    Ok(())
                })
            }),
        )
        .route(
            "/api/quit",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.quit();
                    Ok(())
                })
            }),
        )
        .route("/api/overlay/test", post(|s: State<Engine>, _: Body| change(s, |e| e.test_overlay())))
        .route("/api/hotkeys/register", post(|s: State<Engine>, _: Body| view(s, |e| e.install_hotkeys())))
        .route("/api/data/reload", post(|State(e): State<Engine>, _: Body| async move { done(e.reload_data().await) }))
        .route("/api/{*rest}", get(unknown).post(unknown))
        .method_not_allowed_fallback(|| async {
            json_response(StatusCode::METHOD_NOT_ALLOWED, &ErrorAnswer::new("method not allowed"))
        })
}

/// A change's answer.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Done {
    pub ok: bool,
}

/// What the pages that need the game data answer while it is still downloading.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Loading {
    pub loading: bool,
    pub error: Option<String>,
}

/// `/api/tasks/clear`: how many finished tasks went.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Removed {
    pub removed: usize,
}

/// `/api/riven` for a weapon without riven data: said in place, not as a failed request.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(untagged)]
pub enum RivenTableAnswer {
    Table(crate::app::RivenTable),
    None { error: String },
}

/// A read: `work` runs on the blocking pool, its value is the answer.
async fn view<T: Serialize + Send + 'static>(
    State(engine): State<Engine>,
    work: impl FnOnce(&Engine) -> anyhow::Result<T> + Send + 'static,
) -> ApiResult {
    let value = blocking(move || work(&engine)).await?;
    Ok(ok_json(&value))
}

/// A change with nothing to say back: `{"ok": true}`.
async fn change(
    State(engine): State<Engine>,
    work: impl FnOnce(&Engine) -> anyhow::Result<()> + Send + 'static,
) -> ApiResult {
    blocking(move || work(&engine)).await?;
    Ok(ok_json(&Done { ok: true }))
}

/// An answer computed by async engine work.
fn answer<T: Serialize>(result: anyhow::Result<T>) -> ApiResult {
    Ok(ok_json(&result?))
}

/// A finished async change: `{"ok": true}`.
fn done(result: anyhow::Result<()>) -> ApiResult {
    result?;
    Ok(ok_json(&Done { ok: true }))
}

/// The pages that need the game data say "loading" until it is there.
async fn loading_guard(State(engine): State<Engine>, req: Request, next: Next) -> Response {
    if !engine.catalog.ready() {
        return ok_json(&Loading { loading: true, error: engine.catalog.error() });
    }
    next.run(req).await
}

async fn unknown(req: Request) -> ApiError {
    let path = req.uri().path().trim_start_matches("/api/").to_string();
    UserError::new(format!("unknown endpoint {path}")).into()
}

async fn relics(s: State<Engine>, q: Query) -> ApiResult {
    let owned = q.str("owned") == "1";
    view(s, move |e| Ok(e.accounts.view().relics(owned))).await
}

async fn find(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| Ok(e.accounts.view().find(q.str("q")))).await
}

async fn find_task(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| Ok(crate::tasks::find(&e.accounts.view(), q.str("q")))).await
}

async fn task(State(e): State<Engine>, q: Query) -> ApiResult {
    answer(e.task(q.str("uid").to_string()).await)
}

async fn item(State(e): State<Engine>, q: Query) -> ApiResult {
    answer(e.detail(q.str("key").to_string()).await)
}

async fn syndicate(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| e.syndicate(q.str("tag"))).await
}

/// An item's price history on the platform in the settings: `[{t, sell, buy}, ...]`, oldest first.
async fn price_history(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| e.db.price_history(e.settings().platform, q.str("slug"))).await
}

async fn search(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| Ok(e.search(q.str("q")))).await
}

/// A count in the query: absent or empty takes the default, anything else must be a number.
fn count(q: &Query, key: &str, default: u8) -> anyhow::Result<u8> {
    match q.0.get(key).map(String::as_str) {
        None | Some("") => Ok(default),
        Some(v) => v.parse().map_err(|_| UserError::new(format!("{key} must be a number")).into()),
    }
}

async fn riven_table(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| {
        match e.riven_table(q.str("weapon"), count(&q, "pos", 3)?, count(&q, "neg", 1)?) {
            Ok(v) => Ok(RivenTableAnswer::Table(v)),
            // The page shows "no riven data" in place, not as a failed request.
            Err(err) if err.downcast_ref::<UserError>().is_some() => {
                Ok(RivenTableAnswer::None { error: err.to_string() })
            }
            Err(err) => Err(err),
        }
    })
    .await
}

async fn profile_refresh(State(e): State<Engine>, _: Body) -> ApiResult {
    let key = e
        .accounts
        .active_key()
        .ok_or_else(|| UserError::new("No account yet: start the game once, or enter an account id."))?;
    e.accounts.refresh_profile(&key, false).await?;
    Ok(ok_json(&Done { ok: true }))
}

/// `{uid, onBoard: bool}` puts a task on the board or takes it off; `{uid, target: n | null}`
/// sets how many of a mod, an arcane or a resource it asks for.
async fn manual_task(s: State<Engine>, Typed(b): Typed<SetTask>) -> ApiResult {
    change(s, move |e| {
        if let Some(on) = b.on_board {
            e.accounts.set_task(&b.uid, on)?;
        }
        if let Some(target) = b.target {
            e.accounts.set_task_target(&b.uid, target.map(|n| n.round() as i64))?;
        }
        Ok(())
    })
    .await
}

// ------------------------------------------------------------------ what the changes take

#[derive(Deserialize, TS)]
#[ts(export)]
pub struct AccountId {
    /// The account id, as the user typed or pasted it.
    pub id: String,
    /// The account to give it to; the one shown when absent.
    #[serde(default)]
    #[ts(optional)]
    pub key: Option<String>,
}

#[derive(Deserialize, TS)]
#[ts(export)]
pub struct AccountKey {
    pub key: String,
}

#[derive(Deserialize, TS)]
#[ts(export)]
pub struct ByUid {
    pub uid: String,
}

/// A count set by hand; null removes it.
#[derive(Deserialize, TS)]
#[ts(export)]
pub struct SetCount {
    pub uid: String,
    #[serde(default)]
    pub count: Option<f64>,
}

#[derive(Deserialize, TS)]
#[ts(export)]
pub struct SetPinned {
    pub uid: String,
    pub pinned: bool,
}

#[derive(Deserialize, TS)]
#[ts(export)]
pub struct SetMastered {
    pub uid: String,
    pub mastered: bool,
}

/// Which reward of a screen in the history was picked; a null name undoes the pick.
#[derive(Deserialize, TS)]
#[ts(export)]
pub struct Pick {
    pub id: i64,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SetTask {
    pub uid: String,
    #[serde(default)]
    #[ts(optional)]
    pub on_board: Option<bool>,
    /// Absent: unchanged; null: back to the default target.
    #[serde(default, deserialize_with = "present")]
    #[ts(optional, type = "number | null")]
    pub target: Option<Option<f64>>,
}

/// A field that is there, null or not: `Some(None)` for null, `None` (the default) for absent.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<f64>>, D::Error> {
    Option::<f64>::deserialize(d).map(Some)
}
