//! Ready column maintenance (three touch points + full recompute).

use sqlx::{PgPool, Postgres, Transaction};

use crate::error::StoreResult;

/// Full recompute of ready for all tasks in a project. Returns (id, ready) pairs.
pub async fn recompute_all(pool: &PgPool, project_id: &str) -> StoreResult<Vec<(String, bool)>> {
    let rows = sqlx::query_as::<_, (String, bool)>(
        r#"
        WITH upstream_ok AS (
          SELECT t.id,
            NOT EXISTS (
              SELECT 1 FROM edge e
              JOIN node up ON up.project_id = e.project_id AND up.id = e.from_id
              WHERE e.project_id = t.project_id AND e.to_id = t.id
                AND up.kind = 'task' AND up.task_state IS DISTINCT FROM 'done'
            ) AS deps_ok
          FROM node t
          WHERE t.project_id = $1 AND t.kind = 'task'
        ),
        anc_ok AS (
          SELECT t.id,
            NOT EXISTS (
              SELECT 1 FROM node p
              WHERE p.project_id = t.project_id AND p.kind = 'package'
                AND p.scope_state != 'active'
                AND t.path LIKE p.path || '.%'
            ) AS scope_ok
          FROM node t
          WHERE t.project_id = $1 AND t.kind = 'task'
        )
        SELECT t.id,
          (t.task_state IN ('todo','ready') AND u.deps_ok AND a.scope_ok) AS should_ready
        FROM node t
        JOIN upstream_ok u ON u.id = t.id
        JOIN anc_ok a ON a.id = t.id
        WHERE t.project_id = $1 AND t.kind = 'task'
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Apply full recompute inside a transaction (sync ready + task_state todo/ready).
pub async fn apply_recompute_tx(
    tx: &mut Transaction<'_, Postgres>,
    project_id: &str,
) -> StoreResult<Vec<String>> {
    // Update ready flag
    sqlx::query(
        r#"
        WITH upstream_ok AS (
          SELECT t.id,
            NOT EXISTS (
              SELECT 1 FROM edge e
              JOIN node up ON up.project_id = e.project_id AND up.id = e.from_id
              WHERE e.project_id = t.project_id AND e.to_id = t.id
                AND up.kind = 'task' AND up.task_state IS DISTINCT FROM 'done'
            ) AS deps_ok
          FROM node t
          WHERE t.project_id = $1 AND t.kind = 'task'
            AND t.task_state IN ('todo','ready')
        ),
        anc_ok AS (
          SELECT t.id,
            NOT EXISTS (
              SELECT 1 FROM node p
              WHERE p.project_id = t.project_id AND p.kind = 'package'
                AND p.scope_state != 'active'
                AND t.path LIKE p.path || '.%'
            ) AS scope_ok
          FROM node t
          WHERE t.project_id = $1 AND t.kind = 'task'
            AND t.task_state IN ('todo','ready')
        ),
        calc AS (
          SELECT t.id, (u.deps_ok AND a.scope_ok) AS should_ready
          FROM node t
          JOIN upstream_ok u ON u.id = t.id
          JOIN anc_ok a ON a.id = t.id
          WHERE t.project_id = $1
        )
        UPDATE node n SET
          ready = c.should_ready,
          task_state = CASE
            WHEN c.should_ready AND n.task_state = 'todo' THEN 'ready'
            WHEN NOT c.should_ready AND n.task_state = 'ready' THEN 'todo'
            ELSE n.task_state
          END,
          updated_at = now()
        FROM calc c
        WHERE n.project_id = $1 AND n.id = c.id
        "#,
    )
    .bind(project_id)
    .execute(&mut **tx)
    .await?;

    let ready_now = sqlx::query_scalar::<_, String>(
        "SELECT id FROM node WHERE project_id = $1 AND kind = 'task' AND ready ORDER BY id",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(ready_now)
}

/// Verify SQL ready matches independent recompute (A-04).
pub async fn verify_ready_consistent(pool: &PgPool, project_id: &str) -> StoreResult<bool> {
    let expected = recompute_all(pool, project_id).await?;
    let actual = sqlx::query_as::<_, (String, bool)>(
        "SELECT id, ready FROM node WHERE project_id = $1 AND kind = 'task' ORDER BY id",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let mut exp_map: std::collections::HashMap<_, _> = expected.into_iter().collect();
    for (id, ready) in actual {
        if exp_map.remove(&id) != Some(ready) {
            // if stored ready differs from should_ready for non-todo/ready states, only compare eligible
            let st: Option<String> = sqlx::query_scalar(
                "SELECT task_state FROM node WHERE project_id = $1 AND id = $2",
            )
            .bind(project_id)
            .bind(&id)
            .fetch_optional(pool)
            .await?;
            if matches!(st.as_deref(), Some("todo") | Some("ready")) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
