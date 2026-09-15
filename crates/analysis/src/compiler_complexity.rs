//! Structural cyclomatic measure for one complete imported compiler CFG.
use crate::{AnalysisControl, AnalysisEnvelope, AnalysisError, Result, byte_budget};
use atlas_model::{
    CompilerBlock, CompilerEdgeKind as Edge, CompilerFlowPage, CompilerIdentity,
    CompilerSourceMapping, CompilerTerminator, CompilerUnwind, DefinitionId, Status, UnknownReason,
    digest,
};
use petgraph::{graph::DiGraph, visit::Dfs};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

const VERSION: &str = "compiler-cyclomatic-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerComplexityLimits {
    pub max_blocks: u32,
    pub max_edges: u32,
    pub max_response_bytes: u32,
}
impl Default for CompilerComplexityLimits {
    fn default() -> Self {
        Self {
            max_blocks: 200,
            max_edges: 2000,
            max_response_bytes: 262_144,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CfgComplexityMetrics {
    pub nodes: u32,
    pub edges: u32,
    pub components: u32,
    pub cyclomatic: u32,
    pub runtime_blocks: u32,
    pub runtime_edges: u32,
    pub synthetic_exit_edges: u32,
    pub excluded_imaginary_edges: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CfgExitSite {
    pub block: u32,
    pub kind: String,
    pub span: CompilerSourceMapping,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CompilerComplexity {
    pub envelope: AnalysisEnvelope,
    pub input_digest: String,
    pub definition_id: DefinitionId,
    pub import_id: String,
    pub body_id: String,
    pub compiler: CompilerIdentity,
    pub phase: String,
    pub input_manifest_hash: String,
    pub panic_strategy: String,
    pub body_span: CompilerSourceMapping,
    pub formula: String,
    pub metrics: Option<CfgComplexityMetrics>,
    pub locations_truncated: bool,
    pub reachable_blocks: Vec<u32>,
    pub unreachable_blocks: Vec<u32>,
    pub exit_sites: Vec<CfgExitSite>,
}

fn invalid(message: &str) -> AnalysisError {
    AnalysisError::InvalidInput(message.into())
}
fn limited(report: &mut CompilerComplexity, message: &str) {
    report.envelope.truncated = true;
    report
        .envelope
        .partial(UnknownReason::BudgetExhausted, message);
}
fn finish(
    mut report: CompilerComplexity,
    limits: CompilerComplexityLimits,
) -> Result<CompilerComplexity> {
    if byte_budget(&report, limits.max_response_bytes as usize).is_err() {
        report.reachable_blocks.clear();
        report.unreachable_blocks.clear();
        report.exit_sites.clear();
        report.locations_truncated = true;
        report.envelope.partial(UnknownReason::BudgetExhausted, "CFG location records withheld by response byte budget; completed counts remain separate");
        byte_budget(&report, limits.max_response_bytes as usize)?;
    }
    Ok(report)
}
fn imaginary(term: &CompilerTerminator, edge: Edge) -> bool {
    edge == Edge::Imaginary || (term.kind == "false_unwind" && edge == Edge::Unwind)
}

// Import validation owns compiler trust, source identities and MIR local effects.
// This independent graph guard checks the terminator/successor contract used here.
fn validate_graph_term(term: &CompilerTerminator) -> Result<()> {
    let count = |kind| {
        term.successors
            .iter()
            .filter(|edge| edge.kind == kind)
            .count()
    };
    let normal = count(Edge::Normal);
    let unwind = count(Edge::Unwind);
    let drops = count(Edge::CoroutineDrop);
    let total = term.successors.len();
    let mut switches = BTreeSet::new();
    for edge in &term.successors {
        if edge.kind == Edge::SwitchValue {
            let value = edge
                .switch_value
                .as_ref()
                .and_then(|value| value.parse::<u128>().ok())
                .ok_or_else(|| invalid("switch edge requires a valid unsigned 128-bit value"))?;
            if edge.switch_value.as_deref() != Some(value.to_string().as_str())
                || !switches.insert(value)
            {
                return Err(invalid(
                    "switch values must be distinct canonical decimal strings",
                ));
            }
        } else if edge.switch_value.is_some() {
            return Err(invalid("non-switch edge has a switch value"));
        }
    }
    match &term.unwind {
        Some(CompilerUnwind::Cleanup { target }) => {
            if unwind != 1
                || !term
                    .successors
                    .iter()
                    .any(|edge| edge.kind == Edge::Unwind && edge.target == *target)
            {
                return Err(invalid("cleanup unwind and successor disagree"));
            }
        }
        _ if unwind != 0 => {
            return Err(invalid("unwind successor lacks matching cleanup metadata"));
        }
        _ => {}
    }
    let valid = match term.kind.as_str() {
        "goto" => normal == 1 && total == 1 && term.unwind.is_none(),
        "switch_int" => {
            count(Edge::Otherwise) == 1 && total == switches.len() + 1 && term.unwind.is_none()
        }
        "return" | "unwind_resume" | "unreachable" | "coroutine_drop" | "tail_call" => {
            total == 0 && term.unwind.is_none()
        }
        "unwind_terminate" => {
            total == 0 && matches!(term.unwind, Some(CompilerUnwind::Terminate { .. }))
        }
        "drop" => {
            normal == 1 && drops <= 1 && total == normal + drops + unwind && term.unwind.is_some()
        }
        "call" => normal <= 1 && total == normal + unwind && term.unwind.is_some(),
        "assert" | "false_unwind" => {
            normal == 1 && total == normal + unwind && term.unwind.is_some()
        }
        "yield" => {
            count(Edge::Resume) == 1 && drops <= 1 && total == 1 + drops && term.unwind.is_none()
        }
        "false_edge" => {
            normal == 1 && count(Edge::Imaginary) == 1 && total == 2 && term.unwind.is_none()
        }
        "inline_asm" => total == normal + unwind && term.unwind.is_some(),
        _ => false,
    };
    if !valid {
        return Err(invalid(
            "unsupported terminator or inconsistent CFG successor shape",
        ));
    }
    Ok(())
}

/// Entry-reachable successor multigraph plus a single synthetic exit. Each leaf
/// and external unwind exit connects to that exit, without claiming termination.
/// Pure cyclic regions without any exit endpoint have no published measure.
pub fn analyze_compiler_complexity(
    page: &CompilerFlowPage,
    limits: CompilerComplexityLimits,
    control: &AnalysisControl,
) -> Result<CompilerComplexity> {
    if !(1..=4096).contains(&limits.max_blocks)
        || !(1..=20_000).contains(&limits.max_edges)
        || !(1024..=2_097_152).contains(&limits.max_response_bytes)
    {
        return Err(invalid(
            "CFG limits require 1..4096 blocks, 1..20000 edges and 1024..2097152 response bytes",
        ));
    }
    byte_budget(page, 4 * 1024 * 1024)?;
    if page.offset != 0
        || page.next_offset.is_some()
        || page.total_blocks as usize != page.body.blocks.len()
    {
        return Err(invalid(
            "compiler complexity requires the complete body, not a paginated block window",
        ));
    }
    if page.phase != "runtime_optimized"
        || !matches!(page.panic_strategy.as_str(), "abort" | "unwind")
        || matches!(page.coverage.status, Status::Failed | Status::Unavailable)
        || page.import_id.is_empty()
        || page.body.body_id.is_empty()
        || page.input_manifest_hash.is_empty()
    {
        return Err(invalid(
            "compiler complexity requires an available validated runtime_optimized import",
        ));
    }
    let mut report = CompilerComplexity {
        envelope: AnalysisEnvelope::new(VERSION, page.coverage.clone()),
        input_digest: digest("atlas-compiler-complexity-v1", &(VERSION, page, limits)),
        definition_id: page.definition_id.clone(),
        import_id: page.import_id.clone(),
        body_id: page.body.body_id.clone(),
        compiler: page.compiler.clone(),
        phase: page.phase.clone(),
        input_manifest_hash: page.input_manifest_hash.clone(),
        panic_strategy: page.panic_strategy.clone(),
        body_span: page.body.span.clone(),
        formula: "E - N + 2P".into(),
        metrics: None,
        locations_truncated: false,
        reachable_blocks: vec![],
        unreachable_blocks: vec![],
        exit_sites: vec![],
    };
    report.envelope.partial(UnknownReason::UnsupportedConstruct, "Structural measure of the selected imported CFG, not execution feasibility, termination, or code quality");
    report.envelope.assumptions = vec![
        "A complete validated compiler body is required; compiler trust, source mapping and MIR effects remain the importer responsibility".into(),
        "Only bb0-reachable blocks participate; runtime successor alternatives remain parallel multigraph edges even when targets coincide".into(),
        "Imaginary edges and false_unwind cleanup alternatives are excluded; excluded-edge count covers reachable source blocks only".into(),
        "Each block with external Continue/Terminate unwind contributes one synthetic-exit edge; otherwise a terminal leaf contributes one edge; false_unwind metadata is excluded".into(),
        "A terminal leaf may be unreachable, tail call, nonreturning call or assembly: its synthetic edge does not assert return or termination".into(),
        "N includes the synthetic exit, E includes its edges, P=1 for the entry-reachable connected graph; no exit endpoint means value unavailable".into(),
        "No implicit execution paths or hidden assembly/external effects are added; this measure is distinct from source-flow branches and nesting".into(),
        format!("Configured limits: {} blocks, {} normalized edges and {} response bytes", limits.max_blocks, limits.max_edges, limits.max_response_bytes),
    ];
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    if page.body.blocks.len() > limits.max_blocks as usize {
        limited(&mut report, "Compiler block budget exhausted");
        return finish(report, limits);
    }
    let mut blocks = BTreeMap::<u32, &CompilerBlock>::new();
    let mut edge_count = 0;
    for block in &page.body.blocks {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        if blocks.insert(block.index, block).is_some() {
            return Err(invalid("duplicate compiler block index"));
        }
        edge_count += block.terminator.successors.len();
        if edge_count > limits.max_edges as usize {
            limited(&mut report, "Compiler successor budget exhausted");
            return finish(report, limits);
        }
        validate_graph_term(&block.terminator)?;
    }
    if !blocks.contains_key(&0) {
        return Err(invalid("compiler CFG has no entry block bb0"));
    }
    let mut graph = DiGraph::<u32, ()>::new();
    let indices = blocks
        .keys()
        .map(|index| (*index, graph.add_node(*index)))
        .collect::<BTreeMap<_, _>>();
    for block in blocks.values() {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        for edge in &block.terminator.successors {
            let target = indices
                .get(&edge.target)
                .ok_or_else(|| invalid("compiler successor references a missing block"))?;
            if !imaginary(&block.terminator, edge.kind) {
                graph.add_edge(indices[&block.index], *target, ());
            }
        }
    }
    let mut reached = BTreeSet::new();
    let mut dfs = Dfs::new(&graph, indices[&0]);
    while let Some(node) = dfs.next(&graph) {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        reached.insert(graph[node]);
    }
    report.reachable_blocks = reached.iter().copied().collect();
    report.unreachable_blocks = blocks
        .keys()
        .filter(|index| !reached.contains(index))
        .copied()
        .collect();
    let mut runtime_edges = 0;
    let mut excluded = 0;
    for index in &reached {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        let term = &blocks[index].terminator;
        let successors = term
            .successors
            .iter()
            .filter(|edge| !imaginary(term, edge.kind))
            .count() as u32;
        runtime_edges += successors;
        excluded += term.successors.len() as u32 - successors;
        let external = if term.kind == "false_unwind" {
            None
        } else {
            match term.unwind {
                Some(CompilerUnwind::Continue) => Some("unwind_continue".to_owned()),
                Some(CompilerUnwind::Terminate { .. }) => Some("unwind_terminate".to_owned()),
                _ => None,
            }
        };
        if let Some(kind) =
            external.or_else(|| (successors == 0).then(|| format!("terminal:{}", term.kind)))
        {
            report.exit_sites.push(CfgExitSite {
                block: *index,
                kind,
                span: term.span.clone(),
            });
        }
    }
    if report.exit_sites.is_empty() {
        report.envelope.partial(UnknownReason::UnsupportedConstruct, "No terminal or external-unwind exit endpoint exists; exit-normalized cyclomatic value is unavailable");
        return finish(report, limits);
    }
    let synthetic_exit_edges = report.exit_sites.len() as u32;
    let nodes = reached.len() as u32 + 1;
    let edges = runtime_edges + synthetic_exit_edges;
    if edges > limits.max_edges {
        limited(&mut report, "Exit-normalized CFG edge budget exhausted");
        return finish(report, limits);
    }
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    let cyclomatic = edges
        .checked_add(2)
        .and_then(|value| value.checked_sub(nodes))
        .ok_or_else(|| invalid("invalid connected normalized CFG counts"))?;
    report.metrics = Some(CfgComplexityMetrics {
        nodes,
        edges,
        components: 1,
        cyclomatic,
        runtime_blocks: reached.len() as u32,
        runtime_edges,
        synthetic_exit_edges,
        excluded_imaginary_edges: excluded,
    });
    finish(report, limits)
}
