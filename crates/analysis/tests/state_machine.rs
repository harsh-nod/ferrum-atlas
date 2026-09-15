use atlas_analysis::*;
use atlas_model::*;
use std::time::Duration;

fn fixture(text: &str) -> (SourceFile, Definition) {
    let file_id = FileId("fixture".into());
    let start = text.find("fn event").unwrap() as u32;
    let end = text.rfind('}').unwrap() as u32 + 1;
    let body_start = text[start as usize..].find('{').unwrap() as u32 + start;
    let source = SourceFile {
        id: file_id.clone(),
        path: "src/state.rs".into(),
        content_hash: digest("test-source", text),
        text: text.into(),
    };
    let definition = Definition {
        id: DefinitionId("event".into()),
        context_id: ContextId("context".into()),
        file_id: file_id.clone(),
        name: "event".into(),
        qualified_name: "fixture::event".into(),
        kind: "function".into(),
        parent_id: None,
        signature: "fn event(mut state: State)".into(),
        signature_hash: "signature".into(),
        body_hash: "body".into(),
        span: Span {
            file_id: file_id.clone(),
            start,
            end,
        },
        body_span: Some(Span {
            file_id,
            start: body_start,
            end,
        }),
        cfg: vec![],
        cfg_status: CfgStatus::Active,
        visibility: "private".into(),
        metrics: Metrics::default(),
    };
    (source, definition)
}
fn control() -> AnalysisControl {
    AnalysisControl::new(Duration::from_secs(10))
}
fn run(text: &str, limits: StateMachineLimits) -> StateMachineInference {
    let (source, definition) = fixture(text);
    infer_state_machine(&source, &definition, "State", "state", limits, &control()).unwrap()
}
const SIMPLE: &str = "fn event(mut state: State) { match state { State::Idle => state = State::Ready, State::Ready => { state = State::Done; }, _ => {} } }";
fn excerpt<'a>(source: &'a str, span: &Span) -> &'a str {
    &source[span.start as usize..span.end as usize]
}

#[test]
fn independent_oracle_matches_exact_variants_actions_and_spans() {
    let report = run(SIMPLE, StateMachineLimits::default());
    let pairs = report
        .candidates
        .iter()
        .map(|c| (c.from_variant.as_str(), c.to_variant.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(pairs, [("Idle", "Ready"), ("Ready", "Done")]);
    assert_eq!(report.unknowns.len(), 1);
    assert_eq!(report.envelope.coverage.status, Status::Partial);
    assert!(!report.envelope.truncated);
    assert_eq!(
        excerpt(SIMPLE, &report.candidates[0].assignment_span),
        "state = State::Ready"
    );
    assert_eq!(
        excerpt(SIMPLE, &report.candidates[1].action.span),
        "{ state = State::Done; }"
    );
    assert_ne!(report.candidates[0].id, report.candidates[1].id);
    for candidate in &report.candidates {
        assert_eq!(
            excerpt(SIMPLE, &candidate.action.span),
            candidate.action.text
        );
        assert!(candidate.match_span.start <= candidate.arm_span.start);
        assert!(candidate.arm_span.end <= candidate.match_span.end);
    }
}

#[test]
fn deliberate_wrong_target_mutations_are_detected() {
    let original = run(SIMPLE, StateMachineLimits::default());
    let wrong_target = run(
        &SIMPLE.replace("State::Done", "State::Wrong"),
        StateMachineLimits::default(),
    );
    assert_ne!(
        original.candidates[1].to_variant,
        wrong_target.candidates[1].to_variant
    );
    assert_ne!(original.input_digest, wrong_target.input_digest);
    for mutation in [
        SIMPLE.replace("state = State::Ready", "other = State::Ready"),
        SIMPLE.replace("state = State::Ready", "state = Other::Ready"),
    ] {
        let mutated = run(&mutation, StateMachineLimits::default());
        assert_eq!(mutated.candidates.len(), 1);
        assert!(mutated.unknowns.len() >= 2);
    }
}

#[test]
fn utf8_crlf_offsets_guards_and_raw_identifiers_are_exact() {
    let text = "// snow: \u{96ea}\r\nfn event(mut r#state: State, ready: bool) {\r\n match r#state { State::Idle if ready && true => { r#state = State::Ready; }, _ => {} }\r\n}";
    let report = run(text, StateMachineLimits::default());
    assert_eq!(report.candidates.len(), 1);
    let candidate = &report.candidates[0];
    let guard = candidate.guard.as_ref().unwrap();
    assert_eq!(guard.text, "if ready && true");
    assert_eq!(excerpt(text, &guard.span), guard.text);
    assert_eq!(
        excerpt(text, &candidate.assignment_span),
        "r#state = State::Ready"
    );
    assert_eq!(excerpt(text, &report.body_span).chars().next(), Some('{'));
}

#[test]
fn direct_top_level_local_is_supported_but_nested_matches_are_unknown() {
    let text = "fn event() { let mut state = State::Idle; match state { State::Idle => state = State::Ready, _ => {} }; if true { match state { State::Ready => state = State::Done, _ => {} } } }";
    let report = run(text, StateMachineLimits::default());
    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.candidates[0].from_variant, "Idle");
    assert!(
        report
            .unknowns
            .iter()
            .any(|u| u.reason.contains("top-level"))
    );
}

#[test]
fn ambiguous_bindings_aliases_and_macros_withhold_all_candidates() {
    for extra in [
        "let state = State::Done;",
        "{ let state = State::Done; }",
        "let alias = state;",
        "let alias = &mut state;",
        "change_state!();",
        "fn nested(state: State) {}",
    ] {
        let text = SIMPLE.replacen("{ match state", &format!("{{ {extra} match state"), 1);
        let report = run(&text, StateMachineLimits::default());
        assert!(report.candidates.is_empty(), "{extra}");
        assert!(!report.unknowns.is_empty(), "{extra}");
        assert_eq!(report.envelope.coverage.status, Status::Partial);
    }
    for text in [
        SIMPLE.replace("mut state: State", "state: &mut State"),
        SIMPLE.replace("mut state: State", "ref mut state: State"),
        SIMPLE.replace("mut state: State", "other: State"),
    ] {
        assert!(
            run(&text, StateMachineLimits::default())
                .candidates
                .is_empty()
        );
    }
}

#[test]
fn unsupported_patterns_actions_guards_and_members_are_not_inferred() {
    for arm in [
        "State::Idle | State::Ready => state = State::Done",
        "State::Idle(value) => state = State::Done",
        "State::Idle { value } => state = State::Done",
        "State::Idle => { action(); state = State::Done; }",
        "State::Idle => { state = State::Ready; state = State::Done; }",
        "State::Idle => { if true { state = State::Done; } }",
        "State::Idle => state += State::Done",
        "State::Idle => self.state = State::Done",
        "State::Idle => *state = State::Done",
        "State::Idle => state = State::Done(value)",
        "State::Idle if check() => state = State::Done",
        "State::Idle if { state = State::Ready; true } => state = State::Done",
        "#[cfg(any())] State::Idle => state = State::Done",
    ] {
        let text = format!("fn event(mut state: State) {{ match state {{ {arm}, _ => {{}} }} }}");
        let report = run(&text, StateMachineLimits::default());
        assert!(report.candidates.is_empty(), "{arm}");
        assert!(!report.unknowns.is_empty(), "{arm}");
    }
}

#[test]
fn comments_strings_and_nested_functions_cannot_fabricate_transitions() {
    let text = "fn event(mut state: State) { let text = \"match state { State::Idle => state = State::Done }\"; /* state = State::Done */ fn inner() { match state { State::Idle => state = State::Done, _ => {} } } }";
    let report = run(text, StateMachineLimits::default());
    assert!(report.candidates.is_empty());
    assert!(!report.unknowns.is_empty());
}

#[test]
fn parser_errors_and_inactive_context_are_partial_empty_results() {
    assert!(
        run(
            "fn event(mut state: State) { match state { State::Idle => state = } }",
            StateMachineLimits::default()
        )
        .candidates
        .is_empty()
    );
    let (source, mut definition) = fixture(SIMPLE);
    for cfg in [CfgStatus::Inactive, CfgStatus::Unknown] {
        definition.cfg_status = cfg;
        let report = infer_state_machine(
            &source,
            &definition,
            "State",
            "state",
            StateMachineLimits::default(),
            &control(),
        )
        .unwrap();
        assert!(report.candidates.is_empty());
        assert!(!report.unknowns.is_empty());
        assert_eq!(report.envelope.coverage.status, Status::Partial);
    }
}

#[test]
fn function_and_parameter_attributes_withhold_candidates_even_when_marked_active() {
    for prefix in ["#[cfg(any())]\n", "#[rewrite_body]\n", "#[allow(unused)]\n"] {
        let text = format!("{prefix}{SIMPLE}");
        let (source, mut definition) = fixture(&text);
        definition.span.start = 0;
        let report = infer_state_machine(
            &source,
            &definition,
            "State",
            "state",
            StateMachineLimits::default(),
            &control(),
        )
        .unwrap();
        assert!(report.candidates.is_empty());
        assert!(
            report
                .unknowns
                .iter()
                .any(|record| record.reason.contains("Attributes"))
        );
    }
    let parameter = SIMPLE.replace(
        "mut state: State",
        "#[cfg(feature = \"x\")] mut state: State",
    );
    assert!(
        run(&parameter, StateMachineLimits::default())
            .candidates
            .is_empty()
    );
}

#[test]
fn budgets_are_explicit_and_never_report_complete_negatives() {
    for limits in [
        StateMachineLimits {
            max_body_bytes: 1,
            ..Default::default()
        },
        StateMachineLimits {
            max_nodes: 1,
            ..Default::default()
        },
    ] {
        let report = run(SIMPLE, limits);
        assert!(report.candidates.is_empty());
        assert!(report.envelope.truncated);
        assert_eq!(report.envelope.coverage.status, Status::Partial);
    }
    let one = run(
        SIMPLE,
        StateMachineLimits {
            max_transitions: 1,
            ..Default::default()
        },
    );
    assert_eq!(one.candidates.len(), 1);
    assert!(one.envelope.truncated);
    let unknowns = run(
        "fn event(mut state: State) { match state { _ => {}, State::A | State::B => {} } }",
        StateMachineLimits {
            max_unknowns: 1,
            ..Default::default()
        },
    );
    assert_eq!(unknowns.unknowns.len(), 1);
    assert!(unknowns.envelope.truncated);
    let response = run(
        SIMPLE,
        StateMachineLimits {
            max_response_bytes: 1600,
            ..Default::default()
        },
    );
    assert!(response.candidates.is_empty());
    assert!(response.envelope.truncated);
    assert!(serde_json::to_vec(&response).unwrap().len() <= 1600);
}

#[test]
fn cancellation_and_deadline_prevent_candidates() {
    let (source, definition) = fixture(SIMPLE);
    let cancelled = control();
    cancelled.cancel();
    for (control, cancellation) in [
        (cancelled, true),
        (AnalysisControl::new(Duration::ZERO), false),
    ] {
        let report = infer_state_machine(
            &source,
            &definition,
            "State",
            "state",
            StateMachineLimits::default(),
            &control,
        )
        .unwrap();
        assert!(report.candidates.is_empty());
        assert!(report.envelope.truncated);
        assert_eq!(report.envelope.cancelled, cancellation);
        assert_eq!(report.envelope.deadline_reached, !cancellation);
    }
}

#[test]
fn invalid_selection_spans_and_limits_are_rejected() {
    let (source, definition) = fixture(SIMPLE);
    for (en, state) in [
        ("State<T>", "state"),
        ("State", "self.state"),
        ("State", "*state"),
        ("State", "self"),
        ("", "state"),
    ] {
        assert!(
            infer_state_machine(
                &source,
                &definition,
                en,
                state,
                StateMachineLimits::default(),
                &control()
            )
            .is_err()
        );
    }
    let mut bad = definition.clone();
    bad.body_span.as_mut().unwrap().start += 1;
    assert!(
        infer_state_machine(
            &source,
            &bad,
            "State",
            "state",
            StateMachineLimits::default(),
            &control()
        )
        .is_err()
    );
    bad = definition.clone();
    bad.span.end += 10;
    assert!(
        infer_state_machine(
            &source,
            &bad,
            "State",
            "state",
            StateMachineLimits::default(),
            &control()
        )
        .is_err()
    );
    bad = definition.clone();
    bad.file_id = FileId("other".into());
    assert!(
        infer_state_machine(
            &source,
            &bad,
            "State",
            "state",
            StateMachineLimits::default(),
            &control()
        )
        .is_err()
    );
    assert!(
        infer_state_machine(
            &source,
            &definition,
            "State",
            "state",
            StateMachineLimits {
                max_transitions: 501,
                ..Default::default()
            },
            &control()
        )
        .is_err()
    );
}

#[test]
fn deterministic_digest_binds_context_source_selection_and_limits() {
    let report = run(SIMPLE, StateMachineLimits::default());
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(run(SIMPLE, StateMachineLimits::default())).unwrap()
    );
    let changed_limit = run(
        SIMPLE,
        StateMachineLimits {
            max_nodes: 999,
            ..Default::default()
        },
    );
    assert_ne!(report.input_digest, changed_limit.input_digest);
    let (source, mut definition) = fixture(SIMPLE);
    definition.context_id = ContextId("other-context".into());
    let changed_context = infer_state_machine(
        &source,
        &definition,
        "State",
        "state",
        StateMachineLimits::default(),
        &control(),
    )
    .unwrap();
    assert_ne!(report.input_digest, changed_context.input_digest);
}

fn review(report: &StateMachineInference) -> StateTransitionReview {
    StateTransitionReview {
        input_digest: report.input_digest.clone(),
        candidate_id: report.candidates[0].id.clone(),
        reviewer: "reviewer".into(),
        note: "Reviewed syntax only.\nNo feasibility claim.".into(),
        decision: StateTransitionDecision::Accepted,
    }
}

#[test]
fn reviews_are_digest_bound_declarations_without_proof_upgrade() {
    let report = run(SIMPLE, StateMachineLimits::default());
    let accepted = review(&report);
    validate_state_review(&report, &accepted).unwrap();
    let mut rejected = accepted.clone();
    rejected.decision = StateTransitionDecision::Rejected;
    validate_state_review(&report, &rejected).unwrap();
    assert_eq!(report.envelope.coverage.status, Status::Partial);
    let changed = run(
        &SIMPLE.replace("State::Done", "State::Wrong"),
        StateMachineLimits::default(),
    );
    assert!(validate_state_review(&changed, &accepted).is_err());
    let mut forged = report.clone();
    forged.candidates[0].to_variant = "Wrong".into();
    assert!(validate_state_review(&forged, &accepted).is_err());
    let mut absent = accepted;
    absent.candidate_id = "absent".into();
    assert!(validate_state_review(&report, &absent).is_err());
}

#[test]
fn review_metadata_and_unknown_fields_are_bounded() {
    let report = run(SIMPLE, StateMachineLimits::default());
    for reviewer in [
        "".into(),
        "   ".into(),
        "name\n".into(),
        "x".repeat(129),
        "\u{96ea}".repeat(43),
    ] {
        let mut declaration = review(&report);
        declaration.reviewer = reviewer;
        assert!(validate_state_review(&report, &declaration).is_err());
    }
    for note in ["x".repeat(4097), "text\0".into(), "text\u{7f}".into()] {
        let mut declaration = review(&report);
        declaration.note = note;
        assert!(validate_state_review(&report, &declaration).is_err());
    }
    let mut declaration = review(&report);
    declaration.reviewer = "x".repeat(128);
    declaration.note = "x".repeat(4096);
    validate_state_review(&report, &declaration).unwrap();
    let mut value = serde_json::to_value(declaration).unwrap();
    value["proof"] = true.into();
    assert!(serde_json::from_value::<StateTransitionReview>(value).is_err());
}

#[test]
fn real_capture_and_frontend_definition_body_spans_are_accepted() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("src")).unwrap();
    std::fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='state_machine_oracle'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("src/lib.rs"),
        format!("enum State {{ Idle, Ready, Done }}\n{SIMPLE}\n"),
    )
    .unwrap();
    let (source, context) =
        atlas_ingest::capture(directory.path(), &atlas_ingest::CaptureOptions::default()).unwrap();
    let facts =
        atlas_frontend::analyze(source, context, atlas_frontend::AnalysisLevel::Semantic).unwrap();
    let definition = facts
        .definitions
        .iter()
        .find(|definition| definition.name == "event")
        .unwrap();
    let file = facts
        .source
        .files
        .iter()
        .find(|file| file.id == definition.file_id)
        .unwrap();
    let report = infer_state_machine(
        file,
        definition,
        "State",
        "state",
        StateMachineLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(report.candidates.len(), 2);
    assert_eq!(Some(&report.body_span), definition.body_span.as_ref());
}

#[test]
fn namespaces_are_exact_and_do_not_resolve_aliases() {
    let text = SIMPLE.replace("State::", "crate::State::");
    let (source, definition) = fixture(&text);
    let exact = infer_state_machine(
        &source,
        &definition,
        "crate::State",
        "state",
        StateMachineLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(exact.candidates.len(), 2);
    assert!(
        run(&text, StateMachineLimits::default())
            .candidates
            .is_empty()
    );
}

#[test]
fn syntax_and_hard_input_budgets_withhold_or_reject_explicitly() {
    let text = format!(
        "fn event(mut state: State) {{ match state {{ State::Idle => {{ {}state = State::Ready; }}, _ => {{}} }} }}",
        " ".repeat(4096)
    );
    let report = run(&text, StateMachineLimits::default());
    assert!(report.candidates.is_empty());
    assert!(
        report
            .unknowns
            .iter()
            .any(|unknown| unknown.reason.contains("excerpt budget"))
    );
    let (source, mut definition) = fixture(SIMPLE);
    definition.signature = "x".repeat(65536);
    assert_eq!(
        infer_state_machine(
            &source,
            &definition,
            "State",
            "state",
            StateMachineLimits::default(),
            &control()
        )
        .unwrap_err(),
        AnalysisError::BudgetExhausted
    );
    let oversized = SIMPLE.replace(
        "{ match state",
        &format!("{{ {}match state", " ".repeat(1_048_576)),
    );
    let (source, definition) = fixture(&oversized);
    assert_eq!(
        infer_state_machine(
            &source,
            &definition,
            "State",
            "state",
            StateMachineLimits::default(),
            &control()
        )
        .unwrap_err(),
        AnalysisError::BudgetExhausted
    );
}
