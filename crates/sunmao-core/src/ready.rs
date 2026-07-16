//! Ready derivation (pure): task_state==todo AND all upstream tasks done AND no restricting ancestor.
//! Dual implementation of the SQL precomputed column (A-04).

use crate::state_machine::{ScopeState, TaskState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyInput {
    pub node_id: String,
    pub kind: String, // "task" | "package"
    pub task_state: Option<TaskState>,
    pub path: String,
    /// Upstream task states keyed by from_id (only task edges matter for ready).
    pub upstream_task_states: Vec<(String, TaskState)>,
    /// Ancestor packages that restrict: (path, scope_state)
    pub ancestor_scopes: Vec<(String, ScopeState)>,
}

/// Pure ready predicate for a single node.
pub fn compute_ready(input: &ReadyInput) -> bool {
    if input.kind != "task" {
        return false;
    }
    let Some(ts) = input.task_state else {
        return false;
    };
    // Ready column is true only when storage says todo and deps/ancestors allow
    // (precomputed ready drives claim; transition todo→ready is implicit via this flag).
    if !matches!(ts, TaskState::Todo | TaskState::Ready) {
        return false;
    }
    if matches!(ts, TaskState::Ready) {
        // Already marked ready in storage — still recompute eligibility
    }
    // All upstream tasks must be done
    for (_id, st) in &input.upstream_task_states {
        if *st != TaskState::Done {
            return false;
        }
    }
    // No restricting ancestor
    for (anc_path, scope) in &input.ancestor_scopes {
        if input.path.starts_with(&format!("{anc_path}.")) || input.path.starts_with(&format!("{anc_path}/")) {
            if scope.is_restricting() {
                return false;
            }
        } else if input.path != *anc_path && is_path_descendant(&input.path, anc_path) && scope.is_restricting()
        {
            return false;
        }
    }
    // Also check ancestor by dotted path convention
    for (anc_path, scope) in &input.ancestor_scopes {
        if is_path_descendant(&input.path, anc_path) && scope.is_restricting() {
            return false;
        }
    }
    true
}

fn is_path_descendant(child: &str, ancestor: &str) -> bool {
    if child == ancestor {
        return false;
    }
    child.starts_with(&format!("{ancestor}."))
}

/// Compute ready for many nodes given full snapshot (for property tests vs SQL).
pub fn compute_all_ready(
    nodes: &[(String, String, Option<TaskState>, String)], // id, kind, task_state, path
    edges: &[(String, String)],                            // from, to
    packages: &[(String, String, ScopeState)],             // id, path, scope
) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for (id, kind, task_state, path) in nodes {
        let upstream: Vec<(String, TaskState)> = edges
            .iter()
            .filter(|(_, to)| to == id)
            .filter_map(|(from, _)| {
                nodes
                    .iter()
                    .find(|(nid, k, _, _)| nid == from && k == "task")
                    .and_then(|(nid, _, ts, _)| ts.map(|s| (nid.clone(), s)))
            })
            .collect();
        let ancestors: Vec<(String, ScopeState)> = packages
            .iter()
            .filter(|(_, ppath, _)| is_path_descendant(path, ppath) || path.starts_with(&format!("{ppath}.")))
            .map(|(_, ppath, sc)| (ppath.clone(), *sc))
            .collect();
        let input = ReadyInput {
            node_id: id.clone(),
            kind: kind.clone(),
            task_state: *task_state,
            path: path.clone(),
            upstream_task_states: upstream,
            ancestor_scopes: ancestors,
        };
        out.push((id.clone(), compute_ready(&input)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_todo_no_deps_is_ready() {
        let input = ReadyInput {
            node_id: "nd_a".into(),
            kind: "task".into(),
            task_state: Some(TaskState::Todo),
            path: "root.a".into(),
            upstream_task_states: vec![],
            ancestor_scopes: vec![("root".into(), ScopeState::Active)],
        };
        assert!(compute_ready(&input));
    }

    #[test]
    fn blocked_by_upstream() {
        let input = ReadyInput {
            node_id: "nd_b".into(),
            kind: "task".into(),
            task_state: Some(TaskState::Todo),
            path: "root.b".into(),
            upstream_task_states: vec![("nd_a".into(), TaskState::Todo)],
            ancestor_scopes: vec![],
        };
        assert!(!compute_ready(&input));
    }

    #[test]
    fn blocked_by_closed_ancestor() {
        let input = ReadyInput {
            node_id: "nd_a".into(),
            kind: "task".into(),
            task_state: Some(TaskState::Todo),
            path: "root.mod.a".into(),
            upstream_task_states: vec![],
            ancestor_scopes: vec![("root.mod".into(), ScopeState::Closed)],
        };
        assert!(!compute_ready(&input));
    }

    #[test]
    fn upstream_done_makes_ready() {
        let input = ReadyInput {
            node_id: "nd_b".into(),
            kind: "task".into(),
            task_state: Some(TaskState::Todo),
            path: "root.b".into(),
            upstream_task_states: vec![("nd_a".into(), TaskState::Done)],
            ancestor_scopes: vec![],
        };
        assert!(compute_ready(&input));
    }
}
