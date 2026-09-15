use atlas_analysis::*;
use atlas_model::*;
use std::time::Duration;

fn node(id: &str) -> Definition {
    Definition {
        id: DefinitionId(id.into()),
        context_id: ContextId("context".into()),
        file_id: FileId("file".into()),
        name: id.into(),
        qualified_name: id.into(),
        kind: "function".into(),
        parent_id: None,
        signature: "fn test()".into(),
        signature_hash: "signature".into(),
        body_hash: "body".into(),
        span: Span {
            file_id: FileId("file".into()),
            start: 0,
            end: 1,
        },
        body_span: None,
        cfg: vec![],
        cfg_status: CfgStatus::Active,
        visibility: "pub".into(),
        metrics: Metrics {
            lines: 5,
            branches: 2,
            ..Default::default()
        },
    }
}
fn edge(id: &str, source: &str, target: &str) -> Relation {
    Relation {
        id: RelationId(id.into()),
        source: DefinitionId(source.into()),
        target: Target::Resolved {
            id: DefinitionId(target.into()),
        },
        kind: "calls".into(),
        span: Span {
            file_id: FileId("file".into()),
            start: 0,
            end: 1,
        },
        evidence_id: EvidenceId("evidence".into()),
    }
}
fn control() -> AnalysisControl {
    AnalysisControl::new(Duration::from_secs(5))
}
fn analyze(nodes: &[Definition], edges: &[Relation]) -> GraphAnalysis {
    analyze_graph(
        &SelectedGraph {
            nodes,
            edges,
            coverage: &Coverage::complete(),
            input_truncated: false,
        },
        AnalysisLimits::default(),
        &control(),
    )
    .unwrap()
}
fn path(
    nodes: &[Definition],
    edges: &[Relation],
    target: &str,
    limits: AnalysisLimits,
) -> PathAnalysis {
    find_path(
        &SelectedGraph {
            nodes,
            edges,
            coverage: &Coverage::complete(),
            input_truncated: false,
        },
        &DefinitionId("a".into()),
        Some(&DefinitionId(target.into())),
        limits,
        &control(),
    )
    .unwrap()
}

#[test]
fn components_include_recursion_and_deterministic_condensation() {
    let nodes = [node("a"), node("b"), node("c"), node("d")];
    let edges = [
        edge("1", "a", "b"),
        edge("2", "b", "a"),
        edge("3", "b", "c"),
        edge("4", "d", "d"),
    ];
    let report = analyze(&nodes, &edges);
    assert_eq!(report.components.len(), 3);
    assert_eq!(
        report.components[0].members,
        [DefinitionId("a".into()), DefinitionId("b".into())]
    );
    assert!(report.components[0].recursive);
    assert!(!report.components[1].recursive);
    assert!(report.components[2].recursive);
    assert_eq!(report.component_links.len(), 1);
    assert!(
        report
            .envelope
            .assumptions
            .iter()
            .any(|text| text.contains("entire repository"))
    );
    let reversed = analyze(
        &nodes.into_iter().rev().collect::<Vec<_>>(),
        &edges.into_iter().rev().collect::<Vec<_>>(),
    );
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        serde_json::to_value(reversed).unwrap()
    );
}

#[test]
fn distinct_fan_counts_do_not_count_repeated_sites_as_distinct_targets() {
    let nodes = [node("a"), node("b")];
    let mut unknown = edge("3", "a", "b");
    unknown.target = Target::Unknown {
        reason: UnknownReason::IndirectTargetUnknown,
        label: "callback".into(),
    };
    let report = analyze(&nodes, &[edge("1", "a", "b"), edge("2", "a", "b"), unknown]);
    let a = &report.metrics[0];
    assert_eq!(
        (
            a.distinct_callees,
            a.outgoing_call_sites,
            a.unknown_call_sites
        ),
        (1, 3, 1)
    );
    let b = &report.metrics[1];
    assert_eq!((b.distinct_callers, b.incoming_call_sites), (1, 2));
    assert_eq!(report.envelope.coverage.status, Status::Partial);
    assert_eq!(report.unknown_frontier.len(), 1);
    assert_eq!(report.distributions[0].median, 5);
}

#[test]
fn shortest_path_is_deterministic_with_parallel_edges_and_cycles() {
    let nodes = [node("a"), node("b"), node("c"), node("d")];
    let edges = [
        edge("ab2", "a", "b"),
        edge("ab1", "a", "b"),
        edge("ac", "a", "c"),
        edge("ba", "b", "a"),
        edge("bd", "b", "d"),
        edge("cd", "c", "d"),
    ];
    let report = path(&nodes, &edges, "d", AnalysisLimits::default());
    assert_eq!(report.outcome, PathOutcome::Found);
    assert_eq!(
        report.path,
        ["a", "b", "d"].map(|id| DefinitionId(id.into()))
    );
    assert_eq!(
        report.relations,
        ["ab1", "bd"].map(|id| RelationId(id.into()))
    );
    let reversed = path(
        &nodes.into_iter().rev().collect::<Vec<_>>(),
        &edges.into_iter().rev().collect::<Vec<_>>(),
        "d",
        AnalysisLimits::default(),
    );
    assert_eq!(
        serde_json::to_value(report).unwrap(),
        serde_json::to_value(reversed).unwrap()
    );
}

#[test]
fn zero_length_path_and_depth_zero_are_well_defined() {
    let nodes = [node("a"), node("b")];
    let edges = [edge("ab", "a", "b")];
    assert_eq!(
        path(
            &nodes,
            &edges,
            "a",
            AnalysisLimits {
                max_depth: 0,
                ..Default::default()
            }
        )
        .path,
        [DefinitionId("a".into())]
    );
    let report = path(
        &nodes,
        &edges,
        "b",
        AnalysisLimits {
            max_depth: 0,
            ..Default::default()
        },
    );
    assert_eq!(report.outcome, PathOutcome::Unknown);
    assert!(report.envelope.truncated);
    assert_eq!(report.reachable, [DefinitionId("a".into())]);
}

#[test]
fn missing_path_is_only_a_negative_inside_complete_selection() {
    let nodes = [node("a"), node("b")];
    assert_eq!(
        path(&nodes, &[], "b", AnalysisLimits::default()).outcome,
        PathOutcome::NotFoundInSelection
    );
    assert_eq!(
        path(&nodes, &[], "missing", AnalysisLimits::default()).outcome,
        PathOutcome::Unknown
    );
    let coverage = Coverage::partial(UnknownReason::MissingDependency, "Dependency unavailable");
    let report = find_path(
        &SelectedGraph {
            nodes: &nodes,
            edges: &[],
            coverage: &coverage,
            input_truncated: false,
        },
        &DefinitionId("a".into()),
        Some(&DefinitionId("b".into())),
        AnalysisLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(report.outcome, PathOutcome::Unknown);
}

#[test]
fn unknown_frontier_prevents_false_complete_negative() {
    let nodes = [node("a"), node("b")];
    let mut unknown = edge("unknown", "a", "b");
    unknown.target = Target::Unknown {
        reason: UnknownReason::IndirectTargetUnknown,
        label: "fn pointer".into(),
    };
    let report = path(&nodes, &[unknown], "b", AnalysisLimits::default());
    assert_eq!(report.outcome, PathOutcome::Unknown);
    assert_eq!(report.unknown_frontier.len(), 1);
    let outside = path(
        &nodes,
        &[edge("outside", "a", "external")],
        "b",
        AnalysisLimits::default(),
    );
    assert_eq!(
        outside.unknown_frontier[0].target_definition_id,
        Some(DefinitionId("external".into()))
    );
}

#[test]
fn edge_node_visit_and_input_caps_are_reported() {
    let nodes = [node("a"), node("b"), node("c")];
    let edges = [edge("1", "a", "b"), edge("2", "b", "c")];
    for limits in [
        AnalysisLimits {
            max_nodes: 1,
            ..Default::default()
        },
        AnalysisLimits {
            max_edges: 1,
            ..Default::default()
        },
        AnalysisLimits {
            max_visits: 1,
            ..Default::default()
        },
    ] {
        let report = path(&nodes, &edges, "c", limits);
        assert_eq!(report.outcome, PathOutcome::Unknown);
        assert!(report.envelope.truncated);
    }
    let report = find_path(
        &SelectedGraph {
            nodes: &nodes,
            edges: &edges,
            coverage: &Coverage::complete(),
            input_truncated: true,
        },
        &DefinitionId("c".into()),
        Some(&DefinitionId("a".into())),
        AnalysisLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(report.outcome, PathOutcome::Unknown);
}

#[test]
fn cancellation_and_deadline_never_return_complete_negative() {
    let nodes = [node("a")];
    let cancelled = control();
    cancelled.cancel();
    for control in [cancelled, AnalysisControl::new(Duration::ZERO)] {
        let graph = SelectedGraph {
            nodes: &nodes,
            edges: &[],
            coverage: &Coverage::complete(),
            input_truncated: false,
        };
        let report = find_path(
            &graph,
            &DefinitionId("a".into()),
            None,
            AnalysisLimits::default(),
            &control,
        )
        .unwrap();
        assert_eq!(report.outcome, PathOutcome::Unknown);
        assert!(report.envelope.cancelled || report.envelope.deadline_reached);
        let summary = analyze_graph(&graph, AnalysisLimits::default(), &control).unwrap();
        assert!(summary.envelope.truncated);
        assert!(summary.components.is_empty());
    }
}

#[test]
fn rejects_invalid_limits_duplicate_ids_mixed_contexts_and_byte_overflow() {
    let node_a = node("a");
    let mut other = node("b");
    other.context_id = ContextId("other".into());
    for nodes in [
        vec![node_a.clone(), node_a.clone()],
        vec![node_a.clone(), other],
    ] {
        assert!(matches!(
            analyze_graph(
                &SelectedGraph {
                    nodes: &nodes,
                    edges: &[],
                    coverage: &Coverage::complete(),
                    input_truncated: false
                },
                AnalysisLimits::default(),
                &control()
            ),
            Err(AnalysisError::InvalidInput(_))
        ));
    }
    let graph = SelectedGraph {
        nodes: std::slice::from_ref(&node_a),
        edges: &[],
        coverage: &Coverage::complete(),
        input_truncated: false,
    };
    assert!(matches!(
        analyze_graph(
            &graph,
            AnalysisLimits {
                max_nodes: 0,
                ..Default::default()
            },
            &control()
        ),
        Err(AnalysisError::InvalidInput(_))
    ));
    assert!(matches!(
        analyze_graph(
            &graph,
            AnalysisLimits {
                max_response_bytes: 256,
                ..Default::default()
            },
            &control()
        ),
        Err(AnalysisError::BudgetExhausted)
    ));
    let duplicate = [edge("x", "a", "a"), edge("x", "a", "a")];
    assert!(
        analyze_graph(
            &SelectedGraph {
                edges: &duplicate,
                ..graph
            },
            AnalysisLimits::default(),
            &control()
        )
        .is_err()
    );
}

#[test]
fn empty_graph_has_no_metrics_and_noncall_relations_are_not_calls() {
    let report = analyze(&[], &[]);
    assert!(report.components.is_empty());
    assert!(report.distributions.is_empty());
    let mut relation = edge("1", "a", "a");
    relation.kind = "declares".into();
    let report = analyze(&[node("a")], &[relation]);
    assert!(!report.components[0].recursive);
    assert_eq!(report.selected_call_sites, 0);
}

#[test]
fn reachability_does_not_enumerate_paths_or_loop_on_cycles() {
    let nodes = [node("a"), node("b"), node("c")];
    let edges = [edge("ab", "a", "b"), edge("ba", "b", "a")];
    let report = find_path(
        &SelectedGraph {
            nodes: &nodes,
            edges: &edges,
            coverage: &Coverage::complete(),
            input_truncated: false,
        },
        &DefinitionId("a".into()),
        None,
        AnalysisLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(report.outcome, PathOutcome::ReachabilityComplete);
    assert_eq!(
        report.reachable,
        [DefinitionId("a".into()), DefinitionId("b".into())]
    );
    assert!(report.path.is_empty());
}

#[test]
fn hard_input_caps_refuse_work_before_quadratic_growth() {
    let nodes = (0..10_001)
        .map(|index| node(&index.to_string()))
        .collect::<Vec<_>>();
    assert!(matches!(
        analyze_graph(
            &SelectedGraph {
                nodes: &nodes,
                edges: &[],
                coverage: &Coverage::complete(),
                input_truncated: false
            },
            AnalysisLimits::default(),
            &control()
        ),
        Err(AnalysisError::BudgetExhausted)
    ));
}

#[test]
fn exhaustive_three_node_graphs_match_independent_transitive_closure() {
    let nodes = [node("a"), node("b"), node("c")];
    for mask in 0u16..512 {
        let mut closure = [[false; 3]; 3];
        let mut edges = Vec::new();
        for (source, row) in closure.iter_mut().enumerate() {
            for (target, reachable) in row.iter_mut().enumerate() {
                if mask & (1 << (source * 3 + target)) != 0 {
                    *reachable = true;
                    edges.push(edge(
                        &format!("{source}:{target}"),
                        &nodes[source].id.0,
                        &nodes[target].id.0,
                    ));
                }
            }
        }
        for (index, row) in closure.iter_mut().enumerate() {
            row[index] = true;
        }
        for through in 0..3 {
            for source in 0..3 {
                for target in 0..3 {
                    closure[source][target] |= closure[source][through] && closure[through][target];
                }
            }
        }
        let report = analyze(&nodes, &edges);
        for source in 0..3 {
            for target in 0..3 {
                let same_component = report.components.iter().any(|component| {
                    component.members.contains(&nodes[source].id)
                        && component.members.contains(&nodes[target].id)
                });
                assert_eq!(
                    same_component,
                    closure[source][target] && closure[target][source],
                    "SCC mismatch for mask {mask}"
                );
                let graph = SelectedGraph {
                    nodes: &nodes,
                    edges: &edges,
                    coverage: &Coverage::complete(),
                    input_truncated: false,
                };
                let report = find_path(
                    &graph,
                    &nodes[source].id,
                    Some(&nodes[target].id),
                    AnalysisLimits::default(),
                    &control(),
                )
                .unwrap();
                assert_eq!(
                    report.outcome == PathOutcome::Found,
                    closure[source][target],
                    "BFS mismatch for mask {mask}"
                );
            }
        }
    }
}
