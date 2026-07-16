//! Integration tests against real Postgres + git (M1–M4).

use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sunmao_core::graph::{EdgeDraft, NodeDraft, PublishInput};
use sunmao_store::git::GitWorkspace;
use sunmao_store::projects::ProjectsRepo;
use sunmao_store::Store;
use uuid::Uuid;
// sqlx used for corruption in rebuild test

fn db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://sunmao@localhost/sunmao".into())
}

async fn setup() -> (Store, String, tempfile::TempDir) {
    let pool = PgPoolOptions::new()
        .max_connections(40)
        .connect(&db_url())
        .await
        .expect("pg");
    Store::migrate(&pool).await.expect("migrate");
    let store = Store::new(pool.clone());

    let dir = tempfile::tempdir().unwrap();
    let git = GitWorkspace::new(dir.path());
    git.ensure_repo().unwrap();
    git.write_file("README.md", "root\n").unwrap();
    git.commit_paths(&["README.md".into()], "init").unwrap();

    let name = format!("it-{}", Uuid::new_v4());
    let proj = ProjectsRepo::new(&pool)
        .create_or_get(&name, &dir.path().display().to_string())
        .await
        .unwrap();
    (store, proj.id, dir)
}

fn task(id: &str, title: &str, scope: &str, path: &str) -> NodeDraft {
    NodeDraft {
        id: id.into(),
        parent_id: None,
        kind: "task".into(),
        title: title.into(),
        layer: None,
        role: None,
        spec: serde_json::json!({"goal": title}),
        write_scope: vec![scope.into()],
        required_caps: vec![],
        validators: vec!["scope-diff".into()],
        inputs: serde_json::json!([]),
        max_attempts: 3,
        priority: 0,
        path: Some(path.into()),
    }
}

#[tokio::test]
async fn m1_publish_illegal_returns_422_shape() {
    let (store, pid, _dir) = setup().await;
    let input = PublishInput {
        base_version: 0,
        summary: "bad".into(),
        upsert_nodes: vec![
            task("nd_a", "A", "src/", "a"),
            task("nd_b", "B", "src/api/", "b"),
        ],
        add_edges: vec![],
        remove_nodes: vec![],
    };
    let err = store
        .publish_graph(&pid, "agent:planner", input)
        .await
        .unwrap_err();
    match err {
        sunmao_store::StoreError::Validation { code, details, .. } => {
            assert_eq!(code, "GRAPH_INVALID");
            assert!(details.get("violations").is_some());
        }
        other => panic!("expected validation, got {other}"),
    }
}

#[tokio::test]
async fn m1_publish_legal_ready_and_stale_version() {
    let (store, pid, _dir) = setup().await;
    let input = PublishInput {
        base_version: 0,
        summary: "chain".into(),
        upsert_nodes: vec![
            task("nd_a", "A", "a/", "a"),
            task("nd_b", "B", "b/", "b"),
            task("nd_c", "C", "c/", "c"),
        ],
        add_edges: vec![
            EdgeDraft {
                from: "nd_a".into(),
                to: "nd_b".into(),
            },
            EdgeDraft {
                from: "nd_b".into(),
                to: "nd_c".into(),
            },
        ],
        remove_nodes: vec![],
    };
    let r = store
        .publish_graph(&pid, "agent:planner", input)
        .await
        .unwrap();
    assert_eq!(r["version"], 1);
    let ready: Vec<String> = r["ready_now"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert_eq!(ready, vec!["nd_a".to_string()]);

    // stale
    let stale = PublishInput {
        base_version: 0,
        summary: "stale".into(),
        upsert_nodes: vec![],
        add_edges: vec![],
        remove_nodes: vec![],
    };
    let err = store
        .publish_graph(&pid, "agent:planner", stale)
        .await
        .unwrap_err();
    match err {
        sunmao_store::StoreError::Conflict { code, .. } => assert_eq!(code, "STALE_GRAPH_VERSION"),
        other => panic!("{other}"),
    }

    let ok = sunmao_store::ready_maint::verify_ready_consistent(&store.pool, &pid)
        .await
        .unwrap();
    assert!(ok);
}

#[tokio::test]
async fn m2_concurrent_claim_unique_owner() {
    // 验收：50 Worker 同时抢 100 个无依赖任务，无双领（owner 唯一）。
    let (store, pid, _dir) = setup().await;
    let n_tasks = 100usize;
    let n_workers = 50usize;
    let mut nodes = Vec::new();
    for i in 0..n_tasks {
        nodes.push(task(
            &format!("nd_t{i:03}"),
            &format!("T{i}"),
            &format!("w{i}/"),
            &format!("t{i}"),
        ));
    }
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "parallel-100".into(),
                upsert_nodes: nodes,
                add_edges: vec![],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();

    let store = Arc::new(store);
    let mut handles = Vec::new();
    for w in 0..n_workers {
        let s = store.clone();
        let pid = pid.clone();
        handles.push(tokio::spawn(async move {
            let actor = format!("agent:w{w}");
            let mut mine = Vec::new();
            // 每 worker 循环 claim 直到空，覆盖 50w×100t
            loop {
                match s.claim_next(&pid, &actor, &[], 60).await.unwrap() {
                    Some(c) => mine.push(c.task.id),
                    None => break,
                }
            }
            mine
        }));
    }
    let mut claimed = Vec::new();
    for h in handles {
        claimed.extend(h.await.unwrap());
    }
    claimed.sort();
    let mut uniq = claimed.clone();
    uniq.dedup();
    assert_eq!(
        claimed.len(),
        uniq.len(),
        "double claim detected: total={} unique={}",
        claimed.len(),
        uniq.len()
    );
    assert_eq!(claimed.len(), n_tasks, "expected all tasks claimed");
}

#[tokio::test]
async fn m2_lease_expire_and_fencing() {
    let (store, pid, _dir) = setup().await;
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "lease".into(),
                upsert_nodes: vec![task("nd_l", "L", "lease/", "l")],
                add_edges: vec![],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();

    let c = store
        .claim_next(&pid, "agent:w1", &[], 1)
        .await
        .unwrap()
        .expect("claim");
    let token = c.lease.token;

    // wait expire + reap (remote PG clock; allow a little slack)
    let mut n = 0;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        n = store.reap_expired().await.unwrap();
        if n >= 1 {
            break;
        }
    }
    assert!(n >= 1, "reaper should reclaim expired lease");

    // fencing: old token rejected
    let err = store
        .heartbeat(&pid, "nd_l", "agent:w1", token, 30)
        .await
        .unwrap_err();
    match err {
        sunmao_store::StoreError::Conflict { code, .. } => assert_eq!(code, "LEASE_LOST"),
        other => panic!("{other}"),
    }

    // re-claim attempt_seq=2
    let c2 = store
        .claim_next(&pid, "agent:w2", &[], 30)
        .await
        .unwrap()
        .expect("reclaim");
    assert_eq!(c2.task.attempt_seq, 2);
    assert!(c2.handover.is_some());
}

#[tokio::test]
async fn m3_abc_chain_and_oos_write() {
    let (store, pid, dir) = setup().await;
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "abc".into(),
                upsert_nodes: vec![
                    task("nd_a", "A", "a/", "a"),
                    task("nd_b", "B", "b/", "b"),
                    task("nd_c", "C", "c/", "c"),
                ],
                add_edges: vec![
                    EdgeDraft {
                        from: "nd_a".into(),
                        to: "nd_b".into(),
                    },
                    EdgeDraft {
                        from: "nd_b".into(),
                        to: "nd_c".into(),
                    },
                ],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();

    let git = GitWorkspace::new(dir.path());

    // B not claimable yet
    let only_a = store
        .claim_next(&pid, "agent:w", &[], 60)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(only_a.task.id, "nd_a");
    assert!(store
        .claim_next(&pid, "agent:w2", &[], 60)
        .await
        .unwrap()
        .is_none());

    git.write_file("a/out.txt", "A done\n").unwrap();
    let r = store
        .submit(&pid, "nd_a", "agent:w", only_a.lease.token, None)
        .await
        .unwrap();
    assert_eq!(r.verdict, "done");

    let b = store
        .claim_next(&pid, "agent:w", &[], 60)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.task.id, "nd_b");

    // out of scope write → fail reopen
    git.write_file("docs/bad.md", "oops\n").unwrap();
    git.write_file("b/out.txt", "B\n").unwrap();
    let r = store
        .submit(&pid, "nd_b", "agent:w", b.lease.token, None)
        .await
        .unwrap();
    assert_eq!(r.verdict, "failed");
    assert_eq!(r.next.as_deref(), Some("reopened"));

    // clean and finish B
    let _ = git.checkout_paths(&["docs/bad.md".into()]);
    let _ = std::fs::remove_file(dir.path().join("docs/bad.md"));
    // reclaim B
    let b2 = store
        .claim_next(&pid, "agent:w", &[], 60)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b2.task.id, "nd_b");
    if b2.handover.is_some() {
        store
            .handover_review(
                &pid,
                "nd_b",
                "agent:w",
                b2.lease.token,
                serde_json::json!({
                    "lease_token": b2.lease.token,
                    "wip_assessment": "clean",
                    "decision": "reuse",
                    "discarded_paths": [],
                    "concerns": ""
                }),
            )
            .await
            .unwrap();
    }
    git.write_file("b/out.txt", "B done\n").unwrap();
    let r = store
        .submit(&pid, "nd_b", "agent:w", b2.lease.token, None)
        .await
        .unwrap();
    assert_eq!(r.verdict, "done");

    let c = store
        .claim_next(&pid, "agent:w", &[], 60)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.task.id, "nd_c");
    git.write_file("c/out.txt", "C done\n").unwrap();
    let r = store
        .submit(&pid, "nd_c", "agent:w", c.lease.token, None)
        .await
        .unwrap();
    assert_eq!(r.verdict, "done");

    // 3 task commits + init
    let log = std::process::Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(log.contains("task(nd_a)"));
    assert!(log.contains("task(nd_b)"));
    assert!(log.contains("task(nd_c)"));
}

#[tokio::test]
async fn m4_contract_major_and_events_isolation() {
    let (store, pid, _dir) = setup().await;
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "c".into(),
                upsert_nodes: vec![
                    task("nd_up", "Up", "up/", "up"),
                    task("nd_down", "Down", "down/", "down"),
                ],
                add_edges: vec![EdgeDraft {
                    from: "nd_up".into(),
                    to: "nd_down".into(),
                }],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();

    // fake artifact id for contract publish path
    let art = "ar_test_major";
    let r = store
        .publish_contract(&pid, art, "agent:planner", "major", "2.0.0", "nd_up")
        .await
        .unwrap();
    assert_eq!(r["status"], "pending_major");

    let err = store
        .approve_major(&pid, art, "agent:evil")
        .await
        .unwrap_err();
    match err {
        sunmao_store::StoreError::Forbidden(_) => {}
        other => panic!("{other}"),
    }

    let r = store
        .approve_major(&pid, art, "human")
        .await
        .unwrap();
    assert_eq!(r["approved"], true);
    let affected = r["affected"].as_array().unwrap();
    assert!(affected.iter().any(|x| x.as_str() == Some("nd_down")));

    let t = store.get_task(&pid, "nd_down").await.unwrap();
    assert_eq!(t["needs_replan"], true);

    let events = store.list_events(&pid, 0, None, 100).await.unwrap();
    assert!(!events.is_empty());

    // second project isolation: events empty for other project until activity
    let pool = store.pool.clone();
    let dir2 = tempfile::tempdir().unwrap();
    let git = GitWorkspace::new(dir2.path());
    git.ensure_repo().unwrap();
    git.write_file("x", "x").unwrap();
    git.commit_paths(&["x".into()], "i").unwrap();
    let p2 = ProjectsRepo::new(&pool)
        .create_or_get("other", &dir2.path().display().to_string())
        .await
        .unwrap();
    let e2 = store.list_events(&p2.id, 0, None, 100).await.unwrap();
    assert!(e2.is_empty());
}

#[tokio::test]
async fn m4_events_after_seq_resume() {
    let (store, pid, _dir) = setup().await;
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "e1".into(),
                upsert_nodes: vec![task("nd_x", "X", "x/", "x")],
                add_edges: vec![],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();
    let all = store.list_events(&pid, 0, None, 100).await.unwrap();
    assert!(!all.is_empty());
    let mid = all[0]["seq"].as_i64().unwrap();
    let resumed = store.list_events(&pid, mid, None, 100).await.unwrap();
    assert!(resumed.iter().all(|e| e["seq"].as_i64().unwrap() > mid));
    // after last seq → empty backlog (SSE reconnect resume)
    let last = all.last().unwrap()["seq"].as_i64().unwrap();
    let empty = store.list_events(&pid, last, None, 100).await.unwrap();
    assert!(empty.is_empty());

    let rebuilt = store.rebuild_projection(&pid).await.unwrap();
    assert_eq!(rebuilt["consistent"], true);
    assert!(rebuilt["events_applied"].as_i64().unwrap() >= 1);
}

#[tokio::test]
async fn m4_rebuild_projection_restores_after_corruption() {
    // A-03: event stream rebuild — corrupt projection, rebuild, match prior state.
    let (store, pid, dir) = setup().await;
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "rebuild".into(),
                upsert_nodes: vec![
                    task("nd_a", "A", "a/", "a"),
                    task("nd_b", "B", "b/", "b"),
                ],
                add_edges: vec![EdgeDraft {
                    from: "nd_a".into(),
                    to: "nd_b".into(),
                }],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();

    let git = GitWorkspace::new(dir.path());
    let ca = store
        .claim_next(&pid, "agent:w1", &[], 120)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ca.task.id, "nd_a");
    git.write_file("a/out.txt", "A\n").unwrap();
    let sa = store
        .submit(&pid, "nd_a", "agent:w1", ca.lease.token, None)
        .await
        .unwrap();
    assert_eq!(sa.verdict, "done");

    let cb = store
        .claim_next(&pid, "agent:w2", &[], 120)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cb.task.id, "nd_b");

    // contract impact mark
    let _ = store
        .publish_contract(&pid, "ar_rebuild", "agent:planner", "major", "2.0.0", "nd_a")
        .await
        .unwrap();
    let _ = store
        .approve_major(&pid, "ar_rebuild", "human")
        .await
        .unwrap();

    let before = store.snapshot_projection(&pid).await.unwrap();
    let a_before = before.iter().find(|n| n["id"] == "nd_a").unwrap().clone();
    let b_before = before.iter().find(|n| n["id"] == "nd_b").unwrap().clone();
    assert_eq!(a_before["task_state"], "done");
    assert_eq!(b_before["task_state"], "claimed");
    assert_eq!(b_before["needs_replan"], true);

    // Corrupt projection (simulate drift / disaster)
    sqlx::query(
        r#"
        UPDATE node SET task_state='todo', ready=true, owner=NULL, needs_replan=false
        WHERE project_id=$1 AND id='nd_a'
        "#,
    )
    .bind(&pid)
    .execute(&store.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"
        UPDATE node SET task_state='todo', ready=true, owner=NULL, lease_token=NULL, needs_replan=false
        WHERE project_id=$1 AND id='nd_b'
        "#,
    )
    .bind(&pid)
    .execute(&store.pool)
    .await
    .unwrap();

    let mid = store.snapshot_projection(&pid).await.unwrap();
    assert_eq!(
        mid.iter().find(|n| n["id"] == "nd_a").unwrap()["task_state"],
        "todo"
    );

    let rebuilt = store.rebuild_projection(&pid).await.unwrap();
    assert_eq!(rebuilt["consistent"], true);
    assert!(rebuilt["events_applied"].as_i64().unwrap() >= 3);

    let after = store.snapshot_projection(&pid).await.unwrap();
    let a_after = after.iter().find(|n| n["id"] == "nd_a").unwrap();
    let b_after = after.iter().find(|n| n["id"] == "nd_b").unwrap();
    assert_eq!(a_after["task_state"], a_before["task_state"]);
    assert_eq!(a_after["ready"], a_before["ready"]);
    assert_eq!(b_after["task_state"], b_before["task_state"]);
    assert_eq!(b_after["needs_replan"], b_before["needs_replan"]);
    assert_eq!(b_after["owner"], b_before["owner"]);
}

#[tokio::test]
async fn m3_drain_force_cancels_running() {
    let (store, pid, dir) = setup().await;
    store
        .publish_graph(
            &pid,
            "agent:planner",
            PublishInput {
                base_version: 0,
                summary: "pkg".into(),
                upsert_nodes: vec![
                    NodeDraft {
                        id: "nd_pkg".into(),
                        parent_id: None,
                        kind: "package".into(),
                        title: "P".into(),
                        layer: None,
                        role: None,
                        spec: serde_json::json!({}),
                        write_scope: vec![],
                        required_caps: vec![],
                        validators: vec![],
                        inputs: serde_json::json!([]),
                        max_attempts: 3,
                        priority: 0,
                        path: Some("root".into()),
                    },
                    {
                        let mut t = task("nd_t", "T", "scope/", "root.t");
                        t.parent_id = Some("nd_pkg".into());
                        t
                    },
                ],
                add_edges: vec![],
                remove_nodes: vec![],
            },
        )
        .await
        .unwrap();
    let c = store
        .claim_next(&pid, "agent:w", &[], 60)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.task.id, "nd_t");
    // create WIP then force-close
    let git = GitWorkspace::new(dir.path());
    git.write_file("scope/wip.rs", "wip\n").unwrap();
    let r = store
        .change_scope(&pid, "nd_pkg", "human", "close", "stop", false, true)
        .await
        .unwrap();
    assert_eq!(r["scope_state"], "closed");
    let t = store.get_task(&pid, "nd_t").await.unwrap();
    assert_eq!(t["task_state"], "cancelled");
    // agent cannot scope
    let err = store
        .change_scope(&pid, "nd_pkg", "agent:x", "reopen", "no", false, false)
        .await
        .unwrap_err();
    assert!(matches!(err, sunmao_store::StoreError::Forbidden(_)));
}
