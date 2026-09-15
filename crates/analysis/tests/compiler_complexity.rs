use atlas_analysis::*;
use atlas_model::*;
use std::time::Duration;

fn mapping(block: u32) -> CompilerSourceMapping {
    CompilerSourceMapping::Exact {
        path: "src/lib.rs".into(),
        start_byte: block * 4,
        end_byte: block * 4 + 1,
    }
}
fn edge(target: u32, kind: CompilerEdgeKind) -> CompilerSuccessor {
    CompilerSuccessor {
        target,
        kind,
        switch_value: None,
    }
}
fn block(index: u32, kind: &str, successors: Vec<CompilerSuccessor>) -> CompilerBlock {
    CompilerBlock {
        index,
        is_cleanup: false,
        statements: vec![],
        terminator: CompilerTerminator {
            kind: kind.into(),
            source_scope: 0,
            span: mapping(index),
            locals: CompilerLocalEffects::default(),
            normal_return_defs: vec![],
            successors,
            unwind: None,
            call_target: None,
            assert_expected: None,
        },
    }
}
fn terminal(index: u32) -> CompilerBlock {
    block(index, "return", vec![])
}
fn goto(index: u32, target: u32) -> CompilerBlock {
    block(index, "goto", vec![edge(target, CompilerEdgeKind::Normal)])
}
fn switch(index: u32, targets: &[u32]) -> CompilerBlock {
    let mut successors = targets[..targets.len() - 1]
        .iter()
        .enumerate()
        .map(|(value, target)| CompilerSuccessor {
            target: *target,
            kind: CompilerEdgeKind::SwitchValue,
            switch_value: Some(value.to_string()),
        })
        .collect::<Vec<_>>();
    successors.push(edge(*targets.last().unwrap(), CompilerEdgeKind::Otherwise));
    block(index, "switch_int", successors)
}
fn page(blocks: Vec<CompilerBlock>) -> CompilerFlowPage {
    CompilerFlowPage {
        snapshot_id: SnapshotId("snapshot:one".into()),
        context_id: ContextId("context:one".into()),
        import_id: "import:one".into(),
        definition_id: DefinitionId("definition:event".into()),
        compiler: CompilerIdentity {
            adapter: "atlas-rustc".into(),
            adapter_version: "0.1.0".into(),
            release: "nightly".into(),
            commit_hash: "compiler-hash".into(),
            commit_date: "2026-09-01".into(),
            host: "x86_64-unknown-linux-gnu".into(),
            llvm_version: "22".into(),
        },
        phase: "runtime_optimized".into(),
        input_manifest_hash: "manifest:one".into(),
        panic_strategy: "unwind".into(),
        offset: 0,
        total_blocks: blocks.len() as u32,
        next_offset: None,
        coverage: Coverage::complete(),
        body: CompilerBody {
            body_id: "body:event".into(),
            def_path: "fixture::event".into(),
            kind: "function".into(),
            span: mapping(0),
            argument_count: 0,
            locals: vec![],
            source_scopes: vec![],
            blocks,
        },
    }
}
fn control() -> AnalysisControl {
    AnalysisControl::new(Duration::from_secs(5))
}
fn analyze(input: &CompilerFlowPage, limits: CompilerComplexityLimits) -> CompilerComplexity {
    analyze_compiler_complexity(input, limits, &control()).unwrap()
}
fn run(blocks: Vec<CompilerBlock>) -> CompilerComplexity {
    analyze(&page(blocks), CompilerComplexityLimits::default())
}

#[test]
fn independent_straight_and_multiple_exit_oracles_include_synthetic_exit() {
    let straight = run(vec![goto(0, 1), terminal(1)]);
    assert_eq!(
        straight.metrics,
        Some(CfgComplexityMetrics {
            nodes: 3,
            edges: 2,
            components: 1,
            cyclomatic: 1,
            runtime_blocks: 2,
            runtime_edges: 1,
            synthetic_exit_edges: 1,
            excluded_imaginary_edges: 0
        })
    );
    let branched = run(vec![switch(0, &[1, 2]), terminal(1), terminal(2)]);
    let metrics = branched.metrics.unwrap();
    assert_eq!(
        (metrics.nodes, metrics.edges, metrics.cyclomatic),
        (4, 4, 2)
    );
    assert_eq!(
        branched
            .exit_sites
            .iter()
            .map(|site| site.block)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(branched.exit_sites[0].span, mapping(1));
    assert_eq!(branched.formula, "E - N + 2P");
    assert_eq!(branched.envelope.coverage.status, Status::Partial);
}

#[test]
fn parallel_switch_alternatives_are_not_collapsed_by_target() {
    let report = run(vec![switch(0, &[1, 1, 1]), terminal(1)]);
    let metrics = report.metrics.unwrap();
    assert_eq!(
        (
            metrics.runtime_edges,
            metrics.synthetic_exit_edges,
            metrics.cyclomatic
        ),
        (3, 1, 3)
    );
}

#[test]
fn loops_are_structural_and_no_exit_is_unavailable_not_zero() {
    let report = run(vec![switch(0, &[0, 1]), terminal(1)]);
    assert_eq!(report.metrics.unwrap().cyclomatic, 2);
    let closed = run(vec![goto(0, 0)]);
    assert!(closed.metrics.is_none());
    assert!(!closed.envelope.truncated);
    assert_eq!(closed.reachable_blocks, [0]);
    assert!(
        closed
            .envelope
            .coverage
            .limitations
            .iter()
            .any(|text| text.contains("No terminal"))
    );
}

#[test]
fn unreachable_blocks_and_imaginary_edges_are_explicitly_excluded() {
    let false_edge = block(
        0,
        "false_edge",
        vec![
            edge(1, CompilerEdgeKind::Normal),
            edge(2, CompilerEdgeKind::Imaginary),
        ],
    );
    let report = run(vec![false_edge, terminal(1), switch(2, &[2, 2])]);
    let metrics = report.metrics.unwrap();
    assert_eq!(
        (metrics.cyclomatic, metrics.excluded_imaginary_edges),
        (1, 1)
    );
    assert_eq!(report.reachable_blocks, [0, 1]);
    assert_eq!(report.unreachable_blocks, [2]);
    let report = run(vec![terminal(0), switch(1, &[1, 1, 1])]);
    assert_eq!(report.metrics.unwrap().cyclomatic, 1);
}

#[test]
fn false_unwind_metadata_does_not_invent_runtime_paths() {
    let mut false_unwind = block(
        0,
        "false_unwind",
        vec![
            edge(1, CompilerEdgeKind::Normal),
            edge(2, CompilerEdgeKind::Unwind),
        ],
    );
    false_unwind.terminator.unwind = Some(CompilerUnwind::Cleanup { target: 2 });
    let report = run(vec![false_unwind, terminal(1), terminal(2)]);
    assert_eq!(report.metrics.as_ref().unwrap().cyclomatic, 1);
    assert_eq!(report.metrics.unwrap().excluded_imaginary_edges, 1);
    assert_eq!(report.unreachable_blocks, [2]);
    let mut false_unwind = block(0, "false_unwind", vec![edge(1, CompilerEdgeKind::Normal)]);
    false_unwind.terminator.unwind = Some(CompilerUnwind::Continue);
    let report = run(vec![false_unwind, terminal(1)]);
    assert_eq!(report.exit_sites.len(), 1);
    assert_eq!(report.metrics.unwrap().cyclomatic, 1);
}

#[test]
fn external_unwind_and_cleanup_paths_have_inspectable_exit_conventions() {
    let mut no_normal_return = block(0, "call", vec![]);
    no_normal_return.terminator.unwind = Some(CompilerUnwind::Continue);
    let no_normal_return = run(vec![no_normal_return]);
    assert_eq!(no_normal_return.metrics.unwrap().cyclomatic, 1);
    assert_eq!(no_normal_return.exit_sites.len(), 1);
    assert_eq!(no_normal_return.exit_sites[0].kind, "unwind_continue");
    for unwind in [
        CompilerUnwind::Continue,
        CompilerUnwind::Terminate {
            reason: "abi".into(),
        },
    ] {
        let mut call = block(0, "call", vec![edge(1, CompilerEdgeKind::Normal)]);
        call.terminator.unwind = Some(unwind);
        let report = run(vec![call, terminal(1)]);
        assert_eq!(report.metrics.unwrap().cyclomatic, 2);
        assert_eq!(report.exit_sites.len(), 2);
        assert!(report.exit_sites[0].kind.starts_with("unwind_"));
    }
    let mut call = block(
        0,
        "call",
        vec![
            edge(1, CompilerEdgeKind::Normal),
            edge(2, CompilerEdgeKind::Unwind),
        ],
    );
    call.terminator.unwind = Some(CompilerUnwind::Cleanup { target: 2 });
    let report = run(vec![call, terminal(1), block(2, "unwind_resume", vec![])]);
    assert_eq!(report.metrics.unwrap().cyclomatic, 2);
    assert_eq!(report.exit_sites[1].kind, "terminal:unwind_resume");
}

#[test]
fn terminal_assembly_and_nonreturning_calls_do_not_claim_return() {
    for kind in ["inline_asm", "call"] {
        let mut leaf = block(0, kind, vec![]);
        leaf.terminator.unwind = Some(CompilerUnwind::Unreachable);
        let report = run(vec![leaf]);
        assert_eq!(report.metrics.unwrap().cyclomatic, 1);
        assert_eq!(report.exit_sites[0].kind, format!("terminal:{kind}"));
        assert!(
            report
                .envelope
                .assumptions
                .iter()
                .any(|text| text.contains("does not assert return or termination"))
        );
    }
}

#[test]
fn coroutine_resume_and_drop_alternatives_are_retained() {
    let report = run(vec![
        block(
            0,
            "yield",
            vec![
                edge(1, CompilerEdgeKind::Resume),
                edge(2, CompilerEdgeKind::CoroutineDrop),
            ],
        ),
        terminal(1),
        block(2, "coroutine_drop", vec![]),
    ]);
    assert_eq!(report.metrics.unwrap().cyclomatic, 2);
}

#[test]
fn pagination_missing_entry_duplicates_and_bad_edges_are_rejected() {
    let original = page(vec![goto(0, 1), terminal(1)]);
    let mut cases = vec![];
    let mut input = original.clone();
    input.offset = 1;
    cases.push(input);
    let mut input = original.clone();
    input.next_offset = Some(1);
    cases.push(input);
    let mut input = original.clone();
    input.total_blocks += 1;
    cases.push(input);
    let mut input = original.clone();
    input.body.blocks[1].index = 0;
    cases.push(input);
    let mut input = original.clone();
    input.body.blocks[0].index = 2;
    cases.push(input);
    let mut input = original.clone();
    input.body.blocks[0].terminator.successors[0].target = 99;
    cases.push(input);
    let mut input = original.clone();
    input.body.blocks[0].terminator.successors.clear();
    cases.push(input);
    let mut input = original.clone();
    input.phase = "source".into();
    cases.push(input);
    let mut input = original.clone();
    input.coverage.status = Status::Unavailable;
    cases.push(input);
    let mut input = original.clone();
    input.body.blocks[0].terminator.kind = "future_unknown".into();
    cases.push(input);
    for input in cases {
        assert!(
            analyze_compiler_complexity(&input, CompilerComplexityLimits::default(), &control())
                .is_err()
        );
    }
    assert!(
        analyze_compiler_complexity(
            &page(vec![]),
            CompilerComplexityLimits::default(),
            &control()
        )
        .is_err()
    );
}

#[test]
fn malformed_switch_values_and_unwind_contracts_are_rejected() {
    for value in [
        None,
        Some("01"),
        Some("340282366920938463463374607431768211456"),
    ] {
        let mut input = page(vec![switch(0, &[1, 1]), terminal(1)]);
        input.body.blocks[0].terminator.successors[0].switch_value = value.map(str::to_owned);
        assert!(
            analyze_compiler_complexity(&input, CompilerComplexityLimits::default(), &control())
                .is_err()
        );
    }
    let mut input = page(vec![switch(0, &[1, 1, 1]), terminal(1)]);
    input.body.blocks[0].terminator.successors[1].switch_value = Some("0".into());
    assert!(
        analyze_compiler_complexity(&input, CompilerComplexityLimits::default(), &control())
            .is_err()
    );
    let mut input = page(vec![goto(0, 1), terminal(1)]);
    input.body.blocks[0].terminator.successors[0].switch_value = Some("0".into());
    assert!(
        analyze_compiler_complexity(&input, CompilerComplexityLimits::default(), &control())
            .is_err()
    );
    let mut call = block(
        0,
        "call",
        vec![
            edge(1, CompilerEdgeKind::Normal),
            edge(1, CompilerEdgeKind::Unwind),
        ],
    );
    call.terminator.unwind = Some(CompilerUnwind::Cleanup { target: 2 });
    assert!(
        analyze_compiler_complexity(
            &page(vec![call, terminal(1), terminal(2)]),
            CompilerComplexityLimits::default(),
            &control()
        )
        .is_err()
    );
}

#[test]
fn budgets_and_cancellation_withhold_counts_but_record_caps_do_not() {
    let input = page(vec![goto(0, 1), terminal(1)]);
    for limits in [
        CompilerComplexityLimits {
            max_blocks: 1,
            ..Default::default()
        },
        CompilerComplexityLimits {
            max_edges: 1,
            ..Default::default()
        },
    ] {
        let report = analyze(&input, limits);
        assert!(report.metrics.is_none());
        assert!(report.envelope.truncated);
    }
    let cancelled = control();
    cancelled.cancel();
    for control in [cancelled, AnalysisControl::new(Duration::ZERO)] {
        let report =
            analyze_compiler_complexity(&input, CompilerComplexityLimits::default(), &control)
                .unwrap();
        assert!(report.metrics.is_none());
        assert!(report.envelope.truncated);
    }
    let mut blocks = vec![switch(0, &(1..=100).collect::<Vec<_>>())];
    blocks.extend((1..=100).map(terminal));
    let report = analyze(
        &page(blocks),
        CompilerComplexityLimits {
            max_response_bytes: 3000,
            ..Default::default()
        },
    );
    assert_eq!(report.metrics.unwrap().cyclomatic, 100);
    assert!(report.locations_truncated);
    assert!(!report.envelope.truncated);
    assert!(report.exit_sites.is_empty() && report.reachable_blocks.is_empty());
}

#[test]
fn input_digests_and_output_order_are_deterministic() {
    let input = page(vec![switch(0, &[1, 2]), terminal(1), terminal(2)]);
    let first = analyze(&input, CompilerComplexityLimits::default());
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(analyze(&input, CompilerComplexityLimits::default())).unwrap()
    );
    let mut reversed = input.clone();
    reversed.body.blocks.reverse();
    let second = analyze(&reversed, CompilerComplexityLimits::default());
    assert_eq!(first.metrics, second.metrics);
    assert_eq!(first.reachable_blocks, second.reachable_blocks);
    assert_eq!(first.exit_sites, second.exit_sites);
    let changed = analyze(
        &input,
        CompilerComplexityLimits {
            max_edges: 1999,
            ..Default::default()
        },
    );
    assert_ne!(first.input_digest, changed.input_digest);
    assert!(atlas_analysis::typescript().contains("export type CompilerComplexity"));
}

#[test]
fn independent_branch_excess_oracle_matches_32_dags() {
    for seed in 0..32_u32 {
        let adjacency = (0..6)
            .map(|from| {
                ((from + 1)..6)
                    .filter(|to| (seed.wrapping_mul(17) + from * 5 + to * 3) % (to + 2) < 2)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let blocks = adjacency
            .iter()
            .enumerate()
            .map(|(index, targets)| match targets.len() {
                0 => terminal(index as u32),
                1 => goto(index as u32, targets[0]),
                _ => switch(index as u32, targets),
            })
            .collect();
        let mut reachable = [false; 6];
        reachable[0] = true;
        for from in 0..6 {
            if reachable[from] {
                for target in &adjacency[from] {
                    reachable[*target as usize] = true;
                }
            }
        }
        let expected = 1 + adjacency
            .iter()
            .enumerate()
            .filter(|(index, _)| reachable[*index])
            .map(|(_, edges)| edges.len().saturating_sub(1) as u32)
            .sum::<u32>();
        let measured = run(blocks);
        assert_eq!(
            measured.metrics.unwrap().cyclomatic,
            expected,
            "independent DAG seed {seed}"
        );
    }
}
