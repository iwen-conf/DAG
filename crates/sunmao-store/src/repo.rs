//! Store: event+projection apply, claim, submit, graph publish, contracts.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use sunmao_core::graph::{
    validate_publish, ExistingNode, GraphSnapshot, PublishInput, BUILTIN_VALIDATORS,
};
use sunmao_core::new_id;
use sunmao_core::state_machine::TaskState;
use sunmao_core::write_scope::scopes_conflict;
use uuid::Uuid;

use crate::error::{StoreError, StoreResult};
use crate::event::new_event_id;
use crate::git::GitWorkspace;
use crate::ready_maint::apply_recompute_tx;
use crate::validators::{run_validators, ValidatorFailure};

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NodeRow {
    pub id: String,
    pub project_id: String,
    pub graph_version: i64,
    pub parent_id: Option<String>,
    pub path: String,
    pub kind: String,
    pub title: String,
    pub spec: Value,
    pub task_state: Option<String>,
    pub ready: bool,
    pub priority: i32,
    pub required_caps: Vec<String>,
    pub write_scope: Vec<String>,
    pub inputs: Value,
    pub validators: Vec<String>,
    pub max_attempts: i32,
    pub owner: Option<String>,
    pub lease_token: Option<Uuid>,
    pub lease_expires: Option<DateTime<Utc>>,
    pub scope_state: String,
    pub plan_state: String,
    pub needs_replan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimResponse {
    pub task: ClaimedTask,
    pub lease: LeaseInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handover: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedTask {
    pub id: String,
    pub title: String,
    pub spec: Value,
    pub inputs: Value,
    pub write_scope: Vec<String>,
    pub validators: Vec<String>,
    pub attempt_seq: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub token: Uuid,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitResult {
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failures: Option<Vec<ValidatorFailure>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(pool: &PgPool) -> StoreResult<()> {
        // sqlx::migrate! needs compile-time folder; use runtime migrator
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/migrations"
        )))
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        migrator
            .run(pool)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(())
    }

    pub async fn current_graph_version(&self, project_id: &str) -> StoreResult<i64> {
        let v: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(MAX(version), 0) FROM graph_version WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(v.unwrap_or(0))
    }

    async fn load_snapshot(&self, project_id: &str) -> StoreResult<GraphSnapshot> {
        let version = self.current_graph_version(project_id).await?;
        let rows = sqlx::query(
            r#"
            SELECT id, kind, path, parent_id, write_scope, validators, task_state, title, inputs,
              EXISTS(SELECT 1 FROM attempt a WHERE a.project_id = n.project_id AND a.node_id = n.id) AS has_attempt
            FROM node n WHERE project_id = $1
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;

        let mut nodes = HashMap::new();
        for r in rows {
            let id: String = r.get("id");
            let task_state: Option<String> = r.get("task_state");
            nodes.insert(
                id,
                ExistingNode {
                    kind: r.get("kind"),
                    path: r.get("path"),
                    parent_id: r.get("parent_id"),
                    write_scope: r.get("write_scope"),
                    validators: r.get("validators"),
                    task_state: task_state.as_deref().and_then(TaskState::parse),
                    title: r.get("title"),
                    has_attempt_history: r.get("has_attempt"),
                    inputs: r.get("inputs"),
                },
            );
        }
        let edge_rows = sqlx::query("SELECT from_id, to_id FROM edge WHERE project_id = $1")
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        let mut edges = HashSet::new();
        for e in edge_rows {
            edges.insert((e.get("from_id"), e.get("to_id")));
        }
        Ok(GraphSnapshot {
            version,
            nodes,
            edges,
            registered_validators: BUILTIN_VALIDATORS.iter().map(|s| s.to_string()).collect(),
        })
    }

    pub async fn publish_graph(
        &self,
        project_id: &str,
        actor: &str,
        input: PublishInput,
    ) -> StoreResult<Value> {
        let mut tx = self.pool.begin().await?;
        let current = self.current_graph_version(project_id).await?;
        if input.base_version != current {
            return Err(StoreError::stale_graph(format!(
                "base_version {} != current {current}",
                input.base_version
            )));
        }

        let snapshot = self.load_snapshot(project_id).await?;
        let report = validate_publish(&snapshot, &input);
        if !report.is_ok() {
            return Err(StoreError::Validation {
                code: "GRAPH_INVALID".into(),
                message: format!("{} violation(s)", report.violations.len()),
                details: json!({ "violations": report.violations }),
            });
        }

        let new_version = current + 1;
        sqlx::query(
            "INSERT INTO graph_version (project_id, version, planner, summary) VALUES ($1,$2,$3,$4)",
        )
        .bind(project_id)
        .bind(new_version)
        .bind(actor)
        .bind(&input.summary)
        .execute(&mut *tx)
        .await?;

        // removals
        for id in &input.remove_nodes {
            sqlx::query("DELETE FROM edge WHERE project_id=$1 AND (from_id=$2 OR to_id=$2)")
                .bind(project_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM node WHERE project_id=$1 AND id=$2")
                .bind(project_id)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }

        for n in &input.upsert_nodes {
            let path = n.path.clone().unwrap_or_else(|| {
                if let Some(pid) = &n.parent_id {
                    format!("{}.{}", pid, n.id)
                } else {
                    n.id.clone()
                }
            });
            // resolve path from parent if present
            let path = if let Some(pid) = &n.parent_id {
                let parent_path: Option<String> = sqlx::query_scalar(
                    "SELECT path FROM node WHERE project_id=$1 AND id=$2",
                )
                .bind(project_id)
                .bind(pid)
                .fetch_optional(&mut *tx)
                .await?
                .or_else(|| {
                    input
                        .upsert_nodes
                        .iter()
                        .find(|u| &u.id == pid)
                        .and_then(|u| u.path.clone())
                });
                if let Some(pp) = parent_path {
                    format!(
                        "{}.{}",
                        pp,
                        n.id.rsplit('_').next().unwrap_or(&n.id).chars().take(8).collect::<String>()
                    )
                } else {
                    path
                }
            } else {
                n.path.clone().unwrap_or(path)
            };

            let task_state = if n.kind == "task" { Some("todo") } else { None };
            sqlx::query(
                r#"
                INSERT INTO node (
                  id, project_id, graph_version, parent_id, path, kind, layer, role, title, spec,
                  task_state, ready, priority, required_caps, write_scope, inputs, validators, max_attempts
                ) VALUES (
                  $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,
                  $11,false,$12,$13,$14,$15,$16,$17
                )
                ON CONFLICT (project_id, id) DO UPDATE SET
                  graph_version = EXCLUDED.graph_version,
                  parent_id = EXCLUDED.parent_id,
                  path = EXCLUDED.path,
                  title = EXCLUDED.title,
                  spec = EXCLUDED.spec,
                  priority = EXCLUDED.priority,
                  required_caps = EXCLUDED.required_caps,
                  write_scope = EXCLUDED.write_scope,
                  inputs = EXCLUDED.inputs,
                  validators = EXCLUDED.validators,
                  max_attempts = EXCLUDED.max_attempts,
                  updated_at = now()
                "#,
            )
            .bind(&n.id)
            .bind(project_id)
            .bind(new_version)
            .bind(&n.parent_id)
            .bind(&path)
            .bind(&n.kind)
            .bind(&n.layer)
            .bind(&n.role)
            .bind(&n.title)
            .bind(&n.spec)
            .bind(task_state)
            .bind(n.priority)
            .bind(&n.required_caps)
            .bind(&n.write_scope)
            .bind(&n.inputs)
            .bind(&n.validators)
            .bind(n.max_attempts)
            .execute(&mut *tx)
            .await?;
        }

        for e in &input.add_edges {
            sqlx::query(
                "INSERT INTO edge (project_id, from_id, to_id) VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
            )
            .bind(project_id)
            .bind(&e.from)
            .bind(&e.to)
            .execute(&mut *tx)
            .await?;
        }

        let ready_now = apply_recompute_tx(&mut tx, project_id).await?;

        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,NULL,$3,'graph.published',$4)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(actor)
        .bind(json!({
            "version": new_version,
            "summary": input.summary,
            "added": input.upsert_nodes.iter().map(|n| &n.id).collect::<Vec<_>>(),
            "removed": input.remove_nodes,
        }))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(json!({ "version": new_version, "ready_now": ready_now }))
    }

    pub async fn claim_next(
        &self,
        project_id: &str,
        actor: &str,
        capabilities: &[String],
        lease_ttl_secs: i64,
    ) -> StoreResult<Option<ClaimResponse>> {
        // LIMIT 1 + SKIP LOCKED (doc 02 §2.6); on write-scope conflict skip id and retry.
        // Never lock a batch larger than 1 — concurrent claimants would starve.
        let mut skip_ids: Vec<String> = Vec::new();
        for _ in 0..64 {
            let mut tx = self.pool.begin().await?;
            let cand = sqlx::query(
                r#"
                SELECT n.id, n.title, n.spec, n.inputs, n.write_scope, n.validators
                FROM node n
                WHERE n.project_id = $1
                  AND n.kind = 'task'
                  AND n.ready
                  AND n.required_caps <@ $2
                  AND NOT (n.id = ANY($3))
                ORDER BY n.priority DESC, n.id
                FOR UPDATE OF n SKIP LOCKED
                LIMIT 1
                "#,
            )
            .bind(project_id)
            .bind(capabilities)
            .bind(&skip_ids)
            .fetch_optional(&mut *tx)
            .await?;

            let Some(cand) = cand else {
                tx.rollback().await?;
                return Ok(None);
            };

            let node_id: String = cand.get("id");
            let write_scope: Vec<String> = cand.get("write_scope");

            // running scopes for conflict check (A-02 claim-time)
            let running = sqlx::query_as::<_, (String, Vec<String>)>(
                r#"
                SELECT id, write_scope FROM node
                WHERE project_id = $1 AND kind = 'task'
                  AND task_state IN ('claimed','running','review')
                "#,
            )
            .bind(project_id)
            .fetch_all(&mut *tx)
            .await?;

            if running.iter().any(|(_, rws)| scopes_conflict(&write_scope, rws)) {
                // prefix refine miss: release lock and try another candidate
                skip_ids.push(node_id);
                tx.rollback().await?;
                continue;
            }

            let token = Uuid::new_v4();
            let ttl = if lease_ttl_secs <= 0 { 900 } else { lease_ttl_secs };

            let updated = sqlx::query(
                r#"
                UPDATE node SET
                  task_state = 'claimed',
                  ready = false,
                  owner = $3,
                  lease_token = $4,
                  lease_expires = now() + make_interval(secs => $5::double precision),
                  updated_at = now()
                WHERE project_id = $1 AND id = $2
                RETURNING lease_expires
                "#,
            )
            .bind(project_id)
            .bind(&node_id)
            .bind(actor)
            .bind(token)
            .bind(ttl as f64)
            .fetch_one(&mut *tx)
            .await?;

            let expires: DateTime<Utc> = updated.get("lease_expires");

            let prev_seq: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq_no), 0) FROM attempt WHERE project_id=$1 AND node_id=$2",
            )
            .bind(project_id)
            .bind(&node_id)
            .fetch_one(&mut *tx)
            .await?;
            let seq_no = prev_seq + 1;
            let attempt_id = new_id("at_");
            sqlx::query(
                r#"
                INSERT INTO attempt (id, project_id, node_id, seq_no, owner)
                VALUES ($1,$2,$3,$4,$5)
                "#,
            )
            .bind(&attempt_id)
            .bind(project_id)
            .bind(&node_id)
            .bind(seq_no)
            .bind(actor)
            .execute(&mut *tx)
            .await?;

            let ev_id = new_event_id();
            sqlx::query(
                r#"
                INSERT INTO event (id, project_id, node_id, actor, kind, payload)
                VALUES ($1,$2,$3,$4,'task.claimed',$5)
                "#,
            )
            .bind(&ev_id)
            .bind(project_id)
            .bind(&node_id)
            .bind(actor)
            .bind(json!({ "owner": actor, "lease_token": token, "attempt_seq": seq_no }))
            .execute(&mut *tx)
            .await?;

            let title: String = cand.get("title");
            let spec: Value = cand.get("spec");
            let inputs: Value = cand.get("inputs");
            let validators: Vec<String> = cand.get("validators");

            // handover for relay
            let handover = if seq_no > 1 {
                let prev = sqlx::query(
                    r#"
                    SELECT seq_no, owner, outcome, failure, handover
                    FROM attempt WHERE project_id=$1 AND node_id=$2 AND seq_no < $3
                    ORDER BY seq_no
                    "#,
                )
                .bind(project_id)
                .bind(&node_id)
                .bind(seq_no)
                .fetch_all(&mut *tx)
                .await?;
                let previous_attempts: Vec<Value> = prev
                    .iter()
                    .map(|r| {
                        json!({
                            "seq_no": r.get::<i32,_>("seq_no"),
                            "owner": r.get::<String,_>("owner"),
                            "outcome": r.get::<Option<String>,_>("outcome"),
                            "failure": r.get::<Option<Value>,_>("failure"),
                            "handover_report": r.get::<Option<Value>,_>("handover"),
                        })
                    })
                    .collect();

                let proj = sqlx::query_as::<_, (String,)>(
                    "SELECT repo_path FROM project WHERE id = $1",
                )
                .bind(project_id)
                .fetch_one(&mut *tx)
                .await?;
                let git = GitWorkspace::new(&proj.0);
                let wip = git.status_porcelain().unwrap_or_default();
                let modified: Vec<String> = wip
                    .iter()
                    .filter(|e| {
                        sunmao_core::write_scope::path_in_scopes(&e.path, &write_scope)
                            && e.status != "??"
                    })
                    .map(|e| e.path.clone())
                    .collect();
                let untracked: Vec<String> = wip
                    .iter()
                    .filter(|e| {
                        sunmao_core::write_scope::path_in_scopes(&e.path, &write_scope)
                            && e.status == "??"
                    })
                    .map(|e| e.path.clone())
                    .collect();
                let gv = self.current_graph_version(project_id).await?;
                Some(json!({
                    "previous_attempts": previous_attempts,
                    "work_in_progress": { "modified": modified, "untracked": untracked },
                    "progress_snapshot": { "graph_version": gv, "upstream_artifacts": [] },
                    "instruction": "你接手的是一个中断任务。必须先审查 work_in_progress 中列出的在制品，判断可复用或需清理重做，并调用 handover-review 如实上报结论后再开工。不得隐瞒现场情况。"
                }))
            } else {
                None
            };

            tx.commit().await?;
            return Ok(Some(ClaimResponse {
                task: ClaimedTask {
                    id: node_id,
                    title,
                    spec,
                    inputs,
                    write_scope,
                    validators,
                    attempt_seq: seq_no,
                },
                lease: LeaseInfo {
                    token,
                    expires_at: expires,
                },
                handover,
            }));
        }
        Ok(None)
    }

    pub async fn expandable_packages(&self, project_id: &str) -> StoreResult<Vec<String>> {
        let rows = sqlx::query_scalar::<_, String>(
            r#"
            SELECT id || '（' || path || ', plan_state=' || plan_state || '）'
            FROM node
            WHERE project_id = $1 AND kind = 'package' AND plan_state IN ('draft','planning')
            ORDER BY path
            LIMIT 20
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn heartbeat(
        &self,
        project_id: &str,
        node_id: &str,
        actor: &str,
        lease_token: Uuid,
        lease_ttl_secs: i64,
    ) -> StoreResult<DateTime<Utc>> {
        let ttl = if lease_ttl_secs <= 0 { 900 } else { lease_ttl_secs };
        let row = sqlx::query(
            r#"
            UPDATE node SET lease_expires = now() + make_interval(secs => $4::double precision), updated_at = now()
            WHERE project_id = $1 AND id = $2
              AND owner = $3 AND lease_token = $5
              AND lease_expires > now()
              AND task_state IN ('claimed','running','review')
            RETURNING lease_expires
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .bind(actor)
        .bind(ttl as f64)
        .bind(lease_token)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some(r) => Ok(r.get("lease_expires")),
            None => Err(StoreError::lease_lost("heartbeat rejected")),
        }
    }

    async fn check_lease(
        &self,
        project_id: &str,
        node_id: &str,
        lease_token: Uuid,
    ) -> StoreResult<NodeRow> {
        let row = sqlx::query_as::<_, NodeRow>(
            r#"
            SELECT id, project_id, graph_version, parent_id, path, kind, title, spec,
                   task_state, ready, priority, required_caps, write_scope, inputs, validators,
                   max_attempts, owner, lease_token, lease_expires, scope_state, plan_state, needs_replan
            FROM node WHERE project_id=$1 AND id=$2
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("node {node_id}")))?;

        if row.lease_token != Some(lease_token)
            || row.lease_expires.map(|e| e <= Utc::now()).unwrap_or(true)
            || !matches!(
                row.task_state.as_deref(),
                Some("claimed") | Some("running") | Some("review")
            )
        {
            return Err(StoreError::lease_lost("token mismatch or expired"));
        }
        Ok(row)
    }

    pub async fn handover_review(
        &self,
        project_id: &str,
        node_id: &str,
        actor: &str,
        lease_token: Uuid,
        body: Value,
    ) -> StoreResult<()> {
        let _node = self.check_lease(project_id, node_id, lease_token).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE attempt SET handover = $4
            WHERE project_id=$1 AND node_id=$2
              AND seq_no = (SELECT MAX(seq_no) FROM attempt WHERE project_id=$1 AND node_id=$2)
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .bind(actor)
        .bind(&body)
        .execute(&mut *tx)
        .await?;

        if body.get("decision").and_then(|d| d.as_str()) == Some("discard_and_redo") {
            if let Some(paths) = body.get("discarded_paths").and_then(|p| p.as_array()) {
                let proj: String = sqlx::query_scalar("SELECT repo_path FROM project WHERE id=$1")
                    .bind(project_id)
                    .fetch_one(&mut *tx)
                    .await?;
                let git = GitWorkspace::new(proj);
                let paths: Vec<String> = paths
                    .iter()
                    .filter_map(|p| p.as_str().map(|s| s.to_string()))
                    .collect();
                let _ = git.checkout_paths(&paths);
            }
        }

        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,$3,$4,'task.handover_reported',$5)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(node_id)
        .bind(actor)
        .bind(&body)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn submit(
        &self,
        project_id: &str,
        node_id: &str,
        actor: &str,
        lease_token: Uuid,
        _note: Option<String>,
    ) -> StoreResult<SubmitResult> {
        let node = self.check_lease(project_id, node_id, lease_token).await?;

        // handover required for attempt_seq > 1 without handover
        let attempt = sqlx::query(
            r#"
            SELECT id, seq_no, handover FROM attempt
            WHERE project_id=$1 AND node_id=$2
            ORDER BY seq_no DESC LIMIT 1
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_one(&self.pool)
        .await?;
        let seq_no: i32 = attempt.get("seq_no");
        let handover: Option<Value> = attempt.get("handover");
        if seq_no > 1 && handover.is_none() {
            return Err(StoreError::handover_required());
        }

        let proj: String = sqlx::query_scalar("SELECT repo_path FROM project WHERE id=$1")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await?;
        let git = GitWorkspace::new(&proj);

        // neighbor running scopes
        let neighbors: Vec<Vec<String>> = sqlx::query_scalar(
            r#"
            SELECT write_scope FROM node
            WHERE project_id=$1 AND kind='task'
              AND task_state IN ('claimed','running','review') AND id <> $2
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_all(&self.pool)
        .await?;

        let msg_needle = format!("task({node_id})");
        // idempotent: commit done but PG not
        if let Ok(Some(hash)) = git.find_commit_by_message(&msg_needle) {
            if git
                .partition_by_scope(&node.write_scope, &neighbors)?
                .in_scope
                .is_empty()
            {
                return self
                    .finalize_done(project_id, &node, actor, &attempt.get::<String, _>("id"), seq_no, &hash, &[])
                    .await;
            }
        }

        let diff = git.partition_by_scope(&node.write_scope, &neighbors)?;
        let failures = run_validators(
            &node.validators,
            &git,
            &node.write_scope,
            &diff.in_scope,
            &diff.out_scope,
        )?;

        if !failures.is_empty() {
            return self
                .fail_attempt(
                    project_id,
                    &node,
                    actor,
                    &attempt.get::<String, _>("id"),
                    seq_no,
                    "validation_failed",
                    json!({ "failures": failures }),
                )
                .await
                .map(|next| SubmitResult {
                    verdict: "failed".into(),
                    artifact: None,
                    failures: Some(failures),
                    next: Some(next),
                });
        }

        let commit_hash = if diff.in_scope.is_empty() {
            // no changes — still allow empty completion via empty tree marker file? require at least touch
            return self
                .fail_attempt(
                    project_id,
                    &node,
                    actor,
                    &attempt.get::<String, _>("id"),
                    seq_no,
                    "validation_failed",
                    json!({ "failures": [{ "validator": "artifact-exists", "report": "no in-scope changes to commit" }] }),
                )
                .await
                .map(|next| SubmitResult {
                    verdict: "failed".into(),
                    artifact: None,
                    failures: Some(vec![ValidatorFailure {
                        validator: "artifact-exists".into(),
                        report: "no in-scope changes to commit".into(),
                    }]),
                    next: Some(next),
                });
        } else {
            git.commit_paths(
                &diff.in_scope,
                &format!("task({node_id}): {} [attempt {seq_no}]", node.title),
            )?
        };

        self.finalize_done(
            project_id,
            &node,
            actor,
            &attempt.get::<String, _>("id"),
            seq_no,
            &commit_hash,
            &diff.in_scope,
        )
        .await
    }

    async fn finalize_done(
        &self,
        project_id: &str,
        node: &NodeRow,
        actor: &str,
        attempt_id: &str,
        seq_no: i32,
        commit_hash: &str,
        paths: &[String],
    ) -> StoreResult<SubmitResult> {
        let mut tx = self.pool.begin().await?;
        let art_id = new_id("ar_");
        let digest = {
            let proj: String = sqlx::query_scalar("SELECT repo_path FROM project WHERE id=$1")
                .bind(project_id)
                .fetch_one(&mut *tx)
                .await?;
            GitWorkspace::new(proj)
                .tree_digest(paths)
                .unwrap_or_else(|_| commit_hash.to_string())
        };

        sqlx::query(
            r#"
            UPDATE attempt SET ended_at=now(), outcome='done'
            WHERE id=$1
            "#,
        )
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO artifact (id, project_id, node_id, attempt_id, paths, commit_hash, digest)
            VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(&art_id)
        .bind(project_id)
        .bind(&node.id)
        .bind(attempt_id)
        .bind(paths)
        .bind(commit_hash)
        .bind(&digest)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE node SET task_state='done', ready=false, owner=NULL, lease_token=NULL,
              lease_expires=NULL, updated_at=now()
            WHERE project_id=$1 AND id=$2
            "#,
        )
        .bind(project_id)
        .bind(&node.id)
        .execute(&mut *tx)
        .await?;

        let _ = apply_recompute_tx(&mut tx, project_id).await?;

        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,$3,$4,'task.done',$5)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(&node.id)
        .bind(actor)
        .bind(json!({ "artifact_id": art_id, "commit": commit_hash, "attempt_seq": seq_no }))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(SubmitResult {
            verdict: "done".into(),
            artifact: Some(json!({ "id": art_id, "commit": commit_hash })),
            failures: None,
            next: None,
        })
    }

    async fn fail_attempt(
        &self,
        project_id: &str,
        node: &NodeRow,
        actor: &str,
        attempt_id: &str,
        seq_no: i32,
        outcome: &str,
        failure: Value,
    ) -> StoreResult<String> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
            UPDATE attempt SET ended_at=now(), outcome=$2, failure=$3 WHERE id=$1
            "#,
        )
        .bind(attempt_id)
        .bind(outcome)
        .bind(&failure)
        .execute(&mut *tx)
        .await?;

        let next = if seq_no < node.max_attempts {
            // reopen
            let inputs = inject_failure_inputs(&node.inputs, &failure);
            sqlx::query(
                r#"
                UPDATE node SET task_state='ready', ready=true, owner=NULL, lease_token=NULL,
                  lease_expires=NULL, inputs=$3, updated_at=now()
                WHERE project_id=$1 AND id=$2
                "#,
            )
            .bind(project_id)
            .bind(&node.id)
            .bind(&inputs)
            .execute(&mut *tx)
            .await?;
            "reopened".to_string()
        } else {
            sqlx::query(
                r#"
                UPDATE node SET task_state='failed', ready=false, owner=NULL, lease_token=NULL,
                  lease_expires=NULL, updated_at=now()
                WHERE project_id=$1 AND id=$2
                "#,
            )
            .bind(project_id)
            .bind(&node.id)
            .execute(&mut *tx)
            .await?;
            let ev_id = new_event_id();
            sqlx::query(
                r#"
                INSERT INTO event (id, project_id, node_id, actor, kind, payload)
                VALUES ($1,$2,$3,$4,'task.failed',$5)
                "#,
            )
            .bind(&ev_id)
            .bind(project_id)
            .bind(&node.id)
            .bind(actor)
            .bind(json!({ "reason": outcome, "failure": failure }))
            .execute(&mut *tx)
            .await?;
            "failed_final".to_string()
        };

        if next == "reopened" {
            let ev_id = new_event_id();
            sqlx::query(
                r#"
                INSERT INTO event (id, project_id, node_id, actor, kind, payload)
                VALUES ($1,$2,$3,$4,'task.validated',$5)
                "#,
            )
            .bind(&ev_id)
            .bind(project_id)
            .bind(&node.id)
            .bind(actor)
            .bind(json!({ "verdict": "failed", "failure": failure }))
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(next)
    }

    pub async fn fail_task(
        &self,
        project_id: &str,
        node_id: &str,
        actor: &str,
        lease_token: Uuid,
        reason: Value,
    ) -> StoreResult<Value> {
        let node = self.check_lease(project_id, node_id, lease_token).await?;
        let attempt = sqlx::query(
            r#"
            SELECT id, seq_no FROM attempt WHERE project_id=$1 AND node_id=$2
            ORDER BY seq_no DESC LIMIT 1
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_one(&self.pool)
        .await?;
        let next = self
            .fail_attempt(
                project_id,
                &node,
                actor,
                &attempt.get::<String, _>("id"),
                attempt.get("seq_no"),
                "error",
                reason,
            )
            .await?;
        Ok(json!({ "next": next }))
    }

    pub async fn reap_expired(&self) -> StoreResult<usize> {
        let mut tx = self.pool.begin().await?;
        let expired = sqlx::query(
            r#"
            SELECT project_id, id, owner, max_attempts FROM node
            WHERE task_state IN ('claimed','running','review')
              AND lease_expires IS NOT NULL AND lease_expires < now()
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut count = 0;
        for row in expired {
            let project_id: String = row.get("project_id");
            let node_id: String = row.get("id");
            let owner: Option<String> = row.get("owner");
            let max_attempts: i32 = row.get("max_attempts");

            let seq_no: i32 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(seq_no),0) FROM attempt WHERE project_id=$1 AND node_id=$2",
            )
            .bind(&project_id)
            .bind(&node_id)
            .fetch_one(&mut *tx)
            .await?;

            sqlx::query(
                r#"
                UPDATE attempt SET ended_at=now(), outcome='lease_expired'
                WHERE project_id=$1 AND node_id=$2 AND seq_no=$3 AND ended_at IS NULL
                "#,
            )
            .bind(&project_id)
            .bind(&node_id)
            .bind(seq_no)
            .execute(&mut *tx)
            .await?;

            let final_fail = seq_no >= max_attempts;
            if final_fail {
                sqlx::query(
                    r#"
                    UPDATE node SET task_state='failed', ready=false, owner=NULL,
                      lease_token=NULL, lease_expires=NULL, updated_at=now()
                    WHERE project_id=$1 AND id=$2
                    "#,
                )
                .bind(&project_id)
                .bind(&node_id)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query(
                    r#"
                    UPDATE node SET task_state='ready', ready=true, owner=NULL,
                      lease_token=NULL, lease_expires=NULL, updated_at=now()
                    WHERE project_id=$1 AND id=$2
                    "#,
                )
                .bind(&project_id)
                .bind(&node_id)
                .execute(&mut *tx)
                .await?;
            }

            let ev_id = new_event_id();
            sqlx::query(
                r#"
                INSERT INTO event (id, project_id, node_id, actor, kind, payload)
                VALUES ($1,$2,$3,'system:reaper','task.lease_expired',$4)
                "#,
            )
            .bind(&ev_id)
            .bind(&project_id)
            .bind(&node_id)
            .bind(json!({ "stale_owner": owner, "final": final_fail }))
            .execute(&mut *tx)
            .await?;
            count += 1;
        }
        tx.commit().await?;
        Ok(count)
    }

    pub async fn get_task(&self, project_id: &str, node_id: &str) -> StoreResult<Value> {
        let row = sqlx::query(
            r#"
            SELECT id, title, spec, inputs, write_scope, validators, task_state, ready,
                   owner, lease_expires, max_attempts, needs_replan, path, priority
            FROM node WHERE project_id=$1 AND id=$2
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("task {node_id}")))?;
        Ok(json!({
            "id": row.get::<String,_>("id"),
            "title": row.get::<String,_>("title"),
            "spec": row.get::<Value,_>("spec"),
            "inputs": row.get::<Value,_>("inputs"),
            "write_scope": row.get::<Vec<String>,_>("write_scope"),
            "validators": row.get::<Vec<String>,_>("validators"),
            "task_state": row.get::<Option<String>,_>("task_state"),
            "ready": row.get::<bool,_>("ready"),
            "owner": row.get::<Option<String>,_>("owner"),
            "lease_expires": row.get::<Option<DateTime<Utc>>,_>("lease_expires"),
            "max_attempts": row.get::<i32,_>("max_attempts"),
            "needs_replan": row.get::<bool,_>("needs_replan"),
            "path": row.get::<String,_>("path"),
            "priority": row.get::<i32,_>("priority"),
        }))
    }

    pub async fn get_graph(&self, project_id: &str, root: Option<&str>, depth: i32) -> StoreResult<Value> {
        let nodes = sqlx::query(
            r#"
            SELECT id, parent_id, path, kind, title, task_state, ready, scope_state, plan_state,
                   write_scope, needs_replan, graph_version
            FROM node WHERE project_id=$1
            ORDER BY path
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        let edges = sqlx::query("SELECT from_id, to_id FROM edge WHERE project_id=$1")
            .bind(project_id)
            .fetch_all(&self.pool)
            .await?;
        let version = self.current_graph_version(project_id).await?;
        let node_vals: Vec<Value> = nodes
            .iter()
            .filter(|r| {
                if let Some(root) = root {
                    let path: String = r.get("path");
                    let root_path = nodes
                        .iter()
                        .find(|n| n.get::<String, _>("id") == root)
                        .map(|n| n.get::<String, _>("path"));
                    match root_path {
                        Some(rp) => path == rp || path.starts_with(&format!("{rp}.")),
                        None => true,
                    }
                } else {
                    true
                }
            })
            .map(|r| {
                json!({
                    "id": r.get::<String,_>("id"),
                    "parent_id": r.get::<Option<String>,_>("parent_id"),
                    "path": r.get::<String,_>("path"),
                    "kind": r.get::<String,_>("kind"),
                    "title": r.get::<String,_>("title"),
                    "task_state": r.get::<Option<String>,_>("task_state"),
                    "ready": r.get::<bool,_>("ready"),
                    "scope_state": r.get::<String,_>("scope_state"),
                    "plan_state": r.get::<String,_>("plan_state"),
                    "write_scope": r.get::<Vec<String>,_>("write_scope"),
                    "needs_replan": r.get::<bool,_>("needs_replan"),
                })
            })
            .collect();
        let _ = depth;
        Ok(json!({
            "version": version,
            "nodes": node_vals,
            "edges": edges.iter().map(|e| json!({
                "from": e.get::<String,_>("from_id"),
                "to": e.get::<String,_>("to_id"),
            })).collect::<Vec<_>>(),
        }))
    }

    pub async fn change_scope(
        &self,
        project_id: &str,
        package_id: &str,
        actor: &str,
        action: &str,
        reason: &str,
        permanent: bool,
        force: bool,
    ) -> StoreResult<Value> {
        if actor.starts_with("agent:") {
            return Err(StoreError::Forbidden("HUMAN_ONLY".into()));
        }
        let new_state = match action {
            "pause" => "paused",
            "close" => "closed",
            "reopen" => "active",
            "archive" => "archived",
            other => {
                return Err(StoreError::Validation {
                    code: "INVALID_ACTION".into(),
                    message: format!("unknown action {other}"),
                    details: json!({}),
                })
            }
        };
        let mut tx = self.pool.begin().await?;
        let prev: Option<String> = sqlx::query_scalar(
            "SELECT scope_state FROM node WHERE project_id=$1 AND id=$2 AND kind='package'",
        )
        .bind(project_id)
        .bind(package_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(from) = prev else {
            return Err(StoreError::NotFound(format!("package {package_id}")));
        };
        sqlx::query(
            r#"
            UPDATE node SET scope_state=$3, permanent=$4, scope_reason=$5, scope_actor=$6, updated_at=now()
            WHERE project_id=$1 AND id=$2
            "#,
        )
        .bind(project_id)
        .bind(package_id)
        .bind(new_state)
        .bind(permanent)
        .bind(reason)
        .bind(actor)
        .execute(&mut *tx)
        .await?;

        if force && matches!(action, "close" | "pause") {
            let pkg_path: String = sqlx::query_scalar(
                "SELECT path FROM node WHERE project_id=$1 AND id=$2",
            )
            .bind(project_id)
            .bind(package_id)
            .fetch_one(&mut *tx)
            .await?;
            let running = sqlx::query(
                r#"
                SELECT id, write_scope FROM node
                WHERE project_id=$1 AND kind='task'
                  AND path LIKE $2 || '.%'
                  AND task_state IN ('claimed','running','review')
                "#,
            )
            .bind(project_id)
            .bind(&pkg_path)
            .fetch_all(&mut *tx)
            .await?;
            let repo: String = sqlx::query_scalar("SELECT repo_path FROM project WHERE id=$1")
                .bind(project_id)
                .fetch_one(&mut *tx)
                .await?;
            let git = GitWorkspace::new(repo);
            for r in running {
                let tid: String = r.get("id");
                let ws: Vec<String> = r.get("write_scope");
                let _ = git.checkout_paths(&ws);
                sqlx::query(
                    r#"
                    UPDATE node SET task_state='cancelled', ready=false, owner=NULL,
                      lease_token=NULL, lease_expires=NULL, updated_at=now()
                    WHERE project_id=$1 AND id=$2
                    "#,
                )
                .bind(project_id)
                .bind(&tid)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    r#"
                    UPDATE attempt SET ended_at=now(), outcome='cancelled'
                    WHERE project_id=$1 AND node_id=$2 AND ended_at IS NULL
                    "#,
                )
                .bind(project_id)
                .bind(&tid)
                .execute(&mut *tx)
                .await?;
                let ev_id = new_event_id();
                sqlx::query(
                    r#"
                    INSERT INTO event (id, project_id, node_id, actor, kind, payload)
                    VALUES ($1,$2,$3,$4,'task.cancelled',$5)
                    "#,
                )
                .bind(&ev_id)
                .bind(project_id)
                .bind(&tid)
                .bind(actor)
                .bind(json!({ "by": actor, "reason": "force scope" }))
                .execute(&mut *tx)
                .await?;
            }
        }

        let _ = apply_recompute_tx(&mut tx, project_id).await?;
        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,$3,$4,'package.scope_changed',$5)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(package_id)
        .bind(actor)
        .bind(json!({ "from": from, "to": new_state, "reason": reason, "permanent": permanent }))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({ "scope_state": new_state }))
    }

    pub async fn cancel_task(
        &self,
        project_id: &str,
        node_id: &str,
        actor: &str,
        force: bool,
    ) -> StoreResult<Value> {
        if actor.starts_with("agent:") {
            return Err(StoreError::Forbidden("HUMAN_ONLY".into()));
        }
        let mut tx = self.pool.begin().await?;
        let ws: Option<Vec<String>> = sqlx::query_scalar(
            "SELECT write_scope FROM node WHERE project_id=$1 AND id=$2",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(ws) = ws else {
            return Err(StoreError::NotFound(format!("task {node_id}")));
        };
        if force {
            let repo: String = sqlx::query_scalar("SELECT repo_path FROM project WHERE id=$1")
                .bind(project_id)
                .fetch_one(&mut *tx)
                .await?;
            let _ = GitWorkspace::new(repo).checkout_paths(&ws);
        }
        sqlx::query(
            r#"
            UPDATE node SET task_state='cancelled', ready=false, owner=NULL,
              lease_token=NULL, lease_expires=NULL, updated_at=now()
            WHERE project_id=$1 AND id=$2
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE attempt SET ended_at=now(), outcome='cancelled'
            WHERE project_id=$1 AND node_id=$2 AND ended_at IS NULL
            "#,
        )
        .bind(project_id)
        .bind(node_id)
        .execute(&mut *tx)
        .await?;
        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,$3,$4,'task.cancelled',$5)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(node_id)
        .bind(actor)
        .bind(json!({ "by": actor, "force": force }))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({ "cancelled": true }))
    }

    pub async fn publish_contract(
        &self,
        project_id: &str,
        artifact_id: &str,
        actor: &str,
        bump: &str,
        version: &str,
        node_id: &str,
    ) -> StoreResult<Value> {
        let mut tx = self.pool.begin().await?;
        if bump == "major" {
            sqlx::query(
                r#"
                INSERT INTO contract_pending (project_id, artifact_id, version, bump, node_id, approved)
                VALUES ($1,$2,$3,$4,$5,false)
                ON CONFLICT DO NOTHING
                "#,
            )
            .bind(project_id)
            .bind(artifact_id)
            .bind(version)
            .bind(bump)
            .bind(node_id)
            .execute(&mut *tx)
            .await?;
            let ev_id = new_event_id();
            sqlx::query(
                r#"
                INSERT INTO event (id, project_id, node_id, actor, kind, payload)
                VALUES ($1,$2,$3,$4,'contract.published',$5)
                "#,
            )
            .bind(&ev_id)
            .bind(project_id)
            .bind(node_id)
            .bind(actor)
            .bind(json!({ "artifact_id": artifact_id, "version": version, "bump": bump, "pending": true }))
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(json!({ "status": "pending_major", "version": version }));
        }
        sqlx::query("UPDATE artifact SET version=$2 WHERE id=$1 AND project_id=$3")
            .bind(artifact_id)
            .bind(version)
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,$3,$4,'contract.published',$5)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(node_id)
        .bind(actor)
        .bind(json!({ "artifact_id": artifact_id, "version": version, "bump": bump }))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({ "status": "published", "version": version }))
    }

    pub async fn approve_major(
        &self,
        project_id: &str,
        artifact_id: &str,
        actor: &str,
    ) -> StoreResult<Value> {
        if actor.starts_with("agent:") {
            return Err(StoreError::Forbidden("HUMAN_ONLY".into()));
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            UPDATE contract_pending SET approved=true
            WHERE project_id=$1 AND artifact_id=$2 AND approved=false
            RETURNING node_id, version
            "#,
        )
        .bind(project_id)
        .bind(artifact_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("pending major {artifact_id}")))?;

        let node_id: String = row.get("node_id");
        // mark transitive consumers needs_replan: downstream along edges from this node
        let mut affected = Vec::new();
        let mut frontier = vec![node_id.clone()];
        let mut seen = HashSet::new();
        while let Some(cur) = frontier.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            let downs: Vec<String> = sqlx::query_scalar(
                "SELECT to_id FROM edge WHERE project_id=$1 AND from_id=$2",
            )
            .bind(project_id)
            .bind(&cur)
            .fetch_all(&mut *tx)
            .await?;
            for d in downs {
                sqlx::query(
                    "UPDATE node SET needs_replan=true, updated_at=now() WHERE project_id=$1 AND id=$2",
                )
                .bind(project_id)
                .bind(&d)
                .execute(&mut *tx)
                .await?;
                affected.push(d.clone());
                frontier.push(d);
            }
        }
        let ev_id = new_event_id();
        sqlx::query(
            r#"
            INSERT INTO event (id, project_id, node_id, actor, kind, payload)
            VALUES ($1,$2,$3,$4,'contract.impact_marked',$5)
            "#,
        )
        .bind(&ev_id)
        .bind(project_id)
        .bind(&node_id)
        .bind(actor)
        .bind(json!({ "affected": affected, "artifact_id": artifact_id }))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({ "approved": true, "affected": affected }))
    }

    pub async fn list_events(
        &self,
        project_id: &str,
        after_seq: i64,
        node_id: Option<&str>,
        limit: i64,
    ) -> StoreResult<Vec<Value>> {
        let limit = if limit <= 0 { 100 } else { limit.min(500) };
        let rows = if let Some(nid) = node_id {
            sqlx::query(
                r#"
                SELECT id, seq, node_id, actor, kind, payload, created_at
                FROM event WHERE project_id=$1 AND seq > $2 AND node_id=$3
                ORDER BY seq LIMIT $4
                "#,
            )
            .bind(project_id)
            .bind(after_seq)
            .bind(nid)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                r#"
                SELECT id, seq, node_id, actor, kind, payload, created_at
                FROM event WHERE project_id=$1 AND seq > $2
                ORDER BY seq LIMIT $3
                "#,
            )
            .bind(project_id)
            .bind(after_seq)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.get::<String,_>("id"),
                    "seq": r.get::<i64,_>("seq"),
                    "node_id": r.get::<Option<String>,_>("node_id"),
                    "actor": r.get::<String,_>("actor"),
                    "kind": r.get::<String,_>("kind"),
                    "payload": r.get::<Value,_>("payload"),
                    "created_at": r.get::<DateTime<Utc>,_>("created_at"),
                })
            })
            .collect())
    }

    pub async fn get_artifact(&self, project_id: &str, id: &str) -> StoreResult<Value> {
        let r = sqlx::query(
            r#"
            SELECT id, node_id, attempt_id, paths, commit_hash, digest, version, published_at
            FROM artifact WHERE project_id=$1 AND id=$2
            "#,
        )
        .bind(project_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("artifact {id}")))?;
        Ok(json!({
            "id": r.get::<String,_>("id"),
            "node_id": r.get::<String,_>("node_id"),
            "attempt_id": r.get::<String,_>("attempt_id"),
            "paths": r.get::<Vec<String>,_>("paths"),
            "commit_hash": r.get::<String,_>("commit_hash"),
            "digest": r.get::<String,_>("digest"),
            "version": r.get::<Option<String>,_>("version"),
            "published_at": r.get::<DateTime<Utc>,_>("published_at"),
        }))
    }

    pub async fn replan_context(&self, project_id: &str, task_id: &str) -> StoreResult<Value> {
        let attempts = sqlx::query(
            r#"
            SELECT seq_no, owner, outcome, failure, handover, started_at, ended_at
            FROM attempt WHERE project_id=$1 AND node_id=$2 ORDER BY seq_no
            "#,
        )
        .bind(project_id)
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        let downs: Vec<String> = sqlx::query_scalar(
            "SELECT to_id FROM edge WHERE project_id=$1 AND from_id=$2",
        )
        .bind(project_id)
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(json!({
            "task_id": task_id,
            "attempts": attempts.iter().map(|a| json!({
                "seq_no": a.get::<i32,_>("seq_no"),
                "owner": a.get::<String,_>("owner"),
                "outcome": a.get::<Option<String>,_>("outcome"),
                "failure": a.get::<Option<Value>,_>("failure"),
            })).collect::<Vec<_>>(),
            "affected_downstream": downs,
        }))
    }

    /// Snapshot of rebuildable projection fields (for consistency tests).
    pub async fn snapshot_projection(&self, project_id: &str) -> StoreResult<Vec<Value>> {
        let rows = sqlx::query(
            r#"
            SELECT id, kind, task_state, ready, needs_replan, scope_state, owner,
                   lease_token::text AS lease_token, plan_state
            FROM node WHERE project_id = $1 ORDER BY id
            "#,
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.get::<String,_>("id"),
                    "kind": r.get::<String,_>("kind"),
                    "task_state": r.get::<Option<String>,_>("task_state"),
                    "ready": r.get::<bool,_>("ready"),
                    "needs_replan": r.get::<bool,_>("needs_replan"),
                    "scope_state": r.get::<String,_>("scope_state"),
                    "owner": r.get::<Option<String>,_>("owner"),
                    "lease_token": r.get::<Option<String>,_>("lease_token"),
                    "plan_state": r.get::<String,_>("plan_state"),
                })
            })
            .collect())
    }

    /// Rebuild projection from append-only event stream (A-03).
    ///
    /// Structural rows (node/edge identities, write_scope, paths) are retained —
    /// graph.published payloads currently store ids not full node bodies.
    /// All **state projection** fields (task_state, owner/lease, needs_replan,
    /// scope_state, ready) are reset then reapplied in `seq` order, then ready
    /// is recomputed.
    pub async fn rebuild_projection(&self, project_id: &str) -> StoreResult<Value> {
        let mut tx = self.pool.begin().await?;

        // 1) Reset rebuildable fields to event-stream baseline
        sqlx::query(
            r#"
            UPDATE node SET
              task_state = CASE WHEN kind = 'task' THEN 'todo' ELSE task_state END,
              ready = false,
              owner = NULL,
              lease_token = NULL,
              lease_expires = NULL,
              needs_replan = false,
              scope_state = CASE WHEN kind = 'package' THEN 'active' ELSE scope_state END,
              updated_at = now()
            WHERE project_id = $1
            "#,
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await?;

        // 2) Replay events in seq order
        let events = sqlx::query(
            r#"
            SELECT seq, node_id, kind, payload
            FROM event WHERE project_id = $1 ORDER BY seq
            "#,
        )
        .bind(project_id)
        .fetch_all(&mut *tx)
        .await?;

        let mut applied = 0i64;
        for ev in events {
            let kind: String = ev.get("kind");
            let node_id: Option<String> = ev.get("node_id");
            let payload: Value = ev.get("payload");
            match kind.as_str() {
                "graph.published" => {
                    // nodes already exist structurally; baseline todo is fine
                    applied += 1;
                }
                "task.claimed" => {
                    if let Some(nid) = &node_id {
                        let owner = payload
                            .get("owner")
                            .and_then(|x| x.as_str())
                            .unwrap_or("unknown");
                        let token = payload
                            .get("lease_token")
                            .and_then(|x| x.as_str())
                            .and_then(|s| Uuid::parse_str(s).ok());
                        sqlx::query(
                            r#"
                            UPDATE node SET task_state='claimed', ready=false, owner=$3,
                              lease_token=$4, updated_at=now()
                            WHERE project_id=$1 AND id=$2
                            "#,
                        )
                        .bind(project_id)
                        .bind(nid)
                        .bind(owner)
                        .bind(token)
                        .execute(&mut *tx)
                        .await?;
                        applied += 1;
                    }
                }
                "task.done" => {
                    if let Some(nid) = &node_id {
                        sqlx::query(
                            r#"
                            UPDATE node SET task_state='done', ready=false, owner=NULL,
                              lease_token=NULL, lease_expires=NULL, updated_at=now()
                            WHERE project_id=$1 AND id=$2
                            "#,
                        )
                        .bind(project_id)
                        .bind(nid)
                        .execute(&mut *tx)
                        .await?;
                        applied += 1;
                    }
                }
                "task.failed" => {
                    if let Some(nid) = &node_id {
                        sqlx::query(
                            r#"
                            UPDATE node SET task_state='failed', ready=false, owner=NULL,
                              lease_token=NULL, lease_expires=NULL, updated_at=now()
                            WHERE project_id=$1 AND id=$2
                            "#,
                        )
                        .bind(project_id)
                        .bind(nid)
                        .execute(&mut *tx)
                        .await?;
                        applied += 1;
                    }
                }
                "task.lease_expired" => {
                    if let Some(nid) = &node_id {
                        let final_fail = payload
                            .get("final")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false);
                        if final_fail {
                            sqlx::query(
                                r#"
                                UPDATE node SET task_state='failed', ready=false, owner=NULL,
                                  lease_token=NULL, lease_expires=NULL, updated_at=now()
                                WHERE project_id=$1 AND id=$2
                                "#,
                            )
                            .bind(project_id)
                            .bind(nid)
                            .execute(&mut *tx)
                            .await?;
                        } else {
                            sqlx::query(
                                r#"
                                UPDATE node SET task_state='ready', ready=true, owner=NULL,
                                  lease_token=NULL, lease_expires=NULL, updated_at=now()
                                WHERE project_id=$1 AND id=$2
                                "#,
                            )
                            .bind(project_id)
                            .bind(nid)
                            .execute(&mut *tx)
                            .await?;
                        }
                        applied += 1;
                    }
                }
                "task.cancelled" => {
                    if let Some(nid) = &node_id {
                        sqlx::query(
                            r#"
                            UPDATE node SET task_state='cancelled', ready=false, owner=NULL,
                              lease_token=NULL, lease_expires=NULL, updated_at=now()
                            WHERE project_id=$1 AND id=$2
                            "#,
                        )
                        .bind(project_id)
                        .bind(nid)
                        .execute(&mut *tx)
                        .await?;
                        applied += 1;
                    }
                }
                "task.validated" => {
                    // validation failure path reopens to ready when not final
                    if let Some(nid) = &node_id {
                        let verdict = payload
                            .get("verdict")
                            .and_then(|x| x.as_str())
                            .unwrap_or("");
                        if verdict == "failed" {
                            sqlx::query(
                                r#"
                                UPDATE node SET task_state='ready', ready=true, owner=NULL,
                                  lease_token=NULL, lease_expires=NULL, updated_at=now()
                                WHERE project_id=$1 AND id=$2 AND task_state IS DISTINCT FROM 'failed'
                                "#,
                            )
                            .bind(project_id)
                            .bind(nid)
                            .execute(&mut *tx)
                            .await?;
                            applied += 1;
                        }
                    }
                }
                "package.scope_changed" => {
                    if let Some(nid) = &node_id {
                        let to = payload
                            .get("to")
                            .and_then(|x| x.as_str())
                            .unwrap_or("active");
                        sqlx::query(
                            r#"
                            UPDATE node SET scope_state=$3, updated_at=now()
                            WHERE project_id=$1 AND id=$2
                            "#,
                        )
                        .bind(project_id)
                        .bind(nid)
                        .bind(to)
                        .execute(&mut *tx)
                        .await?;
                        applied += 1;
                    }
                }
                "contract.impact_marked" => {
                    if let Some(arr) = payload.get("affected").and_then(|a| a.as_array()) {
                        for a in arr {
                            if let Some(nid) = a.as_str() {
                                sqlx::query(
                                    r#"
                                    UPDATE node SET needs_replan=true, updated_at=now()
                                    WHERE project_id=$1 AND id=$2
                                    "#,
                                )
                                .bind(project_id)
                                .bind(nid)
                                .execute(&mut *tx)
                                .await?;
                            }
                        }
                        applied += 1;
                    }
                }
                // heartbeats not stored as events; handover_reported does not change node columns
                _ => {}
            }
        }

        // 3) Ready column from dual-implementation SQL
        let ready = apply_recompute_tx(&mut tx, project_id).await?;
        tx.commit().await?;

        let consistent = crate::ready_maint::verify_ready_consistent(&self.pool, project_id).await?;
        Ok(json!({
            "events_applied": applied,
            "ready_count": ready.len(),
            "consistent": consistent,
            "ready": ready,
        }))
    }
}

fn inject_failure_inputs(inputs: &Value, failure: &Value) -> Value {
    let mut arr = inputs.as_array().cloned().unwrap_or_default();
    arr.push(json!({ "type": "previous_failure", "failure": failure }));
    Value::Array(arr)
}

// re-export for tests
pub use sunmao_core::graph::GraphViolation as GViolation;
