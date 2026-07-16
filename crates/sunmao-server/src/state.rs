use sqlx::PgPool;
use sunmao_store::Store;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub pool: PgPool,
    pub sse_tx: broadcast::Sender<SseMsg>,
}

#[derive(Debug, Clone)]
pub struct SseMsg {
    pub project_id: String,
    pub seq: i64,
    pub kind: String,
    pub node_id: Option<String>,
    pub payload: serde_json::Value,
}
