use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sunmao_core::new_id;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: String,
    pub project_id: String,
    pub seq: i64,
    pub node_id: Option<String>,
    pub actor: String,
    pub kind: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

pub fn new_event_id() -> String {
    new_id("ev_")
}
