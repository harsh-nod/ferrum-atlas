use crate::{BuildContext, SnapshotId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobLevel {
    Syntax,
    #[default]
    Semantic,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    Foreground,
    #[default]
    Workspace,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub profile: String,
    #[serde(default)]
    pub level: JobLevel,
    #[serde(default)]
    pub priority: JobPriority,
    #[serde(default)]
    pub context: Option<BuildContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
}
impl JobStatus {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Queued,
    Capture,
    Syntax,
    Semantics,
    Validation,
    Publication,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct JobEvent {
    pub sequence: u32,
    pub timestamp_ms: String,
    pub stage: JobStage,
    pub status: JobStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct JobRecord {
    pub id: String,
    pub request: JobRequest,
    pub status: JobStatus,
    pub created_ms: String,
    pub snapshot_id: Option<SnapshotId>,
    pub message: Option<String>,
    pub events: Vec<JobEvent>,
}
