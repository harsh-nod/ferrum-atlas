use atlas_analysis::*;
use atlas_model::*;
use std::time::Duration;

fn event(index: u64, kind: &str, anchor: Option<&str>) -> TraceEvent {
    TraceEvent {
        sequence: (9_007_199_254_740_992 + index).to_string(),
        timestamp: (18_014_398_509_481_984 + index * 7).to_string(),
        kind: kind.into(),
        definition_id: None,
        correlation_id: anchor.map(String::from),
        loss_count: None,
    }
}
fn bundle(events: Vec<TraceEvent>) -> ObservationBundle {
    ObservationBundle {
        schema_version: SCHEMA_VERSION,
        snapshot_id: SnapshotId("snapshot".into()),
        artifact: ArtifactIdentity {
            sha256: "a".repeat(64),
            source_id: SourceId("source".into()),
            context_id: ContextId("context".into()),
            producer: "fixture".into(),
        },
        tests: vec![],
        streams: vec![TraceStream {
            id: "stream".into(),
            process_or_device: "cpu".into(),
            thread_or_hart: "0".into(),
            clock_domain: "clock-a".into(),
            timestamp_unit: "cycles".into(),
            events,
        }],
        limitations: vec![],
    }
}
fn request() -> TraceCompareRequest {
    TraceCompareRequest {
        before_stream_id: "stream".into(),
        after_stream_id: "stream".into(),
        before_offset: 0,
        after_offset: 0,
        max_events: 200,
        max_anchors: 200,
    }
}
fn control() -> AnalysisControl {
    AnalysisControl::new(Duration::from_secs(5))
}
fn compare(before: &ObservationBundle, after: &ObservationBundle) -> TraceComparison {
    compare_trace_windows(before, after, &request(), &control()).unwrap()
}

#[test]
fn aligns_unique_semantic_anchor_and_reports_first_observed_divergence() {
    let before = bundle(vec![
        event(0, "prefix", None),
        event(1, "checkpoint", Some("packet:17")),
        event(2, "queue_switch", None),
        event(3, "exit", None),
    ]);
    let after = bundle(vec![
        event(0, "checkpoint", Some("packet:17")),
        event(1, "external_effect", None),
        event(2, "exit", None),
    ]);
    let report = compare(&before, &after);
    assert_eq!(report.anchor.as_deref(), Some("packet:17"));
    assert_eq!(report.matching_prefix_events, 1);
    let divergence = report.first_observed_divergence.unwrap();
    assert_eq!(divergence.reason, "event_kind");
    assert_eq!(divergence.before.unwrap().sequence, "9007199254740994");
    assert_eq!(report.envelope.coverage.status, Status::Partial);
}

#[test]
fn wide_integer_timing_stays_per_stream_and_artifacts_stay_separate() {
    let before = bundle(vec![
        event(0, "checkpoint", Some("start")),
        event(1, "exit", None),
    ]);
    let mut after = before.clone();
    after.artifact.sha256 = "b".repeat(64);
    after.streams[0].clock_domain = "device-clock".into();
    after.streams[0].timestamp_unit = "device_cycles".into();
    after.streams[0].events[1].timestamp = "18014398509482054".into();
    let report = compare(&before, &after);
    assert_eq!(report.before_interval.as_deref(), Some("7"));
    assert_eq!(report.after_interval.as_deref(), Some("70"));
    assert_ne!(report.before_artifact.sha256, report.after_artifact.sha256);
    assert_ne!(report.before_clock_domain, report.after_clock_domain);
    assert!(report.first_observed_divergence.is_none());
    assert_eq!(report.matching_prefix_events, 2);
}

#[test]
fn repeated_or_missing_anchors_do_not_manufacture_alignment() {
    let repeated = bundle(vec![
        event(0, "checkpoint", Some("repeat")),
        event(1, "checkpoint", Some("repeat")),
    ]);
    let report = compare(&repeated, &repeated);
    assert!(report.anchor.is_none());
    assert_eq!(report.ambiguous_anchors, ["repeat"]);
    assert_eq!(report.compared_events, 0);
    assert!(report.first_observed_divergence.is_none());
    let no_anchor = bundle(vec![event(0, "entry", None)]);
    assert!(compare(&no_anchor, &no_anchor).anchor.is_none());
}

#[test]
fn anchor_order_inversion_is_an_observation_not_reordered_away() {
    let before = bundle(vec![
        event(0, "checkpoint", Some("start")),
        event(1, "checkpoint", Some("a")),
        event(2, "checkpoint", Some("b")),
    ]);
    let after = bundle(vec![
        event(0, "checkpoint", Some("start")),
        event(1, "checkpoint", Some("b")),
        event(2, "checkpoint", Some("a")),
    ]);
    let report = compare(&before, &after);
    assert_eq!(
        report.first_observed_divergence.unwrap().reason,
        "semantic_anchor"
    );
}

#[test]
fn loss_and_window_bounds_remain_explicit() {
    let mut before = bundle(vec![
        event(0, "checkpoint", Some("start")),
        event(1, "exit", None),
    ]);
    before.streams[0].events[0].loss_count = Some(u64::MAX.to_string());
    before.streams[0].events[1].loss_count = Some(u64::MAX.to_string());
    let report = compare(&before, &before);
    assert_eq!(report.before_loss_count, "36893488147419103230");
    assert!(
        report
            .envelope
            .coverage
            .limitations
            .iter()
            .any(|text| text.contains("explicit event loss"))
    );
    let mut request = request();
    request.max_events = 1;
    let bounded = compare_trace_windows(&before, &before, &request, &control()).unwrap();
    assert!(bounded.envelope.truncated);
    assert_eq!(bounded.matching_prefix_events, 1);
}

#[test]
fn event_count_divergence_is_only_about_selected_windows() {
    let before = bundle(vec![
        event(0, "checkpoint", Some("start")),
        event(1, "exit", None),
    ]);
    let after = bundle(vec![event(0, "checkpoint", Some("start"))]);
    let report = compare(&before, &after);
    let divergence = report.first_observed_divergence.unwrap();
    assert_eq!(divergence.reason, "selected_window_event_count");
    assert!(divergence.after.is_none());
    assert!(
        report
            .envelope
            .coverage
            .limitations
            .iter()
            .any(|text| text.contains("instrumentation"))
    );
}

#[test]
fn nonmonotonic_timestamps_are_not_negative_durations() {
    let before = bundle(vec![
        event(0, "checkpoint", Some("start")),
        event(1, "exit", None),
    ]);
    let mut after = before.clone();
    after.streams[0].events[1].timestamp = "0".into();
    let report = compare(&before, &after);
    assert_eq!(report.before_interval.as_deref(), Some("7"));
    assert!(report.after_interval.is_none());
}

#[test]
fn rejects_non_increasing_sequences_and_invalid_decimal_strings() {
    for invalid in ["9007199254740992", "-1", "18446744073709551616", "1e9", ""] {
        let mut input = bundle(vec![
            event(0, "checkpoint", Some("start")),
            event(1, "exit", None),
        ]);
        input.streams[0].events[1].sequence = invalid.into();
        assert!(matches!(
            compare_trace_windows(&input, &input, &request(), &control()),
            Err(AnalysisError::InvalidInput(_))
        ));
    }
}

#[test]
fn selected_stream_identity_limits_and_anchor_budget_are_enforced() {
    let before = bundle(vec![
        event(0, "checkpoint", Some("a")),
        event(1, "checkpoint", Some("b")),
    ]);
    let mut duplicate = before.clone();
    duplicate.streams.push(duplicate.streams[0].clone());
    assert!(compare_trace_windows(&duplicate, &before, &request(), &control()).is_err());
    let mut request = request();
    request.max_anchors = 1;
    let report = compare_trace_windows(&before, &before, &request, &control()).unwrap();
    assert!(report.envelope.truncated);
    assert!(report.anchor.is_none());
    request.max_events = 0;
    assert!(compare_trace_windows(&before, &before, &request, &control()).is_err());
}

#[test]
fn cancelled_trace_comparison_never_asserts_divergence_or_equality() {
    let input = bundle(vec![event(0, "checkpoint", Some("start"))]);
    let control = control();
    control.cancel();
    let report = compare_trace_windows(&input, &input, &request(), &control).unwrap();
    assert!(report.envelope.cancelled);
    assert!(report.envelope.truncated);
    assert_eq!(report.compared_events, 0);
    assert!(report.first_observed_divergence.is_none());
}
