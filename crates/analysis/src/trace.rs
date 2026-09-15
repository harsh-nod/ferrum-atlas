use crate::{AnalysisControl, AnalysisEnvelope, AnalysisError, Result, byte_budget};
use atlas_model::{
    ArtifactIdentity, Coverage, ObservationBundle, TraceEvent, TraceStream, UnknownReason,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TraceCompareRequest {
    pub before_stream_id: String,
    pub after_stream_id: String,
    pub before_offset: u32,
    pub after_offset: u32,
    pub max_events: u32,
    pub max_anchors: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TracePosition {
    pub stream_id: String,
    pub sequence: String,
    pub timestamp: String,
    pub clock_domain: String,
    pub timestamp_unit: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TraceDivergence {
    pub reason: String,
    pub before: Option<TracePosition>,
    pub after: Option<TracePosition>,
    pub before_kind: Option<String>,
    pub after_kind: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TraceComparison {
    pub envelope: AnalysisEnvelope,
    pub before_artifact: ArtifactIdentity,
    pub after_artifact: ArtifactIdentity,
    pub anchor: Option<String>,
    pub compared_events: u32,
    pub matching_prefix_events: u32,
    pub ambiguous_anchors: Vec<String>,
    pub first_observed_divergence: Option<TraceDivergence>,
    pub before_loss_count: String,
    pub after_loss_count: String,
    pub before_interval: Option<String>,
    pub after_interval: Option<String>,
    pub before_clock_domain: String,
    pub after_clock_domain: String,
    pub before_timestamp_unit: String,
    pub after_timestamp_unit: String,
}

fn selected_stream<'a>(bundle: &'a ObservationBundle, id: &str) -> Result<&'a TraceStream> {
    if bundle.streams.len() > 64
        || bundle
            .streams
            .iter()
            .map(|stream| stream.events.len())
            .sum::<usize>()
            > 10_000
    {
        return Err(AnalysisError::BudgetExhausted);
    }
    let streams = bundle
        .streams
        .iter()
        .filter(|stream| stream.id == id)
        .collect::<Vec<_>>();
    if streams.len() != 1 {
        return Err(AnalysisError::InvalidInput(
            "selected stream must exist exactly once".into(),
        ));
    }
    let stream = streams[0];
    let mut sequence = None;
    for event in &stream.events {
        let value = decimal(&event.sequence)?;
        decimal(&event.timestamp)?;
        if let Some(loss) = &event.loss_count {
            decimal(loss)?;
        }
        if sequence.is_some_and(|previous| value <= previous) {
            return Err(AnalysisError::InvalidInput(
                "stream sequence must increase strictly".into(),
            ));
        }
        sequence = Some(value);
    }
    Ok(stream)
}

fn decimal(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AnalysisError::InvalidInput(
            "trace integers must be decimal u64 strings".into(),
        ));
    }
    value
        .parse()
        .map_err(|_| AnalysisError::InvalidInput("trace integer exceeds u64".into()))
}

fn window(stream: &TraceStream, offset: u32, limit: u32) -> Result<&[TraceEvent]> {
    let start = offset as usize;
    if start > stream.events.len() {
        return Err(AnalysisError::InvalidInput(
            "trace offset exceeds selected stream".into(),
        ));
    }
    Ok(&stream.events[start..stream.events.len().min(start + limit as usize)])
}

fn anchors(events: &[TraceEvent], control: &AnalysisControl) -> BTreeMap<String, Vec<usize>> {
    let mut map = BTreeMap::<String, Vec<usize>>::new();
    for (index, event) in events.iter().enumerate() {
        if control.stopped() {
            break;
        }
        if let Some(anchor) = &event.correlation_id
            && !anchor.is_empty()
        {
            map.entry(anchor.clone()).or_default().push(index);
        }
    }
    map
}

fn position(stream: &TraceStream, event: &TraceEvent) -> TracePosition {
    TracePosition {
        stream_id: stream.id.clone(),
        sequence: event.sequence.clone(),
        timestamp: event.timestamp.clone(),
        clock_domain: stream.clock_domain.clone(),
        timestamp_unit: stream.timestamp_unit.clone(),
    }
}

/// Aligns one explicitly selected stream pair by a unique producer-supplied correlation anchor.
/// It compares event kind and correlation IDs, never definition IDs across artifacts or unrelated clocks.
pub fn compare_trace_windows(
    before: &ObservationBundle,
    after: &ObservationBundle,
    request: &TraceCompareRequest,
    control: &AnalysisControl,
) -> Result<TraceComparison> {
    if !(1..=2_000).contains(&request.max_events)
        || !(1..=2_000).contains(&request.max_anchors)
        || request.before_stream_id.len() > 256
        || request.after_stream_id.len() > 256
    {
        return Err(AnalysisError::InvalidInput(
            "trace windows require 1..2000 events/anchors and stream IDs <=256 bytes".into(),
        ));
    }
    byte_budget(&(before, after), 16 * 1024 * 1024)?;
    let before_stream = selected_stream(before, &request.before_stream_id)?;
    let after_stream = selected_stream(after, &request.after_stream_id)?;
    let before_events = window(before_stream, request.before_offset, request.max_events)?;
    let after_events = window(after_stream, request.after_offset, request.max_events)?;
    let mut envelope =
        AnalysisEnvelope::new("stream-correlation-anchor-prefix-v1", Coverage::complete());
    envelope.assumptions = vec![
        "Only the explicitly selected per-stream windows are compared; timestamps never impose a cross-stream total order".into(),
        "A unique correlation ID is a producer-supplied semantic anchor, not independently verified workload or test-contract compatibility".into(),
        "Equality compares event kind and correlation ID only; payload, source semantics, device state and run inputs may differ".into(),
        "The first observed divergence under this alignment rule is not proof of root cause".into(),
        "Artifact identities remain separate; no historical result reuse or original-pass/Rust-fail classification is inferred".into(),
        "Intervals use each stream's own unit and clock domain, never CPU-time conversion or cross-clock subtraction".into(),
    ];
    envelope.partial(UnknownReason::UnsupportedConstruct, "Trace comparison is an observation-window interpretation, not proof of behavioral equivalence; different work, instrumentation, buffering, sampling or event loss can change event counts");
    for limitation in before.limitations.iter().chain(&after.limitations) {
        if !envelope.coverage.limitations.contains(limitation) {
            envelope.coverage.limitations.push(limitation.clone());
        }
    }
    if request.before_offset != 0
        || request.after_offset != 0
        || before_events.len() + (request.before_offset as usize) < before_stream.events.len()
        || after_events.len() + (request.after_offset as usize) < after_stream.events.len()
    {
        envelope.truncated = true;
        envelope.partial(
            UnknownReason::BudgetExhausted,
            "Only bounded trace windows were selected; events outside them were not compared",
        );
    }
    let before_map = anchors(before_events, control);
    let after_map = anchors(after_events, control);
    let ambiguous = before_map
        .keys()
        .chain(after_map.keys())
        .filter(|key| {
            before_map.get(*key).is_some_and(|list| list.len() > 1)
                || after_map.get(*key).is_some_and(|list| list.len() > 1)
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut unique = before_map
        .iter()
        .filter_map(|(key, list)| {
            let other = after_map.get(key)?;
            (list.len() == 1 && other.len() == 1).then_some((list[0], other[0], key.clone()))
        })
        .collect::<Vec<_>>();
    unique.sort();
    if before_map.len() > request.max_anchors as usize
        || after_map.len() > request.max_anchors as usize
    {
        envelope.truncated = true;
        envelope.partial(
            UnknownReason::BudgetExhausted,
            "Semantic-anchor budget exhausted; no alignment is asserted",
        );
        unique.clear();
    }
    if !ambiguous.is_empty() {
        envelope.partial(UnknownReason::UnsupportedConstruct, "Repeated correlation IDs are ambiguous within a selected window and are not used as alignment anchors");
    }
    if control.stopped() {
        envelope.stop(control);
        unique.clear();
    }
    let loss = |events: &[TraceEvent]| -> Result<String> {
        Ok(events
            .iter()
            .try_fold(0u128, |sum, event| -> Result<u128> {
                Ok(sum
                    + event
                        .loss_count
                        .as_deref()
                        .map(decimal)
                        .transpose()?
                        .unwrap_or(0) as u128)
            })?
            .to_string())
    };
    let before_loss_count = loss(before_events)?;
    let after_loss_count = loss(after_events)?;
    if before_loss_count != "0" || after_loss_count != "0" {
        envelope.partial(UnknownReason::UnsupportedConstruct, "Selected windows contain explicit event loss; missing observations cannot establish absent execution");
    }
    let mut report = TraceComparison {
        envelope,
        before_artifact: before.artifact.clone(),
        after_artifact: after.artifact.clone(),
        anchor: None,
        compared_events: 0,
        matching_prefix_events: 0,
        ambiguous_anchors: ambiguous
            .into_iter()
            .take(request.max_anchors as usize)
            .collect(),
        first_observed_divergence: None,
        before_loss_count,
        after_loss_count,
        before_interval: None,
        after_interval: None,
        before_clock_domain: before_stream.clock_domain.clone(),
        after_clock_domain: after_stream.clock_domain.clone(),
        before_timestamp_unit: before_stream.timestamp_unit.clone(),
        after_timestamp_unit: after_stream.timestamp_unit.clone(),
    };
    if let Some((before_start, after_start, anchor)) = unique.first() {
        report.anchor = Some(anchor.clone());
        if *before_start != 0 || *after_start != 0 {
            report.envelope.partial(
                UnknownReason::UnsupportedConstruct,
                "Events before the first common unique anchor are unaligned and were not compared",
            );
        }
        let left = &before_events[*before_start..];
        let right = &after_events[*after_start..];
        for index in 0..left.len().max(right.len()) {
            if control.stopped() {
                report.envelope.stop(control);
                break;
            }
            let a = left.get(index);
            let b = right.get(index);
            report.compared_events += 1;
            let reason = match (a, b) {
                (Some(a), Some(b)) if a.kind != b.kind => Some("event_kind"),
                (Some(a), Some(b)) if a.correlation_id != b.correlation_id => {
                    Some("semantic_anchor")
                }
                (Some(_), Some(_)) => None,
                _ => Some("selected_window_event_count"),
            };
            if let Some(reason) = reason {
                report.first_observed_divergence = Some(TraceDivergence {
                    reason: reason.into(),
                    before: a.map(|event| position(before_stream, event)),
                    after: b.map(|event| position(after_stream, event)),
                    before_kind: a.map(|event| event.kind.clone()),
                    after_kind: b.map(|event| event.kind.clone()),
                });
                break;
            }
            report.matching_prefix_events += 1;
        }
        let count = report.matching_prefix_events as usize;
        if count > 0 {
            report.before_interval = interval(&left[..count])?;
            report.after_interval = interval(&right[..count])?;
            if report.before_interval.is_none() || report.after_interval.is_none() {
                report.envelope.partial(UnknownReason::UnsupportedConstruct, "Nonmonotonic timestamps prevent an elapsed interval in at least one selected stream");
            }
        }
    } else {
        report.envelope.partial(UnknownReason::UnsupportedConstruct, "No usable unique common semantic anchor exists in the selected windows; no divergence or equivalence is asserted");
    }
    byte_budget(&report, 2 * 1024 * 1024)?;
    Ok(report)
}

fn interval(events: &[TraceEvent]) -> Result<Option<String>> {
    let values = events
        .iter()
        .map(|event| decimal(&event.timestamp))
        .collect::<Result<Vec<_>>>()?;
    if values.windows(2).any(|pair| pair[1] < pair[0]) {
        return Ok(None);
    }
    Ok(values
        .last()
        .zip(values.first())
        .map(|(last, first)| (last - first).to_string()))
}
