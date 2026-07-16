//! ≥20 illegal graph fixtures covering FR-03.3 rules (≥2 each).

use std::collections::{HashMap, HashSet};

use sunmao_core::graph::{
    validate_publish, EdgeDraft, GraphSnapshot, GraphViolation, NodeDraft, PublishInput,
    BUILTIN_VALIDATORS,
};

fn empty_snap() -> GraphSnapshot {
    GraphSnapshot {
        version: 0,
        nodes: HashMap::new(),
        edges: HashSet::new(),
        registered_validators: BUILTIN_VALIDATORS.iter().map(|s| s.to_string()).collect(),
    }
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

fn pub_nodes(nodes: Vec<NodeDraft>, edges: Vec<EdgeDraft>) -> PublishInput {
    PublishInput {
        base_version: 0,
        summary: "fixture".into(),
        upsert_nodes: nodes,
        add_edges: edges,
        remove_nodes: vec![],
    }
}

fn has_rule(v: &[GraphViolation], pred: impl Fn(&GraphViolation) -> bool) -> bool {
    v.iter().any(pred)
}

#[test]
fn fixtures_20_plus_illegal_graphs() {
    let mut cases: Vec<(&str, PublishInput, Box<dyn Fn(&[GraphViolation]) -> bool>)> = Vec::new();

    // cycle ×2
    cases.push((
        "cycle_ab",
        pub_nodes(
            vec![task("nd_a", "a/", "a"), task("nd_b", "b/", "b")],
            vec![
                EdgeDraft {
                    from: "nd_a".into(),
                    to: "nd_b".into(),
                },
                EdgeDraft {
                    from: "nd_b".into(),
                    to: "nd_a".into(),
                },
            ],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::Cycle { .. }))),
    ));
    cases.push((
        "cycle_abc",
        pub_nodes(
            vec![
                task("nd_a", "a/", "a"),
                task("nd_b", "b/", "b"),
                task("nd_c", "c/", "c"),
            ],
            vec![
                EdgeDraft {
                    from: "nd_a".into(),
                    to: "nd_b".into(),
                },
                EdgeDraft {
                    from: "nd_b".into(),
                    to: "nd_c".into(),
                },
                EdgeDraft {
                    from: "nd_c".into(),
                    to: "nd_a".into(),
                },
            ],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::Cycle { .. }))),
    ));

    // dangling ×2
    cases.push((
        "dangling_from",
        pub_nodes(
            vec![task("nd_a", "a/", "a")],
            vec![EdgeDraft {
                from: "nd_ghost".into(),
                to: "nd_a".into(),
            }],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::DanglingDep { .. }))),
    ));
    cases.push((
        "dangling_to",
        pub_nodes(
            vec![task("nd_a", "a/", "a")],
            vec![EdgeDraft {
                from: "nd_a".into(),
                to: "nd_ghost".into(),
            }],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::DanglingDep { .. }))),
    ));

    // duplicate id ×2
    cases.push((
        "dup_id_1",
        pub_nodes(
            vec![task("nd_x", "a/", "a"), task("nd_x", "b/", "b")],
            vec![],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::DuplicateId { .. }))),
    ));
    cases.push((
        "dup_id_2",
        pub_nodes(
            vec![
                task("nd_y", "c/", "c"),
                task("nd_y", "d/", "d"),
                task("nd_z", "e/", "e"),
            ],
            vec![],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::DuplicateId { .. }))),
    ));

    // type invariant ×2+
    let mut no_title = task("nd_t1", "t1/", "t1");
    no_title.title = "  ".into();
    cases.push((
        "empty_title",
        pub_nodes(vec![no_title], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::TypeInvariant { .. }))),
    ));
    let mut no_scope = task("nd_t2", "t2/", "t2");
    no_scope.write_scope.clear();
    cases.push((
        "task_no_scope",
        pub_nodes(vec![no_scope], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::TypeInvariant { .. }))),
    ));
    let mut bad_kind = task("nd_t3", "t3/", "t3");
    bad_kind.kind = "widget".into();
    cases.push((
        "bad_kind",
        pub_nodes(vec![bad_kind], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::TypeInvariant { .. }))),
    ));

    // write conflict ×2+
    cases.push((
        "write_prefix",
        pub_nodes(
            vec![task("nd_w1", "src/api/", "w1"), task("nd_w2", "src/api/user/", "w2")],
            vec![],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::WriteConflict { .. }))),
    ));
    cases.push((
        "write_exact",
        pub_nodes(
            vec![task("nd_w3", "lib/", "w3"), task("nd_w4", "lib/", "w4")],
            vec![],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::WriteConflict { .. }))),
    ));
    cases.push((
        "write_reverse_prefix",
        pub_nodes(
            vec![task("nd_w5", "pkg/mod/", "w5"), task("nd_w6", "pkg/", "w6")],
            vec![],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::WriteConflict { .. }))),
    ));

    // missing input ×2
    let mut mi1 = task("nd_m1", "m1/", "m1");
    mi1.inputs = serde_json::json!([{ "artifact_id": "" }]);
    cases.push((
        "missing_input_empty",
        pub_nodes(vec![mi1], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::MissingInput { .. }))),
    ));
    let mut mi2 = task("nd_m2", "m2/", "m2");
    mi2.inputs = serde_json::json!([{ "version": "1.0" }]);
    cases.push((
        "missing_input_absent",
        pub_nodes(vec![mi2], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::MissingInput { .. }))),
    ));

    // validator unregistered ×2
    let mut v1 = task("nd_v1", "v1/", "v1");
    v1.validators = vec!["nope-validator".into()];
    cases.push((
        "validator_unknown_1",
        pub_nodes(vec![v1], vec![]),
        Box::new(|v| {
            has_rule(v, |x| matches!(x, GraphViolation::ValidatorUnregistered { .. }))
        }),
    ));
    let mut v2 = task("nd_v2", "v2/", "v2");
    v2.validators = vec!["scope-diff".into(), "magic".into()];
    cases.push((
        "validator_unknown_2",
        pub_nodes(vec![v2], vec![]),
        Box::new(|v| {
            has_rule(v, |x| matches!(x, GraphViolation::ValidatorUnregistered { .. }))
        }),
    ));

    // self edge ×2
    cases.push((
        "self_edge_1",
        pub_nodes(
            vec![task("nd_s1", "s1/", "s1")],
            vec![EdgeDraft {
                from: "nd_s1".into(),
                to: "nd_s1".into(),
            }],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::SelfEdge { .. }))),
    ));
    cases.push((
        "self_edge_2",
        pub_nodes(
            vec![task("nd_s2", "s2/", "s2")],
            vec![EdgeDraft {
                from: "nd_s2".into(),
                to: "nd_s2".into(),
            }],
        ),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::SelfEdge { .. }))),
    ));

    // orphan parent ×2
    let mut o1 = task("nd_o1", "o1/", "o1");
    o1.parent_id = Some("nd_missing_parent".into());
    cases.push((
        "orphan_parent_1",
        pub_nodes(vec![o1], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::OrphanParent { .. }))),
    ));
    let mut o2 = task("nd_o2", "o2/", "o2");
    o2.parent_id = Some("nd_also_missing".into());
    cases.push((
        "orphan_parent_2",
        pub_nodes(vec![o2], vec![]),
        Box::new(|v| has_rule(v, |x| matches!(x, GraphViolation::OrphanParent { .. }))),
    ));

    assert!(
        cases.len() >= 20,
        "need ≥20 fixtures, got {}",
        cases.len()
    );

    for (name, input, check) in cases {
        let report = validate_publish(&empty_snap(), &input);
        assert!(
            !report.is_ok(),
            "{name}: expected violations, got ok"
        );
        assert!(
            check(&report.violations),
            "{name}: wrong violations: {:?}",
            report.violations
        );
    }
}
