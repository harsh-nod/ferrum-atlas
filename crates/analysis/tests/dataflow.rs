use atlas_analysis::{
    AnalysisControl, DataflowLimits, LocalDefinition, MirPoint, ReachingDefinitions,
    reaching_definitions,
};
use atlas_model::{
    CompilerBlock, CompilerBody, CompilerEdgeKind, CompilerLocal, CompilerLocalEffects,
    CompilerSourceMapping, CompilerStatement, CompilerSuccessor, CompilerTerminator,
    CompilerUnknownEffect, Coverage, Status,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

fn mapping() -> CompilerSourceMapping {
    CompilerSourceMapping::Unavailable {
        reason: "hand-authored CFG".into(),
    }
}

fn effects(defs: &[u32], uses: &[u32]) -> CompilerLocalEffects {
    CompilerLocalEffects {
        defs: defs.to_vec(),
        uses: uses.to_vec(),
        ..Default::default()
    }
}

fn edge(target: u32, kind: CompilerEdgeKind) -> CompilerSuccessor {
    CompilerSuccessor {
        target,
        kind,
        switch_value: None,
    }
}

fn block(
    index: u32,
    statements: Vec<CompilerLocalEffects>,
    uses: &[u32],
    successors: Vec<CompilerSuccessor>,
) -> CompilerBlock {
    CompilerBlock {
        index,
        is_cleanup: false,
        statements: statements
            .into_iter()
            .enumerate()
            .map(|(index, locals)| CompilerStatement {
                index: index as u32,
                kind: "assign".into(),
                source_scope: 0,
                span: mapping(),
                locals,
            })
            .collect(),
        terminator: CompilerTerminator {
            kind: if successors.is_empty() {
                "return"
            } else {
                "goto"
            }
            .into(),
            source_scope: 0,
            span: mapping(),
            locals: effects(&[], uses),
            normal_return_defs: vec![],
            successors,
            unwind: None,
            call_target: None,
            assert_expected: None,
        },
    }
}

fn body(blocks: Vec<CompilerBlock>, arguments: u32) -> CompilerBody {
    CompilerBody {
        body_id: "mir:hand-reviewed".into(),
        def_path: "fixture::entry".into(),
        kind: "function".into(),
        span: mapping(),
        argument_count: arguments,
        locals: (0..5)
            .map(|index| CompilerLocal {
                index,
                role: "local".into(),
                names: vec![],
                type_display: "u32".into(),
                source_scope: 0,
                span: mapping(),
            })
            .collect(),
        source_scopes: vec![],
        blocks,
    }
}

fn run(body: &CompilerBody) -> ReachingDefinitions {
    reaching_definitions(
        body,
        &Coverage::complete(),
        DataflowLimits::default(),
        &AnalysisControl::new(Duration::from_secs(5)),
    )
    .unwrap()
}

fn statement(block: u32, index: u32) -> MirPoint {
    MirPoint::Statement { block, index }
}
fn definition(local: u32, point: MirPoint) -> LocalDefinition {
    LocalDefinition { local, point }
}
fn used<'a>(
    result: &'a ReachingDefinitions,
    point: &MirPoint,
    local: u32,
) -> &'a atlas_analysis::LocalUse {
    result
        .uses
        .iter()
        .find(|usage| usage.point == *point && usage.local == local)
        .unwrap()
}

#[test]
fn reads_precede_assignments_and_new_definitions_kill_old_ones() {
    let input = body(
        vec![block(
            0,
            vec![
                effects(&[1], &[1]),
                effects(&[1], &[1]),
                effects(&[0], &[1]),
            ],
            &[0],
            vec![],
        )],
        1,
    );
    let result = run(&input);
    assert!(result.fixed_point);
    assert_eq!(result.envelope.coverage.status, Status::Complete);
    assert_eq!(
        used(&result, &statement(0, 0), 1).reaching,
        [definition(1, MirPoint::Entry)]
    );
    assert_eq!(
        used(&result, &statement(0, 1), 1).reaching,
        [definition(1, statement(0, 0))]
    );
    assert_eq!(
        used(&result, &statement(0, 2), 1).reaching,
        [definition(1, statement(0, 1))]
    );
    assert_eq!(
        used(&result, &MirPoint::Terminator { block: 0 }, 0).reaching,
        [definition(0, statement(0, 2))]
    );
    assert!(!result.uses.iter().any(|usage| usage.may_be_uninitialized));
    assert!(
        !result
            .uses
            .iter()
            .any(|usage| usage.possibly_changed_by_unknown_memory)
    );
}

#[test]
fn diamond_merges_reaching_definitions_and_tracks_missing_path_values() {
    let input = body(
        vec![
            block(
                0,
                vec![],
                &[1],
                vec![
                    edge(1, CompilerEdgeKind::SwitchValue),
                    edge(2, CompilerEdgeKind::Otherwise),
                ],
            ),
            block(
                1,
                vec![effects(&[2, 3], &[])],
                &[],
                vec![edge(3, CompilerEdgeKind::Normal)],
            ),
            block(
                2,
                vec![effects(&[2], &[])],
                &[],
                vec![edge(3, CompilerEdgeKind::Normal)],
            ),
            block(3, vec![], &[2, 3], vec![]),
        ],
        1,
    );
    let result = run(&input);
    let point = MirPoint::Terminator { block: 3 };
    assert_eq!(
        used(&result, &point, 2).reaching,
        [
            definition(2, statement(1, 0)),
            definition(2, statement(2, 0))
        ]
    );
    assert!(!used(&result, &point, 2).may_be_uninitialized);
    assert_eq!(
        used(&result, &point, 3).reaching,
        [definition(3, statement(1, 0))]
    );
    assert!(used(&result, &point, 3).may_be_uninitialized);
}

#[test]
fn loops_converge_and_preserve_entry_boundary_definitions() {
    let input = body(
        vec![
            block(
                0,
                vec![effects(&[], &[1])],
                &[],
                vec![edge(1, CompilerEdgeKind::Normal)],
            ),
            block(
                1,
                vec![effects(&[1], &[1])],
                &[],
                vec![
                    edge(0, CompilerEdgeKind::SwitchValue),
                    edge(2, CompilerEdgeKind::Otherwise),
                ],
            ),
            block(2, vec![], &[1], vec![]),
        ],
        1,
    );
    let result = run(&input);
    assert!(result.fixed_point);
    assert!(result.iterations > 3);
    assert_eq!(
        used(&result, &statement(0, 0), 1).reaching,
        [
            definition(1, MirPoint::Entry),
            definition(1, statement(1, 0))
        ]
    );
    assert_eq!(
        used(&result, &MirPoint::Terminator { block: 2 }, 1).reaching,
        [definition(1, statement(1, 0))]
    );
    assert_eq!(observed(&result), oracle(&input, 12, Mutation::None));
    assert_eq!(
        oracle(&input, 6, Mutation::None),
        oracle(&input, 12, Mutation::None)
    );
}

fn call_fixture() -> CompilerBody {
    let mut call = block(
        0,
        vec![],
        &[1],
        vec![
            edge(1, CompilerEdgeKind::Normal),
            edge(2, CompilerEdgeKind::Unwind),
        ],
    );
    call.terminator.kind = "call".into();
    call.terminator.normal_return_defs = vec![1];
    call.terminator.locals.unknown_effects = vec![CompilerUnknownEffect::Call];
    let mut cleanup = block(2, vec![], &[1], vec![]);
    cleanup.is_cleanup = true;
    body(vec![call, block(1, vec![], &[1], vec![]), cleanup], 1)
}

#[test]
fn call_destination_is_defined_only_on_normal_return_not_unwind() {
    let input = call_fixture();
    let result = run(&input);
    let normal = used(&result, &MirPoint::Terminator { block: 1 }, 1);
    let unwind = used(&result, &MirPoint::Terminator { block: 2 }, 1);
    assert_eq!(
        normal.reaching,
        [definition(
            1,
            MirPoint::NormalReturn {
                block: 0,
                target: 1
            }
        )]
    );
    assert_eq!(unwind.reaching, [definition(1, MirPoint::Entry)]);
    assert!(normal.possibly_changed_by_unknown_memory && unwind.possibly_changed_by_unknown_memory);
    assert_eq!(
        result.unknown_memory_effects[0].effects,
        [CompilerUnknownEffect::Call]
    );
    assert_eq!(result.envelope.coverage.status, Status::Partial);
}

#[test]
fn suspension_is_an_unknown_external_boundary_even_without_raw_effects() {
    let mut suspension = block(
        0,
        vec![],
        &[1],
        vec![
            edge(1, CompilerEdgeKind::Resume),
            edge(2, CompilerEdgeKind::CoroutineDrop),
        ],
    );
    suspension.terminator.kind = "yield".into();
    let input = body(
        vec![
            suspension,
            block(1, vec![], &[1], vec![]),
            block(2, vec![], &[1], vec![]),
        ],
        1,
    );
    let result = run(&input);
    assert!(result.fixed_point);
    assert_eq!(result.envelope.coverage.status, Status::Partial);
    assert_eq!(
        result.unknown_memory_effects[0].effects,
        [CompilerUnknownEffect::ExternalState]
    );
    for block in 0..=2 {
        let usage = used(&result, &MirPoint::Terminator { block }, 1);
        assert!(usage.possibly_changed_by_unknown_memory);
        assert_eq!(usage.reaching, [definition(1, MirPoint::Entry)]);
    }
    assert_eq!(result.definitions, [definition(1, MirPoint::Entry)]);
}

#[test]
fn moves_and_storage_lifetimes_clear_tracking_after_reads() {
    let mut moved = effects(&[2], &[1]);
    moved.moves = vec![1];
    let mut live = effects(&[], &[]);
    live.storage_live = vec![2];
    let mut dead = effects(&[], &[]);
    dead.storage_dead = vec![3];
    let input = body(
        vec![block(
            0,
            vec![
                moved,
                effects(&[], &[1, 2]),
                live,
                effects(&[3], &[2]),
                dead,
            ],
            &[3],
            vec![],
        )],
        1,
    );
    let result = run(&input);
    assert_eq!(
        used(&result, &statement(0, 0), 1).reaching,
        [definition(1, MirPoint::Entry)]
    );
    assert!(used(&result, &statement(0, 1), 1).may_be_uninitialized);
    assert!(used(&result, &statement(0, 1), 1).reaching.is_empty());
    assert!(!used(&result, &statement(0, 1), 2).may_be_uninitialized);
    assert!(used(&result, &statement(0, 3), 2).may_be_uninitialized);
    assert!(used(&result, &MirPoint::Terminator { block: 0 }, 3).may_be_uninitialized);
}

#[test]
fn unknown_memory_is_preserved_across_joins_without_inventing_local_writes() {
    let mut unknown = effects(&[], &[1]);
    unknown.unknown_effects = vec![
        CompilerUnknownEffect::PointerAliasing,
        CompilerUnknownEffect::PartialWrite,
    ];
    let input = body(
        vec![
            block(
                0,
                vec![effects(&[], &[1])],
                &[],
                vec![
                    edge(1, CompilerEdgeKind::SwitchValue),
                    edge(2, CompilerEdgeKind::Otherwise),
                ],
            ),
            block(
                1,
                vec![unknown],
                &[],
                vec![edge(3, CompilerEdgeKind::Normal)],
            ),
            block(2, vec![], &[], vec![edge(3, CompilerEdgeKind::Normal)]),
            block(3, vec![effects(&[1], &[])], &[1], vec![]),
        ],
        1,
    );
    let result = run(&input);
    assert!(!used(&result, &statement(0, 0), 1).possibly_changed_by_unknown_memory);
    assert!(used(&result, &statement(1, 0), 1).possibly_changed_by_unknown_memory);
    let after = used(&result, &MirPoint::Terminator { block: 3 }, 1);
    assert!(after.possibly_changed_by_unknown_memory);
    assert_eq!(after.reaching, [definition(1, statement(3, 0))]);
    assert!(!result.definitions.contains(&definition(1, statement(1, 0))));
}

#[test]
fn unreachable_and_imaginary_only_blocks_produce_no_claims() {
    let mut unreachable = effects(&[1], &[4]);
    unreachable.unknown_effects = vec![CompilerUnknownEffect::InlineAssembly];
    let input = body(
        vec![
            block(0, vec![], &[1], vec![edge(1, CompilerEdgeKind::Imaginary)]),
            block(1, vec![unreachable], &[1], vec![]),
        ],
        1,
    );
    let result = run(&input);
    assert_eq!(result.reachable_blocks, [0]);
    assert_eq!(result.definitions, [definition(1, MirPoint::Entry)]);
    assert_eq!(result.uses.len(), 1);
    assert!(result.unknown_memory_effects.is_empty());
    assert_eq!(result.envelope.coverage.status, Status::Partial);
}

#[test]
fn input_and_iteration_limits_never_return_empty_complete_success() {
    let input = call_fixture();
    for limits in [
        DataflowLimits {
            max_blocks: 1,
            ..Default::default()
        },
        DataflowLimits {
            max_locals: 1,
            ..Default::default()
        },
        DataflowLimits {
            max_facts: 1,
            ..Default::default()
        },
        DataflowLimits {
            max_iterations: 1,
            ..Default::default()
        },
    ] {
        let result = reaching_definitions(
            &input,
            &Coverage::complete(),
            limits,
            &AnalysisControl::new(Duration::from_secs(1)),
        )
        .unwrap();
        assert!(!result.fixed_point);
        assert!(result.envelope.truncated);
        assert_eq!(result.envelope.coverage.status, Status::Partial);
        assert!(result.uses.is_empty());
    }
    for cancelled in [true, false] {
        let control = AnalysisControl::new(if cancelled {
            Duration::from_secs(1)
        } else {
            Duration::ZERO
        });
        if cancelled {
            control.cancel();
        }
        let result = reaching_definitions(
            &input,
            &Coverage::complete(),
            DataflowLimits::default(),
            &control,
        )
        .unwrap();
        assert!(!result.fixed_point);
        assert!(result.envelope.truncated);
        assert_eq!(result.envelope.cancelled, cancelled);
        assert_eq!(result.envelope.deadline_reached, !cancelled);
    }
}

#[test]
fn retained_state_and_response_budgets_are_enforced() {
    let input = body(
        (0..30)
            .map(|index| {
                block(
                    index,
                    vec![effects(&[1, 2, 3, 4], &[])],
                    &[1, 2, 3, 4],
                    if index < 29 {
                        vec![edge(index + 1, CompilerEdgeKind::Normal)]
                    } else {
                        vec![]
                    },
                )
            })
            .collect(),
        1,
    );
    let result = reaching_definitions(
        &input,
        &Coverage::complete(),
        DataflowLimits {
            max_response_bytes: 2048,
            ..Default::default()
        },
        &AnalysisControl::new(Duration::from_secs(1)),
    )
    .unwrap();
    assert!(result.fixed_point);
    assert!(result.envelope.truncated);
    assert!(result.uses.is_empty());
    assert!(serde_json::to_vec(&result).unwrap().len() <= 2048);

    let mut input = body(
        (0..25)
            .map(|index| {
                block(
                    index,
                    vec![],
                    &[],
                    if index < 24 {
                        vec![edge(index + 1, CompilerEdgeKind::Normal)]
                    } else {
                        vec![]
                    },
                )
            })
            .collect(),
        4,
    );
    input.blocks[0].statements.push(CompilerStatement {
        index: 0,
        kind: "assign".into(),
        source_scope: 0,
        span: mapping(),
        locals: effects(&[0], &[]),
    });
    let result = reaching_definitions(
        &input,
        &Coverage::complete(),
        DataflowLimits {
            max_facts: 100,
            ..Default::default()
        },
        &AnalysisControl::new(Duration::from_secs(1)),
    )
    .unwrap();
    assert!(!result.fixed_point);
    assert!(result.envelope.truncated);
    assert!(
        result
            .envelope
            .coverage
            .limitations
            .iter()
            .any(|reason| reason.contains("retained"))
    );
}

#[test]
fn malformed_references_and_statement_order_are_rejected() {
    let valid = call_fixture();
    for mutation in 0..7 {
        let mut input = valid.clone();
        match mutation {
            0 => input.blocks[0].terminator.successors[0].target = 99,
            1 => input.blocks[0].terminator.locals.uses = vec![99],
            2 => input.blocks[0].terminator.normal_return_defs = vec![99],
            3 => input.blocks[1].index = 0,
            4 => input.locals[1].index = 0,
            5 => input.argument_count = 99,
            6 => input.blocks[0].statements.push(CompilerStatement {
                index: 7,
                kind: "assign".into(),
                source_scope: 0,
                span: mapping(),
                locals: effects(&[0], &[]),
            }),
            _ => unreachable!(),
        }
        assert!(
            reaching_definitions(
                &input,
                &Coverage::complete(),
                DataflowLimits::default(),
                &AnalysisControl::new(Duration::from_secs(1))
            )
            .is_err(),
            "mutation {mutation}"
        );
    }
    assert!(
        reaching_definitions(
            &valid,
            &Coverage::complete(),
            DataflowLimits {
                max_iterations: 0,
                ..Default::default()
            },
            &AnalysisControl::new(Duration::from_secs(1))
        )
        .is_err()
    );
}

#[test]
fn output_fact_exhaustion_marks_a_converged_result_as_incomplete() {
    let mut blocks = vec![block(
        0,
        vec![],
        &[],
        (1..=10)
            .map(|target| edge(target, CompilerEdgeKind::SwitchValue))
            .collect(),
    )];
    blocks.extend((1..=10).map(|index| {
        block(
            index,
            vec![effects(&[1], &[])],
            &[],
            vec![edge(11, CompilerEdgeKind::Normal)],
        )
    }));
    blocks.push(block(11, vec![effects(&[], &[1]); 100], &[], vec![]));
    let input = body(blocks, 0);
    let result = reaching_definitions(
        &input,
        &Coverage::complete(),
        DataflowLimits {
            max_facts: 500,
            ..Default::default()
        },
        &AnalysisControl::new(Duration::from_secs(1)),
    )
    .unwrap();
    assert!(result.fixed_point);
    assert!(result.envelope.truncated);
    assert!(!result.uses.is_empty());
    assert!(result.uses.len() < 100);
    assert!(
        result
            .envelope
            .coverage
            .limitations
            .iter()
            .any(|reason| reason.contains("output exceeds"))
    );
    assert_eq!(result.uses[0].reaching.len(), 10);
}

#[test]
fn ordered_output_is_independent_of_input_container_order_and_inherits_coverage() {
    let mut input = call_fixture();
    let first = serde_json::to_value(run(&input)).unwrap();
    input.blocks.reverse();
    input.locals.reverse();
    for block in &mut input.blocks {
        block.terminator.successors.reverse();
    }
    let result = run(&input);
    assert_eq!(serde_json::to_value(&result).unwrap(), first);
    let roundtrip: ReachingDefinitions = serde_json::from_value(first).unwrap();
    assert_eq!(roundtrip.uses, result.uses);
    let coverage = Coverage::partial(
        atlas_model::UnknownReason::ArtifactMismatch,
        "selected mapping incomplete",
    );
    let result = reaching_definitions(
        &input,
        &coverage,
        DataflowLimits::default(),
        &AnalysisControl::new(Duration::from_secs(1)),
    )
    .unwrap();
    assert!(
        result
            .envelope
            .coverage
            .limitations
            .contains(&"selected mapping incomplete".into())
    );
}

type Observation = BTreeMap<(MirPoint, u32), (BTreeSet<LocalDefinition>, bool)>;

fn observed(result: &ReachingDefinitions) -> Observation {
    result
        .uses
        .iter()
        .map(|usage| {
            (
                (usage.point.clone(), usage.local),
                (
                    usage.reaching.iter().cloned().collect(),
                    usage.may_be_uninitialized,
                ),
            )
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mutation {
    None,
    ForgetKills,
    ReturnOnUnwind,
}

// Independent, bounded path enumeration: each path holds one current assignment,
// with no worklist, joins, or production transfer functions.
fn oracle(input: &CompilerBody, max_depth: usize, mutation: Mutation) -> Observation {
    let blocks: BTreeMap<_, _> = input
        .blocks
        .iter()
        .map(|block| (block.index, block))
        .collect();
    let initial: BTreeMap<_, _> = (1..=input.argument_count)
        .map(|local| (local, BTreeSet::from([definition(local, MirPoint::Entry)])))
        .collect();
    let mut paths = vec![(0, initial, 0)];
    let mut result: Observation = BTreeMap::new();
    while let Some((index, mut current, depth)) = paths.pop() {
        if depth == max_depth {
            continue;
        }
        let block = blocks[&index];
        for (point, effects) in block
            .statements
            .iter()
            .map(|statement| {
                (
                    MirPoint::Statement {
                        block: index,
                        index: statement.index,
                    },
                    &statement.locals,
                )
            })
            .chain([(
                MirPoint::Terminator { block: index },
                &block.terminator.locals,
            )])
        {
            for local in effects.uses.iter().chain(&effects.moves) {
                let aggregate = result.entry((point.clone(), *local)).or_default();
                match current.get(local) {
                    Some(definitions) if !definitions.is_empty() => {
                        aggregate.0.extend(definitions.clone())
                    }
                    _ => aggregate.1 = true,
                }
            }
            for local in effects
                .moves
                .iter()
                .chain(&effects.storage_live)
                .chain(&effects.storage_dead)
            {
                current.remove(local);
            }
            for local in &effects.defs {
                if mutation != Mutation::ForgetKills {
                    current.remove(local);
                }
                current
                    .entry(*local)
                    .or_default()
                    .insert(definition(*local, point.clone()));
            }
        }
        for edge in &block.terminator.successors {
            if edge.kind == CompilerEdgeKind::Imaginary {
                continue;
            }
            let mut next = current.clone();
            if edge.kind == CompilerEdgeKind::Normal || mutation == Mutation::ReturnOnUnwind {
                for local in &block.terminator.normal_return_defs {
                    next.insert(
                        *local,
                        BTreeSet::from([definition(
                            *local,
                            MirPoint::NormalReturn {
                                block: index,
                                target: edge.target,
                            },
                        )]),
                    );
                }
            }
            paths.push((edge.target, next, depth + 1));
        }
    }
    result
}

#[test]
fn independent_path_oracle_matches_twenty_four_deterministic_dags() {
    for seed in 1..=24_u64 {
        let mut random = seed;
        let mut next = || {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            (random >> 32) as u32
        };
        let count = 4 + next() % 5;
        let input = body(
            (0..count)
                .map(|index| {
                    let assigned = next() % 5;
                    let read = next() % 5;
                    let mut statements = vec![effects(&[assigned], &[read])];
                    if next() % 2 == 0 {
                        statements.push(effects(&[next() % 5], &[assigned]));
                    }
                    let successors = if index + 1 == count {
                        vec![]
                    } else {
                        vec![
                            edge(index + 1, CompilerEdgeKind::SwitchValue),
                            edge(
                                index + 1 + next() % (count - index - 1),
                                CompilerEdgeKind::Otherwise,
                            ),
                        ]
                    };
                    block(index, statements, &[next() % 5], successors)
                })
                .collect(),
            2,
        );
        let actual = run(&input);
        assert!(actual.fixed_point);
        assert_eq!(
            observed(&actual),
            oracle(&input, count as usize + 1, Mutation::None),
            "seed {seed}"
        );
    }
}

#[test]
fn oracle_fixtures_detect_missing_kills_and_call_on_unwind_mutations() {
    let input = body(
        vec![block(
            0,
            vec![effects(&[1], &[]), effects(&[1], &[])],
            &[1],
            vec![],
        )],
        1,
    );
    assert_ne!(
        observed(&run(&input)),
        oracle(&input, 4, Mutation::ForgetKills)
    );
    let call = call_fixture();
    assert_ne!(
        observed(&run(&call)),
        oracle(&call, 4, Mutation::ReturnOnUnwind)
    );
    assert_eq!(observed(&run(&input)), oracle(&input, 4, Mutation::None));
    assert_eq!(observed(&run(&call)), oracle(&call, 4, Mutation::None));
}
