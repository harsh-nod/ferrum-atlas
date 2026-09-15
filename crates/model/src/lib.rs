//! Versioned producer-independent facts and public transport contracts.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use ts_rs::TS;

mod observations;
pub use observations::*;
mod jobs;
pub use jobs::*;
mod analysis;
pub use analysis::*;
mod compiler;
pub use compiler::*;
mod compiler_import;
pub use compiler_import::*;

pub const SCHEMA_VERSION: u32 = 1;
pub const API_VERSION: &str = "1";

pub fn digest<T: Serialize + ?Sized>(domain: &str, value: &T) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ferrum-atlas:1\0");
    hash.update(domain.as_bytes());
    hash.update([0]);
    hash.update(serde_json::to_vec(value).expect("canonical model serializes"));
    format!("{domain}:{:x}", hash.finalize())
}

macro_rules! id_type {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
        }
        impl From<String> for $name { fn from(s: String) -> Self { Self(s) } }
        impl AsRef<str> for $name { fn as_ref(&self) -> &str { &self.0 } }
    )+};
}
id_type!(
    RepositoryId,
    SourceId,
    SnapshotId,
    ContextId,
    FileId,
    DefinitionId,
    RelationId,
    EvidenceId
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Complete,
    Partial,
    Failed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum UnknownReason {
    MissingDependency,
    CfgUnknown,
    MacroUnavailable,
    IndirectTargetUnknown,
    ExternalBoundary,
    UnsupportedConstruct,
    BudgetExhausted,
    AnalysisFailed,
    ArtifactMismatch,
    SyntaxOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ReasonCount {
    pub reason: UnknownReason,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Coverage {
    pub status: Status,
    pub reasons: Vec<ReasonCount>,
    pub limitations: Vec<String>,
}
impl Coverage {
    pub fn complete() -> Self {
        Self {
            status: Status::Complete,
            reasons: vec![],
            limitations: vec![],
        }
    }
    pub fn partial(reason: UnknownReason, limitation: impl Into<String>) -> Self {
        Self {
            status: Status::Partial,
            reasons: vec![ReasonCount { reason, count: 1 }],
            limitations: vec![limitation.into()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SourceFile {
    pub id: FileId,
    pub path: String,
    pub content_hash: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SourceSnapshot {
    pub id: SourceId,
    pub repository_id: RepositoryId,
    pub revision: String,
    pub files: Vec<SourceFile>,
    pub manifests: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CrateInput {
    pub name: String,
    pub root_file: String,
    pub edition: String,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BuildContext {
    pub id: ContextId,
    pub name: String,
    pub target: String,
    pub features: Vec<String>,
    pub default_features: bool,
    pub cfg: BTreeMap<String, Option<String>>,
    pub crates: Vec<CrateInput>,
    pub manifest_digest: String,
    pub trust: String,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Span {
    pub file_id: FileId,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Metrics {
    pub lines: u32,
    pub branches: u32,
    pub returns: u32,
    pub awaits: u32,
    pub unsafe_blocks: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CfgStatus {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Definition {
    pub id: DefinitionId,
    pub context_id: ContextId,
    pub file_id: FileId,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub parent_id: Option<DefinitionId>,
    pub signature: String,
    pub signature_hash: String,
    pub body_hash: String,
    pub span: Span,
    pub body_span: Option<Span>,
    pub cfg: Vec<String>,
    pub cfg_status: CfgStatus,
    pub visibility: String,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Resolved {
        id: DefinitionId,
    },
    Unknown {
        reason: UnknownReason,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Relation {
    pub id: RelationId,
    pub source: DefinitionId,
    pub target: Target,
    pub kind: String,
    pub span: Span,
    pub evidence_id: EvidenceId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Evidence {
    pub id: EvidenceId,
    pub basis: String,
    pub producer: String,
    pub inputs: Vec<String>,
    pub assumptions: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Diagnostic {
    pub path: Option<String>,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FlowPoint {
    pub kind: String,
    pub label: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FunctionFlow {
    pub definition_id: DefinitionId,
    pub phase: String,
    pub points: Vec<FlowPoint>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FactBatch {
    pub schema_version: u32,
    pub source: SourceSnapshot,
    pub context: BuildContext,
    pub producer: String,
    pub definitions: Vec<Definition>,
    pub relations: Vec<Relation>,
    pub evidence: Vec<Evidence>,
    pub flows: Vec<FunctionFlow>,
    pub coverage: Coverage,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub source_id: SourceId,
    pub repository_id: RepositoryId,
    pub context: BuildContext,
    pub revision: String,
    pub created_at: String,
    pub file_count: u32,
    pub definition_count: u32,
    pub relation_count: u32,
    pub coverage: Coverage,
    pub producer: String,
    pub fact_digest: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Page {
    pub truncated: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Work {
    pub deadline_reached: bool,
    pub elapsed_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct QueryResponse<T> {
    pub api_version: String,
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub items: Vec<T>,
    pub coverage: Coverage,
    pub page: Page,
    pub work: Work,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GraphRequest {
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub definition_id: DefinitionId,
    pub direction: Direction,
    pub depth: u32,
    pub max_nodes: u32,
    pub max_edges: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GraphResponse {
    pub api_version: String,
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub nodes: Vec<Definition>,
    pub edges: Vec<Relation>,
    pub coverage: Coverage,
    pub page: Page,
    pub work: Work,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceWindow {
    pub snapshot_id: SnapshotId,
    pub file_id: FileId,
    pub path: String,
    pub content_hash: String,
    pub text: String,
    pub start_line: u32,
    pub total_lines: u32,
    pub start_byte: u32,
    pub truncated: bool,
    pub encoding: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DefinitionDetail {
    pub snapshot_id: SnapshotId,
    pub definition: Definition,
    pub evidence: Vec<Evidence>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DiffRequest {
    pub before: SnapshotId,
    pub after: SnapshotId,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DefinitionChange {
    pub kind: String,
    pub before: Option<Definition>,
    pub after: Option<Definition>,
    pub correspondence: String,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DiffResponse {
    pub before: SnapshotId,
    pub after: SnapshotId,
    pub context_changed: bool,
    pub changes: Vec<DefinitionChange>,
    pub impact_candidates: Vec<Definition>,
    pub coverage: Coverage,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Capabilities {
    pub api_version: String,
    pub schema_version: u32,
    pub analysis_levels: Vec<String>,
    pub graph_families: Vec<String>,
    pub trust_modes: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub correlation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SnapshotPinRequest {
    pub context_id: ContextId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SnapshotPinState {
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub name: String,
    pub retained: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SnapshotPreparation {
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub ready: bool,
}

pub fn typescript() -> String {
    let mut output = String::from("// Generated by cargo run -p xtask -- types. Do not edit.\n");
    macro_rules! emit { ($($t:ty),+ $(,)?) => { $(output.push_str("export "); output.push_str(&<$t>::decl()); output.push('\n');)+ }; }
    emit!(
        RepositoryId,
        SourceId,
        SnapshotId,
        ContextId,
        FileId,
        DefinitionId,
        RelationId,
        EvidenceId,
        Status,
        UnknownReason,
        ReasonCount,
        Coverage,
        SourceFile,
        SourceSnapshot,
        CrateInput,
        BuildContext,
        Span,
        CfgStatus,
        Metrics,
        Definition,
        Target,
        Relation,
        Evidence,
        Diagnostic,
        FlowPoint,
        FunctionFlow,
        FactBatch,
        Snapshot,
        Page,
        Work,
        QueryResponse<Definition>,
        Direction,
        GraphRequest,
        GraphResponse,
        SourceWindow,
        DefinitionDetail,
        DiffRequest,
        DefinitionChange,
        DiffResponse,
        Capabilities,
        ApiError,
        SnapshotPinRequest,
        SnapshotPinState,
        SnapshotPreparation
    );
    emit!(
        TestOutcome,
        ArtifactIdentity,
        TestObservation,
        TraceEvent,
        TraceStream,
        ObservationBundle,
        ObservationSummary,
        ObservationWindow
    );
    emit!(
        JobLevel,
        JobPriority,
        JobRequest,
        JobStatus,
        JobStage,
        JobEvent,
        JobRecord
    );
    emit!(
        AnalysisResponse<Definition>,
        PathRequest,
        TraceAlignmentRequest,
        TraceAlignmentResponse<Definition>
    );
    emit!(
        CompilerBundle,
        CompilerIdentity,
        CompilerInputs,
        CompilerArtifact,
        CompilerSourceFile,
        CompilerSourceMapping,
        CompilerBody,
        CompilerLocal,
        CompilerSourceScope,
        CompilerBlock,
        CompilerStatement,
        CompilerLocalEffects,
        CompilerUnknownEffect,
        CompilerTerminator,
        CompilerSuccessor,
        CompilerEdgeKind,
        CompilerUnwind,
        CompilerCallTarget,
        CompilerDefinitionMapping,
        CompilerBodyMapping,
        CompilerImport,
        CompilerImportSummary,
        CompilerFlowPage
    );
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_domains_and_ordered_maps_are_stable() {
        let a = BTreeMap::from([("b", 2), ("a", 1)]);
        let b = BTreeMap::from([("a", 1), ("b", 2)]);
        assert_eq!(digest("source", &a), digest("source", &b));
        assert_ne!(digest("source", &a), digest("context", &a));
    }
    #[test]
    fn unknown_targets_are_explicit_and_round_trip() {
        let target = Target::Unknown {
            reason: UnknownReason::IndirectTargetUnknown,
            label: "callback".into(),
        };
        let json = serde_json::to_string(&target).unwrap();
        assert!(json.contains("\"kind\":\"unknown\""));
        assert_eq!(serde_json::from_str::<Target>(&json).unwrap(), target);
        assert!(serde_json::from_str::<Target>("{\"kind\":\"resolved\"}").is_err());
    }
}
