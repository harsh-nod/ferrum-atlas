//! Machine contract for imported, explicitly trusted compiler extraction.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const COMPILER_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerBundle {
    pub schema_version: u32,
    pub compiler: CompilerIdentity,
    pub inputs: CompilerInputs,
    pub phase: String,
    pub bodies: Vec<CompilerBody>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerIdentity {
    pub adapter: String,
    pub adapter_version: String,
    pub release: String,
    pub commit_hash: String,
    pub commit_date: String,
    pub host: String,
    pub llvm_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerInputs {
    pub manifest_hash: String,
    pub files: Vec<CompilerSourceFile>,
    pub crate_name: String,
    pub crate_root: String,
    pub edition: String,
    pub target: String,
    pub panic_strategy: String,
    pub mir_opt_level: u32,
    pub rustc_args: Vec<String>,
    pub environment_policy: String,
    pub trust: String,
    pub compiled_artifact: Option<CompilerArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerArtifact {
    pub kind: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerSourceFile {
    pub path: String,
    pub sha256: String,
    pub byte_length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompilerSourceMapping {
    Exact {
        path: String,
        start_byte: u32,
        end_byte: u32,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerBody {
    /// Input-manifest-local identity, never a source DefinitionId.
    pub body_id: String,
    pub def_path: String,
    pub kind: String,
    pub span: CompilerSourceMapping,
    pub argument_count: u32,
    pub locals: Vec<CompilerLocal>,
    pub source_scopes: Vec<CompilerSourceScope>,
    pub blocks: Vec<CompilerBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerLocal {
    pub index: u32,
    pub role: String,
    pub names: Vec<String>,
    /// Display-only compiler type spelling; consumers must not parse it.
    pub type_display: String,
    pub source_scope: u32,
    pub span: CompilerSourceMapping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerSourceScope {
    pub index: u32,
    pub parent: Option<u32>,
    pub span: CompilerSourceMapping,
    pub inlined_def_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerBlock {
    pub index: u32,
    pub is_cleanup: bool,
    pub statements: Vec<CompilerStatement>,
    pub terminator: CompilerTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerStatement {
    pub index: u32,
    pub kind: String,
    pub source_scope: u32,
    pub span: CompilerSourceMapping,
    pub locals: CompilerLocalEffects,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerLocalEffects {
    /// Whole-local assignments only, not writes through aliases or projections.
    pub defs: Vec<u32>,
    /// Conservative local references, including address-taking and projections.
    pub uses: Vec<u32>,
    pub moves: Vec<u32>,
    pub storage_live: Vec<u32>,
    pub storage_dead: Vec<u32>,
    pub unknown_effects: Vec<CompilerUnknownEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CompilerUnknownEffect {
    Call,
    IndirectCall,
    PointerAliasing,
    BorrowAliasing,
    PartialWrite,
    Drop,
    InlineAssembly,
    Intrinsic,
    Retag,
    ExternalState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerTerminator {
    pub kind: String,
    pub source_scope: u32,
    pub span: CompilerSourceMapping,
    pub locals: CompilerLocalEffects,
    /// Call destinations become initialized only on the normal-return edge.
    pub normal_return_defs: Vec<u32>,
    pub successors: Vec<CompilerSuccessor>,
    pub unwind: Option<CompilerUnwind>,
    pub call_target: Option<CompilerCallTarget>,
    pub assert_expected: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerSuccessor {
    pub target: u32,
    pub kind: CompilerEdgeKind,
    /// Unsigned MIR bit pattern, as decimal text to preserve all 128 bits.
    pub switch_value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CompilerEdgeKind {
    Normal,
    SwitchValue,
    Otherwise,
    Unwind,
    Resume,
    CoroutineDrop,
    Imaginary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompilerUnwind {
    Continue,
    Unreachable,
    Terminate { reason: String },
    Cleanup { target: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompilerCallTarget {
    /// A compiler FnDef identity, not a fully monomorphized runtime target.
    FunctionDefinition {
        def_path: String,
        is_local: bool,
    },
    Indirect {
        reason: String,
    },
}
