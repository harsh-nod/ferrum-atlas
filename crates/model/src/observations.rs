use crate::{ContextId, DefinitionId, SnapshotId, SourceId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Pass,
    Fail,
    ExpectedFail,
    UnexpectedPass,
    Timeout,
    InfrastructureError,
    NotRun,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ArtifactIdentity {
    pub sha256: String,
    pub source_id: SourceId,
    pub context_id: ContextId,
    pub producer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TestObservation {
    pub name: String,
    pub outcome: TestOutcome,
    pub elapsed_ns: Option<String>,
    pub timeout_ns: Option<String>,
    pub reason: Option<String>,
    pub definition_ids: Vec<DefinitionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TraceEvent {
    pub sequence: String,
    pub timestamp: String,
    pub kind: String,
    pub definition_id: Option<DefinitionId>,
    pub correlation_id: Option<String>,
    pub loss_count: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TraceStream {
    pub id: String,
    pub process_or_device: String,
    pub thread_or_hart: String,
    pub clock_domain: String,
    pub timestamp_unit: String,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ObservationBundle {
    pub schema_version: u32,
    pub snapshot_id: SnapshotId,
    pub artifact: ArtifactIdentity,
    pub tests: Vec<TestObservation>,
    pub streams: Vec<TraceStream>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ObservationSummary {
    pub id: String,
    pub snapshot_id: SnapshotId,
    pub artifact: ArtifactIdentity,
    pub test_count: u32,
    pub event_count: u32,
    pub clock_domains: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ObservationWindow {
    pub summary: ObservationSummary,
    pub tests: Vec<TestObservation>,
    pub streams: Vec<TraceStream>,
    pub offset: u32,
    pub next_offset: Option<u32>,
    pub truncated: bool,
}
