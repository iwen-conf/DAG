use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use sunmao_core::graph::PublishInput;
use sunmao_store::projects::ProjectsRepo;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::state::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/projects", post(create_project).get(list_projects))
        .route("/v1/projects/lookup", get(lookup_project))
        .route("/v1/projects/{pid}", get(get_project))
        .route("/v1/projects/{pid}/graph", get(get_graph))
        .route("/v1/projects/{pid}/graph/publish", post(publish_graph))
        .route("/v1/projects/{pid}/tasks/claim-next", post(claim_next))
        .route("/v1/projects/{pid}/tasks/{id}", get(get_task))
        .route("/v1/projects/{pid}/tasks/{id}/heartbeat", post(heartbeat))
        .route(
            "/v1/projects/{pid}/tasks/{id}/handover-review",
            post(handover_review),
        )
        .route("/v1/projects/{pid}/tasks/{id}/submit", post(submit))
        .route("/v1/projects/{pid}/tasks/{id}/fail", post(fail_task))
        .route("/v1/projects/{pid}/tasks/{id}/cancel", post(cancel_task))
        .route("/v1/projects/{pid}/packages/{id}/scope", post(change_scope))
        .route(
            "/v1/projects/{pid}/contracts/{id}/publish",
            post(publish_contract),
        )
        .route(
            "/v1/projects/{pid}/contracts/{id}/approve-major",
            post(approve_major),
        )
        .route("/v1/projects/{pid}/artifacts/{id}", get(get_artifact))
        .route("/v1/projects/{pid}/events", get(list_events))
        .route("/v1/projects/{pid}/events/stream", get(events_stream))
        .route("/v1/projects/{pid}/replan-context", get(replan_context))
        .route(
            "/v1/projects/{pid}/admin/rebuild-projection",
            post(rebuild_projection),
        )
        .route("/v1/projects/{pid}/admin/verify", post(verify_ready))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn require_actor(headers: &HeaderMap) -> Result<String, (StatusCode, Json<Value>)> {
    match headers
        .get("X-Sunmao-Actor")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
    {
        Some(a) => Ok(a.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "code": "MISSING_ACTOR",
                "message": "X-Sunmao-Actor header required",
                "details": {}
            })),
        )),
    }
}

#[derive(Deserialize)]
struct CreateProject {
    name: String,
    repo_path: String,
}

async fn create_project(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreateProject>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let repo = ProjectsRepo::new(&st.pool);
    let p = repo
        .create_or_get(&body.name, &body.repo_path)
        .await
        .map_err(map_err)?;
    Ok(Json(json!({
        "id": p.id,
        "name": p.name,
        "repo_path": p.repo_path,
        "created_at": p.created_at,
    })))
}

async fn list_projects(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let repo = ProjectsRepo::new(&st.pool);
    let list = repo.list().await.map_err(map_err)?;
    Ok(Json(json!({ "projects": list })))
}

#[derive(Deserialize)]
struct LookupQ {
    repo_path: String,
}

async fn lookup_project(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<LookupQ>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let repo = ProjectsRepo::new(&st.pool);
    match repo.lookup_by_path(&q.repo_path).await.map_err(map_err)? {
        Some(p) => Ok(Json(json!({
            "id": p.id, "name": p.name, "repo_path": p.repo_path, "created_at": p.created_at
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "code": "NOT_FOUND", "message": "no project for repo_path", "details": {} })),
        )),
    }
}

async fn get_project(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let repo = ProjectsRepo::new(&st.pool);
    let p = repo.get(&pid).await.map_err(map_err)?;
    Ok(Json(json!({
        "id": p.id, "name": p.name, "repo_path": p.repo_path, "created_at": p.created_at
    })))
}

#[derive(Deserialize)]
struct GraphQ {
    root: Option<String>,
    depth: Option<i32>,
}

async fn get_graph(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
    Query(q): Query<GraphQ>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let g = st
        .store
        .get_graph(&pid, q.root.as_deref(), q.depth.unwrap_or(10))
        .await
        .map_err(map_err)?;
    Ok(Json(g))
}

async fn publish_graph(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
    Json(body): Json<PublishInput>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    match st.store.publish_graph(&pid, &actor, body).await {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err(map_err(e)),
    }
}

#[derive(Deserialize)]
struct ClaimBody {
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default = "default_ttl")]
    lease_ttl_secs: i64,
}

fn default_ttl() -> i64 {
    900
}

async fn claim_next(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
    Json(body): Json<ClaimBody>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    match st
        .store
        .claim_next(&pid, &actor, &body.capabilities, body.lease_ttl_secs)
        .await
        .map_err(map_err)?
    {
        Some(c) => Ok((StatusCode::OK, Json(json!(c))).into_response()),
        None => {
            let expandable = st
                .store
                .expandable_packages(&pid)
                .await
                .map_err(map_err)?;
            Ok((
                StatusCode::NO_CONTENT,
                Json(json!({ "hint": { "expandable_packages": expandable } })),
            )
                .into_response())
        }
    }
}

async fn get_task(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    Ok(Json(st.store.get_task(&pid, &id).await.map_err(map_err)?))
}

#[derive(Deserialize)]
struct LeaseBody {
    lease_token: Uuid,
    #[serde(default = "default_ttl")]
    lease_ttl_secs: i64,
    #[serde(default)]
    note: Option<String>,
}

async fn heartbeat(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<LeaseBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    let exp = st
        .store
        .heartbeat(&pid, &id, &actor, body.lease_token, body.lease_ttl_secs)
        .await
        .map_err(map_err)?;
    Ok(Json(json!({ "expires_at": exp })))
}

async fn handover_review(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    let token: Uuid = body
        .get("lease_token")
        .and_then(|t| t.as_str())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "code": "MISSING_TOKEN", "message": "lease_token required", "details": {} })),
            )
        })?;
    st.store
        .handover_review(&pid, &id, &actor, token, body)
        .await
        .map_err(map_err)?;
    Ok(Json(json!({ "ok": true })))
}

async fn submit(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<LeaseBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    let r = st
        .store
        .submit(&pid, &id, &actor, body.lease_token, body.note)
        .await
        .map_err(map_err)?;
    Ok(Json(json!(r)))
}

async fn fail_task(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    let token: Uuid = body
        .get("lease_token")
        .and_then(|t| t.as_str())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "code": "MISSING_TOKEN", "message": "lease_token required", "details": {} })),
            )
        })?;
    let reason = body.get("reason").cloned().unwrap_or(json!({}));
    Ok(Json(
        st.store
            .fail_task(&pid, &id, &actor, token, reason)
            .await
            .map_err(map_err)?,
    ))
}

#[derive(Deserialize)]
struct CancelBody {
    #[serde(default)]
    force: bool,
}

async fn cancel_task(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<CancelBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    Ok(Json(
        st.store
            .cancel_task(&pid, &id, &actor, body.force)
            .await
            .map_err(map_err)?,
    ))
}

#[derive(Deserialize)]
struct ScopeBody {
    action: String,
    reason: String,
    #[serde(default)]
    permanent: bool,
    #[serde(default)]
    force: bool,
}

async fn change_scope(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<ScopeBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    Ok(Json(
        st.store
            .change_scope(
                &pid,
                &id,
                &actor,
                &body.action,
                &body.reason,
                body.permanent,
                body.force,
            )
            .await
            .map_err(map_err)?,
    ))
}

#[derive(Deserialize)]
struct ContractPublish {
    bump: String,
    version: String,
    node_id: String,
}

async fn publish_contract(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
    Json(body): Json<ContractPublish>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    Ok(Json(
        st.store
            .publish_contract(&pid, &id, &actor, &body.bump, &body.version, &body.node_id)
            .await
            .map_err(map_err)?,
    ))
}

async fn approve_major(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let actor = require_actor(&headers)?;
    Ok(Json(
        st.store
            .approve_major(&pid, &id, &actor)
            .await
            .map_err(map_err)?,
    ))
}

async fn get_artifact(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((pid, id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    Ok(Json(
        st.store.get_artifact(&pid, &id).await.map_err(map_err)?,
    ))
}

#[derive(Deserialize)]
struct EventsQ {
    after_seq: Option<i64>,
    node: Option<String>,
    limit: Option<i64>,
}

async fn list_events(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
    Query(q): Query<EventsQ>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let events = st
        .store
        .list_events(
            &pid,
            q.after_seq.unwrap_or(0),
            q.node.as_deref(),
            q.limit.unwrap_or(100),
        )
        .await
        .map_err(map_err)?;
    Ok(Json(json!({ "events": events })))
}

async fn events_stream(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
    Query(q): Query<EventsQ>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, (StatusCode, Json<Value>)>
{
    let _ = require_actor(&headers)?;
    let after = q.after_seq.unwrap_or(0);
    // backlog
    let backlog = st
        .store
        .list_events(&pid, after, None, 500)
        .await
        .map_err(map_err)?;
    let mut rx = st.sse_tx.subscribe();
    let pid2 = pid.clone();
    let stream = async_stream::stream! {
        for ev in backlog {
            let kind = ev.get("kind").and_then(|k| k.as_str()).unwrap_or("event");
            let data = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
            yield Ok(Event::default().event(kind).data(data));
        }
        loop {
            match rx.recv().await {
                Ok(msg) if msg.project_id == pid2 => {
                    let data = json!({
                        "seq": msg.seq,
                        "node_id": msg.node_id,
                        "kind": msg.kind,
                        "payload": msg.payload,
                    });
                    yield Ok(Event::default().event(msg.kind).data(data.to_string()));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Deserialize)]
struct ReplanQ {
    task: String,
}

async fn replan_context(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
    Query(q): Query<ReplanQ>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    Ok(Json(
        st.store
            .replan_context(&pid, &q.task)
            .await
            .map_err(map_err)?,
    ))
}

async fn rebuild_projection(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    Ok(Json(
        st.store.rebuild_projection(&pid).await.map_err(map_err)?,
    ))
}

async fn verify_ready(
    State(st): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(pid): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let _ = require_actor(&headers)?;
    let ok = sunmao_store::ready_maint::verify_ready_consistent(&st.pool, &pid)
        .await
        .map_err(map_err)?;
    Ok(Json(json!({ "consistent": ok })))
}

fn map_err(e: sunmao_store::StoreError) -> (StatusCode, Json<Value>) {
    use sunmao_store::StoreError::*;
    match e {
        NotFound(m) => (
            StatusCode::NOT_FOUND,
            Json(json!({"code":"NOT_FOUND","message":m,"details":{}})),
        ),
        Conflict { code, message } => (
            StatusCode::CONFLICT,
            Json(json!({"code":code,"message":message,"details":{}})),
        ),
        Validation {
            code,
            message,
            details,
        } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"code":code,"message":message,"details":details})),
        ),
        Forbidden(m) => (
            StatusCode::FORBIDDEN,
            Json(json!({
                "code": if m == "HUMAN_ONLY" { "HUMAN_ONLY" } else { "FORBIDDEN" },
                "message": m,
                "details": {}
            })),
        ),
        other => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"code":"INTERNAL","message":other.to_string(),"details":{}})),
        ),
    }
}
