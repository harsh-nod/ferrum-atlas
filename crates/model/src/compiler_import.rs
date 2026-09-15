use crate::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompilerDefinitionMapping {
    Matched { definition_id: DefinitionId },
    Unmapped { reason: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerBodyMapping {
    pub body_id: String,
    pub mapping: CompilerDefinitionMapping,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerImport {
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub bundle: CompilerBundle,
    pub mappings: Vec<CompilerBodyMapping>,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CompilerImportSummary {
    pub id: String,
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub compiler: CompilerIdentity,
    pub phase: String,
    pub body_count: u32,
    pub mapped_count: u32,
    pub coverage: Coverage,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CompilerFlowPage {
    pub snapshot_id: SnapshotId,
    pub context_id: ContextId,
    pub import_id: String,
    pub definition_id: DefinitionId,
    pub compiler: CompilerIdentity,
    pub phase: String,
    pub input_manifest_hash: String,
    pub panic_strategy: String,
    pub body: CompilerBody,
    pub offset: u32,
    pub total_blocks: u32,
    pub next_offset: Option<u32>,
    pub coverage: Coverage,
}
