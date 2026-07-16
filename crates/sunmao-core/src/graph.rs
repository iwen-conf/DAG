//! Graph compile validation (FR-03.3) and publish helpers.

use crate::state_machine::TaskState;
use crate::write_scope::scopes_conflict;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Built-in validator names registered in v1 (FR-08.1).
pub const BUILTIN_VALIDATORS: &[&str] = &["scope-diff", "cmd", "artifact-exists", "cargo-check", "true"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDraft {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String, // package | task
    pub title: String,
    #[serde(default)]
    pub layer: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub spec: serde_json::Value,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub required_caps: Vec<String>,
    #[serde(default)]
    pub validators: Vec<String>,
    #[serde(default)]
    pub inputs: serde_json::Value,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: i32,
    #[serde(default)]
    pub priority: i32,
    /// Materialized path; if empty, derived from parent path + id suffix.
    #[serde(default)]
    pub path: Option<String>,
}

fn default_max_attempts() -> i32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeDraft {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishInput {
    pub base_version: i64,
    pub summary: String,
    #[serde(default)]
    pub upsert_nodes: Vec<NodeDraft>,
    #[serde(default)]
    pub add_edges: Vec<EdgeDraft>,
    #[serde(default)]
    pub remove_nodes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum GraphViolation {
    Cycle { path: Vec<String> },
    DanglingDep { edge: EdgeDraft },
    DuplicateId { id: String },
    TypeInvariant { id: String, message: String },
    WriteConflict { nodes: Vec<String>, scope: String },
    MissingInput { id: String, message: String },
    ValidatorUnregistered { id: String, validator: String },
    UnknownRemove { id: String },
    SelfEdge { id: String },
    OrphanParent { id: String, parent_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidationReport {
    pub violations: Vec<GraphViolation>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    pub version: i64,
    /// Existing nodes: id -> (kind, path, parent_id, write_scope, validators, task_state, title)
    pub nodes: HashMap<String, ExistingNode>,
    pub edges: HashSet<(String, String)>,
    pub registered_validators: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct ExistingNode {
    pub kind: String,
    pub path: String,
    pub parent_id: Option<String>,
    pub write_scope: Vec<String>,
    pub validators: Vec<String>,
    pub task_state: Option<TaskState>,
    pub title: String,
    pub has_attempt_history: bool,
    pub inputs: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub version: i64,
    pub ready_now: Vec<String>,
}

/// Validate a publish against the current snapshot. Returns all violations (FR-03.4).
pub fn validate_publish(snapshot: &GraphSnapshot, input: &PublishInput) -> ValidationReport {
    let mut report = ValidationReport::default();
    let mut nodes: HashMap<String, ExistingNode> = snapshot.nodes.clone();
    let mut edges = snapshot.edges.clone();

    // Apply removals virtually
    for id in &input.remove_nodes {
        match nodes.get(id) {
            None => report.violations.push(GraphViolation::UnknownRemove { id: id.clone() }),
            Some(n) if n.has_attempt_history => report.violations.push(GraphViolation::TypeInvariant {
                id: id.clone(),
                message: "cannot remove node with attempt history".into(),
            }),
            Some(_) => {
                nodes.remove(id);
                edges.retain(|(f, t)| f != id && t != id);
            }
        }
    }

    // Duplicate IDs within upsert batch
    let mut seen_upsert = HashSet::new();
    for n in &input.upsert_nodes {
        if !seen_upsert.insert(n.id.clone()) {
            report.violations.push(GraphViolation::DuplicateId { id: n.id.clone() });
        }
    }

    // Upsert nodes
    for n in &input.upsert_nodes {
        if n.kind != "package" && n.kind != "task" {
            report.violations.push(GraphViolation::TypeInvariant {
                id: n.id.clone(),
                message: format!("unknown kind {}", n.kind),
            });
            continue;
        }
        if n.title.trim().is_empty() {
            report.violations.push(GraphViolation::TypeInvariant {
                id: n.id.clone(),
                message: "title must be non-empty".into(),
            });
        }
        if n.kind == "task" && n.write_scope.is_empty() {
            report.violations.push(GraphViolation::TypeInvariant {
                id: n.id.clone(),
                message: "task must declare write_scope".into(),
            });
        }
        // Validator registration
        for v in &n.validators {
            if !snapshot.registered_validators.contains(v) {
                report.violations.push(GraphViolation::ValidatorUnregistered {
                    id: n.id.clone(),
                    validator: v.clone(),
                });
            }
        }
        // Missing inputs: if inputs is array with null artifact refs
        if n.kind == "task" {
            if let Some(arr) = n.inputs.as_array() {
                for (i, item) in arr.iter().enumerate() {
                    if item.get("artifact_id").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                        report.violations.push(GraphViolation::MissingInput {
                            id: n.id.clone(),
                            message: format!("inputs[{i}] missing artifact_id"),
                        });
                    }
                }
            }
        }
        // Parent existence (after merges)
        if let Some(pid) = &n.parent_id {
            let parent_ok = nodes.contains_key(pid)
                || input.upsert_nodes.iter().any(|u| &u.id == pid);
            if !parent_ok {
                report.violations.push(GraphViolation::OrphanParent {
                    id: n.id.clone(),
                    parent_id: pid.clone(),
                });
            }
        }
        let path = n.path.clone().unwrap_or_else(|| {
            if let Some(pid) = &n.parent_id {
                if let Some(p) = nodes.get(pid) {
                    format!("{}.{}", p.path, short_id(&n.id))
                } else if let Some(p) = input.upsert_nodes.iter().find(|u| &u.id == pid) {
                    let pp = p.path.clone().unwrap_or_else(|| short_id(&p.id));
                    format!("{}.{}", pp, short_id(&n.id))
                } else {
                    short_id(&n.id)
                }
            } else {
                short_id(&n.id)
            }
        });
        nodes.insert(
            n.id.clone(),
            ExistingNode {
                kind: n.kind.clone(),
                path,
                parent_id: n.parent_id.clone(),
                write_scope: n.write_scope.clone(),
                validators: n.validators.clone(),
                task_state: if n.kind == "task" {
                    Some(TaskState::Todo)
                } else {
                    None
                },
                title: n.title.clone(),
                has_attempt_history: nodes
                    .get(&n.id)
                    .map(|e| e.has_attempt_history)
                    .unwrap_or(false),
                inputs: n.inputs.clone(),
            },
        );
    }

    // Add edges
    for e in &input.add_edges {
        if e.from == e.to {
            report.violations.push(GraphViolation::SelfEdge { id: e.from.clone() });
            continue;
        }
        if !nodes.contains_key(&e.from) || !nodes.contains_key(&e.to) {
            report.violations.push(GraphViolation::DanglingDep {
                edge: e.clone(),
            });
            continue;
        }
        edges.insert((e.from.clone(), e.to.clone()));
    }

    // Cycle detection on full edge set
    if let Some(cycle) = find_cycle(&nodes, &edges) {
        report.violations.push(GraphViolation::Cycle { path: cycle });
    }

    // Type invariant: packages should not be task deps that look wrong — tasks may depend on tasks only for ready
    // Write-scope conflicts among sibling ready tasks (all new tasks are todo/ready candidates)
    let task_nodes: Vec<_> = nodes
        .iter()
        .filter(|(_, n)| n.kind == "task")
        .map(|(id, n)| (id.clone(), n.write_scope.clone()))
        .collect();
    for i in 0..task_nodes.len() {
        for j in (i + 1)..task_nodes.len() {
            let (id_a, sc_a) = &task_nodes[i];
            let (id_b, sc_b) = &task_nodes[j];
            if scopes_conflict(sc_a, sc_b) {
                // find a representative overlapping scope string
                let scope = sc_a
                    .first()
                    .cloned()
                    .unwrap_or_else(|| sc_b.first().cloned().unwrap_or_default());
                report.violations.push(GraphViolation::WriteConflict {
                    nodes: vec![id_a.clone(), id_b.clone()],
                    scope,
                });
            }
        }
    }

    report
}

fn short_id(id: &str) -> String {
    id.rsplit('_').next().unwrap_or(id).chars().take(8).collect()
}

fn find_cycle(
    nodes: &HashMap<String, ExistingNode>,
    edges: &HashSet<(String, String)>,
) -> Option<Vec<String>> {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for (f, t) in edges {
        if nodes.contains_key(f) && nodes.contains_key(t) {
            adj.entry(f.clone()).or_default().push(t.clone());
        }
    }
    let mut color: HashMap<String, u8> = HashMap::new(); // 0 white, 1 gray, 2 black
    let mut stack = Vec::new();
    let mut cycle_out = None;

    fn dfs(
        u: &str,
        adj: &HashMap<String, Vec<String>>,
        color: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
        cycle_out: &mut Option<Vec<String>>,
    ) {
        if cycle_out.is_some() {
            return;
        }
        color.insert(u.to_string(), 1);
        stack.push(u.to_string());
        if let Some(ns) = adj.get(u) {
            for v in ns {
                match color.get(v).copied().unwrap_or(0) {
                    1 => {
                        // cycle
                        if let Some(pos) = stack.iter().position(|x| x == v) {
                            let mut c = stack[pos..].to_vec();
                            c.push(v.clone());
                            *cycle_out = Some(c);
                        }
                        return;
                    }
                    0 => dfs(v, adj, color, stack, cycle_out),
                    _ => {}
                }
                if cycle_out.is_some() {
                    return;
                }
            }
        }
        stack.pop();
        color.insert(u.to_string(), 2);
    }

    for id in nodes.keys() {
        if color.get(id).copied().unwrap_or(0) == 0 {
            dfs(id, &adj, &mut color, &mut stack, &mut cycle_out);
            if cycle_out.is_some() {
                break;
            }
        }
    }
    cycle_out
}

/// After a successful virtual merge, compute which task ids become ready.
pub fn ready_after_publish(snapshot_nodes: &HashMap<String, ExistingNode>, edges: &HashSet<(String, String)>) -> Vec<String> {
    use crate::ready::{compute_ready, ReadyInput};
    use crate::state_machine::ScopeState;

    let packages: Vec<_> = snapshot_nodes
        .iter()
        .filter(|(_, n)| n.kind == "package")
        .map(|(_, n)| (n.path.clone(), ScopeState::Active)) // scope from path only in pure path; store fills real scopes
        .collect();

    let mut ready = Vec::new();
    for (id, n) in snapshot_nodes {
        if n.kind != "task" {
            continue;
        }
        let upstream: Vec<_> = edges
            .iter()
            .filter(|(_, to)| to == id)
            .filter_map(|(from, _)| {
                snapshot_nodes.get(from).and_then(|u| {
                    if u.kind == "task" {
                        Some((from.clone(), u.task_state.unwrap_or(TaskState::Todo)))
                    } else {
                        None
                    }
                })
            })
            .collect();
        let ancestors: Vec<_> = packages
            .iter()
            .filter(|(pp, _)| n.path.starts_with(&format!("{pp}.")))
            .cloned()
            .collect();
        let input = ReadyInput {
            node_id: id.clone(),
            kind: "task".into(),
            task_state: n.task_state.or(Some(TaskState::Todo)),
            path: n.path.clone(),
            upstream_task_states: upstream,
            ancestor_scopes: ancestors,
        };
        if compute_ready(&input) {
            ready.push(id.clone());
        }
    }
    ready.sort();
    ready
}

/// Topological layers for diagnostics.
pub fn topo_sort(edges: &HashSet<(String, String)>, node_ids: &[String]) -> Option<Vec<String>> {
    let mut indeg: HashMap<String, usize> = node_ids.iter().map(|id| (id.clone(), 0)).collect();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for (f, t) in edges {
        if indeg.contains_key(f) && indeg.contains_key(t) {
            *indeg.get_mut(t).unwrap() += 1;
            adj.entry(f.clone()).or_default().push(t.clone());
        }
    }
    let mut q: VecDeque<String> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut out = Vec::new();
    while let Some(u) = q.pop_front() {
        out.push(u.clone());
        if let Some(ns) = adj.get(&u) {
            for v in ns {
                let e = indeg.get_mut(v).unwrap();
                *e -= 1;
                if *e == 0 {
                    q.push_back(v.clone());
                }
            }
        }
    }
    if out.len() == node_ids.len() {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snap() -> GraphSnapshot {
        GraphSnapshot {
            version: 0,
            nodes: HashMap::new(),
            edges: HashSet::new(),
            registered_validators: BUILTIN_VALIDATORS.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn accepts_simple_chain() {
        let input = PublishInput {
            base_version: 0,
            summary: "chain".into(),
            upsert_nodes: vec![
                NodeDraft {
                    id: "nd_a".into(),
                    parent_id: None,
                    kind: "task".into(),
                    title: "A".into(),
                    layer: None,
                    role: None,
                    spec: serde_json::json!({}),
                    write_scope: vec!["a/".into()],
                    required_caps: vec![],
                    validators: vec!["scope-diff".into()],
                    inputs: serde_json::json!([]),
                    max_attempts: 3,
                    priority: 0,
                    path: Some("a".into()),
                },
                NodeDraft {
                    id: "nd_b".into(),
                    parent_id: None,
                    kind: "task".into(),
                    title: "B".into(),
                    layer: None,
                    role: None,
                    spec: serde_json::json!({}),
                    write_scope: vec!["b/".into()],
                    required_caps: vec![],
                    validators: vec!["scope-diff".into()],
                    inputs: serde_json::json!([]),
                    max_attempts: 3,
                    priority: 0,
                    path: Some("b".into()),
                },
            ],
            add_edges: vec![EdgeDraft {
                from: "nd_a".into(),
                to: "nd_b".into(),
            }],
            remove_nodes: vec![],
        };
        let r = validate_publish(&empty_snap(), &input);
        assert!(r.is_ok(), "{:?}", r.violations);
    }

    #[test]
    fn rejects_cycle() {
        let input = PublishInput {
            base_version: 0,
            summary: "cycle".into(),
            upsert_nodes: vec![
                task("nd_a", "a/", "a"),
                task("nd_b", "b/", "b"),
            ],
            add_edges: vec![
                EdgeDraft {
                    from: "nd_a".into(),
                    to: "nd_b".into(),
                },
                EdgeDraft {
                    from: "nd_b".into(),
                    to: "nd_a".into(),
                },
            ],
            remove_nodes: vec![],
        };
        let r = validate_publish(&empty_snap(), &input);
        assert!(r.violations.iter().any(|v| matches!(v, GraphViolation::Cycle { .. })));
    }

    #[test]
    fn rejects_dangling() {
        let input = PublishInput {
            base_version: 0,
            summary: "dangle".into(),
            upsert_nodes: vec![task("nd_a", "a/", "a")],
            add_edges: vec![EdgeDraft {
                from: "nd_ghost".into(),
                to: "nd_a".into(),
            }],
            remove_nodes: vec![],
        };
        let r = validate_publish(&empty_snap(), &input);
        assert!(r
            .violations
            .iter()
            .any(|v| matches!(v, GraphViolation::DanglingDep { .. })));
    }

    #[test]
    fn rejects_write_conflict() {
        let input = PublishInput {
            base_version: 0,
            summary: "wc".into(),
            upsert_nodes: vec![
                task("nd_a", "src/api/", "a"),
                task("nd_b", "src/api/user/", "b"),
            ],
            add_edges: vec![],
            remove_nodes: vec![],
        };
        let r = validate_publish(&empty_snap(), &input);
        assert!(r
            .violations
            .iter()
            .any(|v| matches!(v, GraphViolation::WriteConflict { .. })));
    }

    #[test]
    fn rejects_unknown_validator() {
        let mut t = task("nd_a", "a/", "a");
        t.validators = vec!["not-a-real-validator".into()];
        let input = PublishInput {
            base_version: 0,
            summary: "v".into(),
            upsert_nodes: vec![t],
            add_edges: vec![],
            remove_nodes: vec![],
        };
        let r = validate_publish(&empty_snap(), &input);
        assert!(r
            .violations
            .iter()
            .any(|v| matches!(v, GraphViolation::ValidatorUnregistered { .. })));
    }

    #[test]
    fn rejects_duplicate_id() {
        let input = PublishInput {
            base_version: 0,
            summary: "dup".into(),
            upsert_nodes: vec![task("nd_a", "a/", "a"), task("nd_a", "b/", "b")],
            add_edges: vec![],
            remove_nodes: vec![],
        };
        let r = validate_publish(&empty_snap(), &input);
        assert!(r
            .violations
            .iter()
            .any(|v| matches!(v, GraphViolation::DuplicateId { .. })));
    }

    fn task(id: &str, scope: &str, path: &str) -> NodeDraft {
        NodeDraft {
            id: id.into(),
            parent_id: None,
            kind: "task".into(),
            title: id.into(),
            layer: None,
            role: None,
            spec: serde_json::json!({}),
            write_scope: vec![scope.into()],
            required_caps: vec![],
            validators: vec!["scope-diff".into()],
            inputs: serde_json::json!([]),
            max_attempts: 3,
            priority: 0,
            path: Some(path.into()),
        }
    }
}
