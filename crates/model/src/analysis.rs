use crate::{ContextId, DefinitionId, GraphRequest, SnapshotId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnalysisResponse<T> {
    pub api_version: String,
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub analysis: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PathRequest {
    pub graph: GraphRequest,
    pub target: Option<DefinitionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TraceAlignmentRequest {
    pub before_snapshot_id: SnapshotId,
    pub before_context_id: ContextId,
    pub before_observation_id: String,
    pub after_snapshot_id: SnapshotId,
    pub after_context_id: ContextId,
    pub after_observation_id: String,
    pub before_stream_id: String,
    pub after_stream_id: String,
    pub before_offset: u32,
    pub after_offset: u32,
    pub max_events: u32,
    pub max_anchors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TraceAlignmentResponse<T> {
    pub before: SnapshotId,
    pub after: SnapshotId,
    pub before_context: ContextId,
    pub after_context: ContextId,
    pub comparison: T,
}
