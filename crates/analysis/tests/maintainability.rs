use atlas_analysis::*;
use atlas_model::*;
use std::time::Duration;

fn fixture(fragment: &str) -> (SourceFile, Definition) {
    let prefix = "// prefix: caf\u{e9}\r\n";
    let file_id = FileId("source:metrics".into());
    let start = prefix.len() as u32;
    let end = start + fragment.len() as u32;
    let body_start = start + fragment.find('{').unwrap() as u32;
    let source = SourceFile {
        id: file_id.clone(),
        path: "src/metrics.rs".into(),
        content_hash: digest("metrics-fixture", fragment),
        text: format!("{prefix}{fragment}"),
    };
    let definition = Definition {
        id: DefinitionId("definition:event".into()),
        context_id: ContextId("context:metrics".into()),
        file_id: file_id.clone(),
        name: "event".into(),
        qualified_name: "metrics::event".into(),
        kind: "function".into(),
        parent_id: None,
        signature: "fn event()".into(),
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
    AnalysisControl::new(Duration::from_secs(5))
}
fn report(fragment: &str, limits: MaintainabilityLimits) -> SourceMaintainability {
    let (source, definition) = fixture(fragment);
    analyze_maintainability(&source, &definition, limits, &control()).unwrap()
}
fn excerpt<'a>(source: &'a SourceFile, span: &Span) -> &'a str {
    &source.text[span.start as usize..span.end as usize]
}
const SIMPLE: &str = "fn event() {\n  // ignored\n  let x = 1;\n\n  /* ignored\n     too */\n  if x > 0 {\n    unsafe { call(); }\n  }\n}";

#[test]
fn independent_oracle_counts_source_lines_lexical_tokens_and_nesting() {
    let measured = report(SIMPLE, MaintainabilityLimits::default());
    assert_eq!(
        measured.metrics,
        Some(SyntaxMetrics {
            source_lines: 6,
            lexical_tokens: 24,
            max_nesting: 1,
            unsafe_boundaries: 1
        })
    );
    assert_eq!(measured.envelope.coverage.status, Status::Partial);
    assert!(!measured.envelope.truncated);
    assert!(!measured.locations_truncated);
    let (source, definition) = fixture(SIMPLE);
    assert_eq!(measured.span, definition.span);
    assert_eq!(Some(&measured.body_span), definition.body_span.as_ref());
    assert_eq!(
        measured
            .code_lines
            .iter()
            .map(|span| excerpt(&source, span))
            .collect::<Vec<_>>(),
        [
            "fn event() {",
            "  let x = 1;",
            "  if x > 0 {",
            "    unsafe { call(); }",
            "  }",
            "}"
        ]
    );
    assert_eq!(
        excerpt(&source, &measured.unsafe_sites[0].keyword_span),
        "unsafe"
    );
    assert_eq!(
        excerpt(&source, &measured.unsafe_sites[0].boundary_span),
        "unsafe { call(); }"
    );
    assert!(measured.unsafe_sites[0].enclosing_boundary.is_none());
}

#[test]
fn deliberate_count_and_nesting_mutations_fail_the_independent_oracle() {
    let baseline = report(SIMPLE, MaintainabilityLimits::default());
    let changed = report(
        &SIMPLE.replace("  let x = 1;", "  let x = 1;\n  let y = 2;"),
        MaintainabilityLimits::default(),
    );
    assert_ne!(baseline.input_digest, changed.input_digest);
    let metrics = changed.metrics.unwrap();
    assert_eq!(metrics.source_lines, 7);
    assert_eq!(metrics.lexical_tokens, 29);
    assert_ne!(
        baseline.metrics.as_ref().unwrap().source_lines,
        metrics.source_lines
    );
    let deeper = report(
        &SIMPLE.replace("unsafe { call(); }", "if ready { unsafe { call(); } }"),
        MaintainabilityLimits::default(),
    );
    assert_eq!(deeper.metrics.unwrap().max_nesting, 2);
}

#[test]
fn token_count_uses_lexer_not_combined_cst_punctuation() {
    let measured = report("fn event(){a::b();}", MaintainabilityLimits::default());
    assert_eq!(measured.metrics.unwrap().lexical_tokens, 13);
}

#[test]
fn raw_strings_comments_blank_literal_lines_and_utf8_crlf_stay_distinct() {
    let fragment = "fn event() {\r\n let text = r#\"first\r\n // not a comment\r\n   \r\n /* still literal */\r\n last\"#;\r\n}";
    let (source, definition) = fixture(fragment);
    let measured = analyze_maintainability(
        &source,
        &definition,
        MaintainabilityLimits::default(),
        &control(),
    )
    .unwrap();
    let metrics = measured.metrics.unwrap();
    assert_eq!(
        (
            metrics.source_lines,
            metrics.lexical_tokens,
            metrics.unsafe_boundaries
        ),
        (6, 11, 0)
    );
    assert_eq!(
        excerpt(&source, &measured.code_lines[2]),
        " // not a comment\r"
    );
    assert_eq!(
        excerpt(&source, &measured.code_lines[3]),
        " /* still literal */\r"
    );
    assert!(
        measured
            .code_lines
            .iter()
            .all(|span| source.text.is_char_boundary(span.start as usize)
                && source.text.is_char_boundary(span.end as usize))
    );
    let lexical = report(
        "fn event() { let text = \"unsafe { if value {} }\"; /* unsafe { } */ }",
        MaintainabilityLimits::default(),
    );
    assert_eq!(
        (
            lexical.metrics.as_ref().unwrap().max_nesting,
            lexical.metrics.unwrap().unsafe_boundaries
        ),
        (0, 0)
    );
}

#[test]
fn versioned_nesting_is_lexical_and_else_if_is_nested() {
    let measured = report(
        "fn event() { if a { while b { for x in y { match x { _ => loop { break; } } } } } else if c {} let c = || if d {}; let a = async { if e {} }; fn nested() { if f {} } }",
        MaintainabilityLimits::default(),
    );
    assert_eq!(measured.metrics.unwrap().max_nesting, 5);
    let sites = measured
        .nesting_sites
        .iter()
        .map(|site| (site.kind.as_str(), site.depth))
        .collect::<Vec<_>>();
    assert_eq!(
        sites,
        [
            ("if", 1),
            ("while", 2),
            ("for", 3),
            ("match", 4),
            ("loop", 5),
            ("if", 2),
            ("closure", 1),
            ("if", 2),
            ("async_block", 1),
            ("if", 2),
            ("nested_function", 1),
            ("if", 2)
        ]
    );
    let blocks = report(
        "fn event() { { unsafe { const { if a {} } } } }",
        MaintainabilityLimits::default(),
    );
    assert_eq!(blocks.metrics.unwrap().max_nesting, 1);
}

#[test]
fn unsafe_boundaries_include_exact_keywords_and_lexical_enclosures() {
    let fragment = "unsafe fn event() { let p: unsafe fn() = value; unsafe { unsafe { call(); } } unsafe fn nested() {} }";
    let (source, _) = fixture(fragment);
    let measured = report(fragment, MaintainabilityLimits::default());
    assert_eq!(measured.metrics.unwrap().unsafe_boundaries, 5);
    assert_eq!(
        measured
            .unsafe_sites
            .iter()
            .map(|site| site.kind.as_str())
            .collect::<Vec<_>>(),
        [
            "function",
            "function_pointer_type",
            "block",
            "block",
            "function"
        ]
    );
    for site in &measured.unsafe_sites {
        assert_eq!(excerpt(&source, &site.keyword_span), "unsafe");
    }
    assert!(measured.unsafe_sites[0].enclosing_boundary.is_none());
    assert_eq!(
        measured.unsafe_sites[2].enclosing_boundary.as_ref(),
        Some(&measured.unsafe_sites[0].boundary_span)
    );
    assert_eq!(
        measured.unsafe_sites[3].enclosing_boundary.as_ref(),
        Some(&measured.unsafe_sites[2].boundary_span)
    );
    let attributes = report(
        "#[unsafe(no_mangle)] unsafe fn event() {}",
        MaintainabilityLimits::default(),
    );
    assert_eq!(attributes.metrics.unwrap().unsafe_boundaries, 2);
    assert_eq!(attributes.unsafe_sites[1].kind, "attribute");
}

#[test]
fn macro_spellings_are_counted_but_expansion_metrics_are_unknown() {
    let measured = report(
        "fn event() { macro_rules! make { () => { unsafe { if a {} } } } make!(); }",
        MaintainabilityLimits::default(),
    );
    let metrics = measured.metrics.unwrap();
    assert!(metrics.lexical_tokens > 10);
    assert_eq!((metrics.max_nesting, metrics.unsafe_boundaries), (0, 0));
    assert_eq!(measured.unknowns.len(), 2);
    assert!(
        measured
            .unknowns
            .iter()
            .all(|unknown| unknown.reason.contains("Macro expansion"))
    );
    assert!(
        measured
            .envelope
            .coverage
            .reasons
            .iter()
            .any(|reason| reason.reason == UnknownReason::MacroUnavailable)
    );
    assert!(
        measured
            .envelope
            .assumptions
            .iter()
            .any(|text| text.contains("generated-origin provenance is unknown"))
    );
}

#[test]
fn conditional_attributes_and_unknown_cfg_do_not_become_active_code_claims() {
    let (source, mut definition) = fixture("#[cfg(any())] fn event() { unsafe { call(); } }");
    definition.cfg_status = CfgStatus::Inactive;
    let measured = analyze_maintainability(
        &source,
        &definition,
        MaintainabilityLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(measured.metrics.unwrap().unsafe_boundaries, 1);
    assert_eq!(measured.envelope.coverage.status, Status::Partial);
    assert!(
        measured
            .unknowns
            .iter()
            .any(|unknown| unknown.reason.contains("not active"))
    );
    assert!(
        measured
            .unknowns
            .iter()
            .any(|unknown| unknown.reason.contains("Attribute effects"))
    );
}

#[test]
fn traversal_and_source_budgets_withhold_all_count_values() {
    for limits in [
        MaintainabilityLimits {
            max_source_bytes: 1,
            ..Default::default()
        },
        MaintainabilityLimits {
            max_nodes: 1,
            ..Default::default()
        },
        MaintainabilityLimits {
            max_tokens: 1,
            ..Default::default()
        },
    ] {
        let measured = report(SIMPLE, limits);
        assert!(measured.metrics.is_none());
        assert!(measured.envelope.truncated);
        assert_eq!(measured.envelope.coverage.status, Status::Partial);
    }
}

#[test]
fn location_caps_and_response_caps_preserve_only_completed_counts() {
    let fragment = "fn event() {\n if a { unsafe { call(); } }\n if b { unsafe { call(); } }\n}";
    let full = report(fragment, MaintainabilityLimits::default());
    let capped = report(
        fragment,
        MaintainabilityLimits {
            max_locations: 1,
            ..Default::default()
        },
    );
    assert_eq!(full.metrics, capped.metrics);
    assert!(capped.locations_truncated);
    assert!(!capped.envelope.truncated);
    assert_eq!(
        (
            capped.code_lines.len(),
            capped.nesting_sites.len(),
            capped.unsafe_sites.len()
        ),
        (1, 1, 1)
    );
    let large = format!(
        "fn event() {{\n{}\n}}",
        "if a { unsafe { call(); } }\n".repeat(100)
    );
    let capped = report(
        &large,
        MaintainabilityLimits {
            max_response_bytes: 2400,
            ..Default::default()
        },
    );
    assert!(capped.metrics.is_some());
    assert!(capped.locations_truncated);
    assert!(!capped.envelope.truncated);
    assert!(
        capped.code_lines.is_empty()
            && capped.nesting_sites.is_empty()
            && capped.unsafe_sites.is_empty()
    );
    assert!(serde_json::to_vec(&capped).unwrap().len() <= 2400);
}

#[test]
fn cancellation_and_deadlines_never_publish_partial_counts_as_final() {
    let (source, definition) = fixture(SIMPLE);
    let cancelled = control();
    cancelled.cancel();
    for control in [cancelled, AnalysisControl::new(Duration::ZERO)] {
        let measured = analyze_maintainability(
            &source,
            &definition,
            MaintainabilityLimits::default(),
            &control,
        )
        .unwrap();
        assert!(measured.metrics.is_none());
        assert!(measured.envelope.truncated);
        assert!(measured.envelope.cancelled || measured.envelope.deadline_reached);
    }
}

#[test]
fn malformed_source_spans_and_hard_limits_are_explicit() {
    let malformed = report("fn event() { let = ; }", MaintainabilityLimits::default());
    assert!(malformed.metrics.is_none());
    assert!(!malformed.unknowns.is_empty());
    let (source, definition) = fixture(SIMPLE);
    let mut bad = definition.clone();
    bad.body_span.as_mut().unwrap().start += 1;
    assert!(
        analyze_maintainability(&source, &bad, MaintainabilityLimits::default(), &control())
            .is_err()
    );
    bad = definition.clone();
    bad.file_id = FileId("other".into());
    assert!(
        analyze_maintainability(&source, &bad, MaintainabilityLimits::default(), &control())
            .is_err()
    );
    bad = definition.clone();
    bad.span.end += 10;
    assert!(
        analyze_maintainability(&source, &bad, MaintainabilityLimits::default(), &control())
            .is_err()
    );
    bad = definition.clone();
    bad.signature = "x".repeat(65_536);
    assert_eq!(
        analyze_maintainability(&source, &bad, MaintainabilityLimits::default(), &control())
            .unwrap_err(),
        AnalysisError::BudgetExhausted
    );
    assert!(
        analyze_maintainability(
            &source,
            &definition,
            MaintainabilityLimits {
                max_nodes: 0,
                ..Default::default()
            },
            &control()
        )
        .is_err()
    );
    let (source, definition) = fixture(&format!("fn event() {{{}}}", " ".repeat(1_048_576)));
    assert_eq!(
        analyze_maintainability(
            &source,
            &definition,
            MaintainabilityLimits::default(),
            &control()
        )
        .unwrap_err(),
        AnalysisError::BudgetExhausted
    );
}

#[test]
fn digests_bind_source_definition_context_and_all_budgets() {
    let baseline = report(SIMPLE, MaintainabilityLimits::default());
    assert_eq!(
        serde_json::to_value(&baseline).unwrap(),
        serde_json::to_value(report(SIMPLE, MaintainabilityLimits::default())).unwrap()
    );
    let other = report(
        SIMPLE,
        MaintainabilityLimits {
            max_locations: 10,
            ..Default::default()
        },
    );
    assert_ne!(baseline.input_digest, other.input_digest);
    assert_eq!(baseline.metrics, other.metrics);
    let (source, mut definition) = fixture(SIMPLE);
    definition.context_id = ContextId("other".into());
    let other = analyze_maintainability(
        &source,
        &definition,
        MaintainabilityLimits::default(),
        &control(),
    )
    .unwrap();
    assert_ne!(baseline.input_digest, other.input_digest);
    assert!(atlas_analysis::typescript().contains("export type SourceMaintainability"));
    let mut value = serde_json::to_value(baseline).unwrap();
    value["quality_score"] = 100.into();
    assert!(serde_json::from_value::<SourceMaintainability>(value).is_err());
}

#[test]
fn real_capture_definition_metadata_matches_exact_source_metrics() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='metric_oracle'\nversion='0.1.0'\nedition='2024'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("lib.rs"), SIMPLE).unwrap();
    let (source, context) =
        atlas_ingest::capture(directory.path(), &atlas_ingest::CaptureOptions::default()).unwrap();
    let facts =
        atlas_frontend::analyze(source, context, atlas_frontend::AnalysisLevel::Syntax).unwrap();
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
    let measured = analyze_maintainability(
        file,
        definition,
        MaintainabilityLimits::default(),
        &control(),
    )
    .unwrap();
    assert_eq!(
        measured.metrics,
        report(SIMPLE, MaintainabilityLimits::default()).metrics
    );
}

#[test]
fn long_multibyte_literal_is_one_token_and_preserves_exact_line_span() {
    let fragment = format!(
        "fn event() {{ let text = r#\"{}\"#; }}",
        "\u{96ea}".repeat(6000)
    );
    let (source, _) = fixture(&fragment);
    let measured = report(&fragment, MaintainabilityLimits::default());
    assert_eq!(
        measured.metrics.unwrap(),
        SyntaxMetrics {
            source_lines: 1,
            lexical_tokens: 11,
            max_nesting: 0,
            unsafe_boundaries: 0
        }
    );
    assert_eq!(excerpt(&source, &measured.code_lines[0]), fragment);
}
