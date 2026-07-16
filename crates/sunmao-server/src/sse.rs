use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgListener;
use tokio::sync::broadcast;

use crate::state::SseMsg;

pub async fn listen_loop(db_url: &str, tx: broadcast::Sender<SseMsg>) -> anyhow::Result<()> {
    loop {
        if let Err(e) = listen_once(db_url, tx.clone()).await {
            tracing::warn!(error = %e, "LISTEN reconnect in 2s");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

async fn listen_once(db_url: &str, tx: broadcast::Sender<SseMsg>) -> anyhow::Result<()> {
    let mut listener = PgListener::connect(db_url)
        .await
        .context("PgListener connect")?;
    listener.listen("sunmao_events").await?;
    loop {
        let notification = listener.recv().await?;
        let payload = notification.payload();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
            let msg = SseMsg {
                project_id: v
                    .get("project_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                seq: v.get("seq").and_then(|x| x.as_i64()).unwrap_or(0),
                kind: v
                    .get("kind")
                    .and_then(|x| x.as_str())
                    .unwrap_or("event")
                    .to_string(),
                node_id: v
                    .get("node_id")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                payload: v,
            };
            let _ = tx.send(msg);
        }
    }
}
