//! The API's routes. GET answers read the engine; POST changes it. Every answer is JSON.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    ApiError, ApiResult, Engine, JsonBody, MAX_BODY, MAX_IMPORT_BODY, Query, blocking, json_response, ok_json,
};
use crate::UserError;

type Body = JsonBody<MAX_BODY>;
type Import = JsonBody<MAX_IMPORT_BODY>;

pub fn api(engine: Engine, stop: CancellationToken) -> Router<Engine> {
    // What needs the game data: while it is still downloading, these say so instead.
    let catalog = Router::new()
        .route("/api/mastery", get(|s: State<Engine>| view(s, |e| Ok(e.view().mastery()))))
        .route("/api/primes", get(|s: State<Engine>| view(s, |e| Ok(e.view().primes()))))
        .route("/api/inventory", get(|s: State<Engine>| view(s, |e| Ok(e.view().inventory()))))
        .route("/api/foundry", get(|s: State<Engine>| view(s, |e| Ok(foundry(e)))))
        .route("/api/relics", get(relics))
        .route("/api/trade", get(|s: State<Engine>| view(s, |e| Ok(e.trade()))))
        .route("/api/find", get(find))
        .route("/api/find-task", get(find_task))
        .route("/api/tasks", get(|s: State<Engine>| view(s, |e| Ok(e.tasks()))))
        .route("/api/task", get(task))
        .route("/api/item", get(item))
        .route("/api/history", get(|s: State<Engine>| view(s, |e| Ok(e.history()))))
        .route("/api/hotkeys", get(|s: State<Engine>| view(s, |e| Ok(e.hotkeys()))))
        .route("/api/riven", get(riven_table))
        .route("/api/rivens", get(|s: State<Engine>| view(s, |e| Ok(e.rivens()))))
        .route("/api/fissures", get(|s: State<Engine>| view(s, |e| Ok(e.fissures()))))
        .route("/api/standing", get(|s: State<Engine>| view(s, |e| Ok(e.standing()))))
        .route("/api/syndicate", get(syndicate))
        .route("/api/search", get(search))
        .route_layer(middleware::from_fn_with_state(engine, loading_guard));

    let events_stop = stop.clone();
    Router::new()
        .merge(catalog)
        .route("/api/state", get(|s: State<Engine>| view(s, |e| Ok(e.summary()))))
        .route("/api/settings", get(|s: State<Engine>| view(s, |e| Ok(serde_json::to_value(e.settings())?))))
        .route("/api/world", get(|s: State<Engine>| view(s, |e| Ok(e.world()))))
        .route("/api/rewards", get(|s: State<Engine>| view(s, |e| Ok(result_json(e.rewards())))))
        .route("/api/riven/result", get(|s: State<Engine>| view(s, |e| Ok(result_json(e.riven())))))
        .route("/api/art", get(super::art::serve))
        .route("/api/events", get(move |s: State<Engine>| super::events::stream(s, events_stop.clone())))
        // Changes.
        .route("/api/settings", post(|s: State<Engine>, JsonBody(b): Body| change(s, move |e| e.update_settings(&b))))
        .route("/api/account/set", post(account_set))
        .route(
            "/api/account/switch",
            post(|s: State<Engine>, JsonBody(b): Body| {
                change(s, move |e| e.switch_account(b["key"].as_str().unwrap_or("")))
            }),
        )
        .route(
            "/api/account/forget",
            post(|s: State<Engine>, JsonBody(b): Body| {
                change(s, move |e| e.forget_account(b["key"].as_str().unwrap_or("")))
            }),
        )
        .route("/api/profile/refresh", post(profile_refresh))
        .route(
            "/api/profile/import",
            post(|s: State<Engine>, JsonBody(b): Import| change(s, move |e| e.import_profile(&b))),
        )
        .route(
            "/api/inventory/import",
            post(|s: State<Engine>, JsonBody(b): Import| {
                change(s, move |e| e.import_inventory(&b, "imported file").map(|_| ()))
            }),
        )
        .route(
            "/api/inventory/clear",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.clear_inventory();
                    Ok(())
                })
            }),
        )
        .route(
            "/api/manual/count",
            post(|s: State<Engine>, JsonBody(b): Body| {
                change(s, move |e| e.set_manual_count(&uid(&b)?, b["count"].as_f64().map(|c| c as i64)))
            }),
        )
        .route(
            "/api/manual/pin",
            post(|s: State<Engine>, JsonBody(b): Body| {
                change(s, move |e| e.set_pinned(&uid(&b)?, b["pinned"].as_bool().unwrap_or(false)))
            }),
        )
        .route("/api/manual/task", post(manual_task))
        .route(
            "/api/manual/mastered",
            post(|s: State<Engine>, JsonBody(b): Body| {
                change(s, move |e| e.set_manual_mastered(&uid(&b)?, b["mastered"].as_bool().unwrap_or(false)))
            }),
        )
        .route(
            "/api/tasks/clear",
            post(|s: State<Engine>, _: Body| view(s, |e| Ok(json!({"removed": e.clear_done_tasks()?})))),
        )
        .route(
            "/api/rewards/scan",
            post(|s: State<Engine>, _: Body| view(s, |e| Ok(serde_json::to_value(&*e.scan_rewards())?))),
        )
        .route(
            "/api/riven/scan",
            post(|s: State<Engine>, _: Body| view(s, |e| Ok(serde_json::to_value(&*e.scan_riven())?))),
        )
        .route("/api/riven/grade", post(|s: State<Engine>, JsonBody(b): Body| view(s, move |e| e.grade_riven(&b))))
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
        .route(
            "/api/data/reload",
            post(|s: State<Engine>, _: Body| {
                change(s, |e| {
                    e.load_static(true);
                    match e.catalog_error() {
                        Some(err) => Err(UserError::new(err).into()),
                        None => Ok(()),
                    }
                })
            }),
        )
        .route("/api/{*rest}", get(unknown).post(unknown))
        .method_not_allowed_fallback(|| async {
            json_response(StatusCode::METHOD_NOT_ALLOWED, &json!({"error": "method not allowed"}))
        })
}

/// A read: `work` runs on the blocking pool, its value is the answer.
async fn view(
    State(engine): State<Engine>,
    work: impl FnOnce(&Engine) -> anyhow::Result<Value> + Send + 'static,
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
    Ok(ok_json(&json!({"ok": true})))
}

/// The pages that need the game data say "loading" until it is there.
async fn loading_guard(State(engine): State<Engine>, req: Request, next: Next) -> Response {
    if engine.catalog().loaded_at.is_none() {
        return ok_json(&json!({"loading": true, "error": engine.catalog_error()}));
    }
    next.run(req).await
}

async fn unknown(req: Request) -> ApiError {
    let path = req.uri().path().trim_start_matches("/api/").to_string();
    UserError::new(format!("unknown endpoint {path}")).into()
}

fn uid(b: &Value) -> anyhow::Result<String> {
    b["uid"].as_str().map(str::to_owned).ok_or_else(|| UserError::new("uid is required").into())
}

fn result_json<T: serde::Serialize>(r: Option<Arc<T>>) -> Value {
    r.map_or(json!({}), |r| serde_json::to_value(&*r).unwrap_or_default())
}

fn foundry(e: &Engine) -> Value {
    let v = e.view();
    let mut items = v.craftable();
    if let Some(list) = items.as_array_mut() {
        list.extend(v.crafts());
    }
    json!({"pending": v.pending_foundry(), "items": items})
}

async fn relics(s: State<Engine>, q: Query) -> ApiResult {
    let owned = q.str("owned") == "1";
    view(s, move |e| Ok(e.view().relics(owned))).await
}

async fn find(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| Ok(e.view().find(q.str("q")))).await
}

async fn find_task(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| Ok(crate::tasks::find(&e.view(), q.str("q")))).await
}

async fn task(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| e.task(q.str("uid"))).await
}

async fn item(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| e.detail(q.str("key"))).await
}

async fn syndicate(s: State<Engine>, q: Query) -> ApiResult {
    view(s, move |e| e.syndicate(q.str("tag"))).await
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
            Ok(v) => Ok(v),
            // The page shows "no riven data" in place, not as a failed request.
            Err(err) if err.downcast_ref::<UserError>().is_some() => Ok(json!({"error": err.to_string()})),
            Err(err) => Err(err),
        }
    })
    .await
}

async fn account_set(s: State<Engine>, JsonBody(b): Body) -> ApiResult {
    change(s, move |e| e.set_account_id(b["id"].as_str().unwrap_or(""), b["key"].as_str())).await
}

async fn profile_refresh(s: State<Engine>, _: Body) -> ApiResult {
    change(s, |e| {
        let key = e
            .active_key()
            .ok_or_else(|| UserError::new("No account yet: start the game once, or enter an account id."))?;
        e.refresh_profile(&key, false)?;
        Ok(())
    })
    .await
}

/// `{uid, onBoard: bool}` puts a task on the board or takes it off; `{uid, target: n | null}`
/// sets how many of a mod, an arcane or a resource it asks for.
async fn manual_task(s: State<Engine>, JsonBody(b): Body) -> ApiResult {
    change(s, move |e| {
        let uid = uid(&b)?;
        if let Some(on) = b["onBoard"].as_bool() {
            e.set_task(&uid, on)?;
        }
        if let Some(target) = b.get("target") {
            e.set_task_target(&uid, target.as_f64().map(|n| n.round() as i64))?;
        }
        Ok(())
    })
    .await
}
