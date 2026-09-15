//! Whole-local reaching definitions over one compiler body, not pointer/value analysis.
use crate::{AnalysisControl, AnalysisEnvelope, AnalysisError, Result, byte_budget};
use atlas_model::{
    CompilerBlock, CompilerBody, CompilerEdgeKind, CompilerLocalEffects, CompilerUnknownEffect,
    Coverage, UnknownReason,
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DataflowLimits {
    pub max_blocks: u32,
    pub max_locals: u32,
    /// Applies separately to input facts, retained abstract-state facts, and output facts.
    pub max_facts: u32,
    pub max_iterations: u32,
    pub max_response_bytes: u32,
}

impl Default for DataflowLimits {
    fn default() -> Self {
        Self {
            max_blocks: 1000,
            max_locals: 1000,
            max_facts: 50_000,
            max_iterations: 100_000,
            max_response_bytes: 2 * 1024 * 1024,
        }
    }
}

impl DataflowLimits {
    fn validate(self) -> Result<()> {
        if !(1..=4096).contains(&self.max_blocks)
            || !(1..=4096).contains(&self.max_locals)
            || !(1..=200_000).contains(&self.max_facts)
            || !(1..=1_000_000).contains(&self.max_iterations)
            || !(1024..=2 * 1024 * 1024).contains(&self.max_response_bytes)
        {
            return Err(invalid(
                "limits require 1..4096 blocks/locals, 1..200000 facts, 1..1000000 iterations, and 1024..2097152 response bytes",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MirPoint {
    Entry,
    Statement { block: u32, index: u32 },
    Terminator { block: u32 },
    NormalReturn { block: u32, target: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LocalDefinition {
    pub local: u32,
    pub point: MirPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LocalUse {
    pub local: u32,
    pub point: MirPoint,
    pub reaching: Vec<LocalDefinition>,
    /// An abstract tracking gap on some path, not a Rust undefined-behavior verdict.
    pub may_be_uninitialized: bool,
    pub possibly_changed_by_unknown_memory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UnknownMemoryEffect {
    pub point: MirPoint,
    pub effects: Vec<CompilerUnknownEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReachingDefinitions {
    pub envelope: AnalysisEnvelope,
    pub body_id: String,
    pub fixed_point: bool,
    pub iterations: u32,
    pub reachable_blocks: Vec<u32>,
    pub definitions: Vec<LocalDefinition>,
    pub uses: Vec<LocalUse>,
    pub unknown_memory_effects: Vec<UnknownMemoryEffect>,
}

impl ReachingDefinitions {
    fn new(body: &CompilerBody, coverage: &Coverage) -> Self {
        let mut envelope = AnalysisEnvelope::new("mir-reaching-definitions-v1", coverage.clone());
        envelope.assumptions = vec![
            "One selected compiler body; all listed runtime edges are considered feasible; imaginary edges are excluded".into(),
            "Definitions track whole-local assignments, not pointee values, aliases, cross-function values, or Rust validity".into(),
            "Argument locals are defined at entry; moves and storage lifetime events clear tracked local values after reads".into(),
            "Unknown memory effects are never resolved; they conservatively taint later reachable uses without inventing assignments".into(),
        ];
        Self {
            envelope,
            body_id: body.body_id.clone(),
            fixed_point: false,
            iterations: 0,
            reachable_blocks: vec![],
            definitions: vec![],
            uses: vec![],
            unknown_memory_effects: vec![],
        }
    }

    fn limited(&mut self, reason: &str) {
        self.envelope.truncated = true;
        self.envelope
            .partial(UnknownReason::BudgetExhausted, reason);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Value {
    definitions: BTreeSet<LocalDefinition>,
    uninitialized: bool,
}

impl Default for Value {
    fn default() -> Self {
        Self {
            definitions: BTreeSet::new(),
            uninitialized: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct State {
    // Missing local entries mean no tracked definition, possibly uninitialized.
    values: BTreeMap<u32, Value>,
    unknown: BTreeSet<CompilerUnknownEffect>,
}

impl State {
    fn facts(&self) -> usize {
        self.values.len()
            + self
                .values
                .values()
                .map(|value| value.definitions.len())
                .sum::<usize>()
            + self.unknown.len()
    }

    fn define(&mut self, local: u32, point: &MirPoint) {
        self.values.insert(
            local,
            Value {
                definitions: BTreeSet::from([LocalDefinition {
                    local,
                    point: point.clone(),
                }]),
                uninitialized: false,
            },
        );
    }

    fn transfer(&mut self, effects: &CompilerLocalEffects, point: &MirPoint) {
        for local in effects
            .moves
            .iter()
            .chain(&effects.storage_live)
            .chain(&effects.storage_dead)
        {
            self.values.remove(local);
        }
        for local in &effects.defs {
            self.define(*local, point);
        }
        self.unknown.extend(&effects.unknown_effects);
    }

    fn join(&mut self, other: &Self, control: &AnalysisControl) -> bool {
        for (local, value) in &mut self.values {
            if control.stopped() {
                return false;
            }
            if !other.values.contains_key(local) {
                value.uninitialized = true;
            }
        }
        for (local, value) in &other.values {
            if control.stopped() {
                return false;
            }
            let destination = self.values.entry(*local).or_default();
            destination.definitions.extend(value.definitions.clone());
            destination.uninitialized |= value.uninitialized;
        }
        self.unknown.extend(&other.unknown);
        true
    }
}

fn invalid(reason: &str) -> AnalysisError {
    AnalysisError::InvalidInput(reason.into())
}

fn local_references(effects: &CompilerLocalEffects) -> impl Iterator<Item = &u32> {
    effects
        .defs
        .iter()
        .chain(&effects.uses)
        .chain(&effects.moves)
        .chain(&effects.storage_live)
        .chain(&effects.storage_dead)
}

fn terminator_effects(block: &CompilerBlock) -> Cow<'_, CompilerLocalEffects> {
    if block.terminator.kind == "yield"
        || block.terminator.successors.iter().any(|edge| {
            matches!(
                edge.kind,
                CompilerEdgeKind::Resume | CompilerEdgeKind::CoroutineDrop
            )
        })
    {
        let mut effects = block.terminator.locals.clone();
        if !effects
            .unknown_effects
            .contains(&CompilerUnknownEffect::ExternalState)
        {
            effects
                .unknown_effects
                .push(CompilerUnknownEffect::ExternalState);
        }
        Cow::Owned(effects)
    } else {
        Cow::Borrowed(&block.terminator.locals)
    }
}

/// A deterministic may-analysis. Incomplete fixed-point searches emit no def-use claims.
pub fn reaching_definitions(
    body: &CompilerBody,
    coverage: &Coverage,
    limits: DataflowLimits,
    control: &AnalysisControl,
) -> Result<ReachingDefinitions> {
    limits.validate()?;
    if body.body_id.is_empty() || body.body_id.len() > 256 {
        return Err(invalid("compiler body identity must contain 1..256 bytes"));
    }
    let mut result = ReachingDefinitions::new(body, coverage);
    if control.stopped() {
        result.envelope.stop(control);
        return Ok(result);
    }
    if body.blocks.len() > limits.max_blocks as usize
        || body.locals.len() > limits.max_locals as usize
    {
        result.limited("Selected body exceeds the block or local budget");
        return Ok(result);
    }
    if byte_budget(body, 32 * 1024 * 1024).is_err() {
        result.limited("Selected compiler body exceeds the 32 MiB input budget");
        return Ok(result);
    }
    if body.blocks.is_empty() || body.locals.is_empty() {
        return Err(invalid(
            "a MIR body requires an entry block and a return local",
        ));
    }
    let blocks: BTreeMap<_, _> = body
        .blocks
        .iter()
        .map(|block| (block.index, block))
        .collect();
    let locals: BTreeSet<_> = body.locals.iter().map(|local| local.index).collect();
    if blocks.len() != body.blocks.len()
        || locals.len() != body.locals.len()
        || !blocks.contains_key(&0)
        || !locals.contains(&0)
        || body.argument_count as usize >= body.locals.len()
        || !(1..=body.argument_count).all(|local| locals.contains(&local))
    {
        return Err(invalid(
            "duplicate or missing entry block, return local, or argument local",
        ));
    }
    let mut input_facts = blocks.len() + locals.len();
    for block in blocks.values() {
        if control.stopped() {
            result.envelope.stop(control);
            return Ok(result);
        }
        if input_facts + block.statements.len() > limits.max_facts as usize {
            result.limited("Selected body exceeds the input fact budget");
            return Ok(result);
        }
        for (index, statement) in block.statements.iter().enumerate() {
            if statement.index as usize != index {
                return Err(invalid(
                    "statement indices must match their execution order",
                ));
            }
        }
        for effects in block
            .statements
            .iter()
            .map(|statement| &statement.locals)
            .chain([&block.terminator.locals])
        {
            input_facts += 1;
            for local in local_references(effects) {
                input_facts += 1;
                if input_facts > limits.max_facts as usize {
                    result.limited("Selected body exceeds the input fact budget");
                    return Ok(result);
                }
                if !locals.contains(local) {
                    return Err(invalid("local effects reference a missing local"));
                }
            }
            input_facts += effects.unknown_effects.len();
            if input_facts > limits.max_facts as usize {
                result.limited("Selected body exceeds the input fact budget");
                return Ok(result);
            }
            if control.stopped() {
                result.envelope.stop(control);
                return Ok(result);
            }
        }
        for local in &block.terminator.normal_return_defs {
            input_facts += 1;
            if input_facts > limits.max_facts as usize {
                result.limited("Selected body exceeds the input fact budget");
                return Ok(result);
            }
            if !locals.contains(local) {
                return Err(invalid(
                    "normal-return assignment references a missing local",
                ));
            }
        }
        for edge in &block.terminator.successors {
            input_facts += 1;
            if input_facts > limits.max_facts as usize {
                result.limited("Selected body exceeds the input fact budget");
                return Ok(result);
            }
            if !blocks.contains_key(&edge.target) {
                return Err(invalid("successor references a missing block"));
            }
            if edge.kind == CompilerEdgeKind::Imaginary {
                result.envelope.partial(
                    UnknownReason::UnsupportedConstruct,
                    "Imaginary compiler edges are excluded from runtime reaching definitions",
                );
            }
        }
        if input_facts > limits.max_facts as usize {
            result.limited("Selected body exceeds the input fact budget");
            return Ok(result);
        }
    }

    let mut entry = State::default();
    for local in 1..=body.argument_count {
        entry.define(local, &MirPoint::Entry);
    }
    let mut retained_facts = entry.facts();
    if retained_facts > limits.max_facts as usize {
        result.limited("Argument state exceeds the retained fact budget");
        return Ok(result);
    }
    let mut entries = BTreeMap::from([(0, entry)]);
    let mut worklist = BTreeSet::from([0]);
    while let Some(index) = worklist.pop_first() {
        if control.stopped() {
            result.envelope.stop(control);
            return Ok(result);
        }
        if result.iterations == limits.max_iterations {
            result.limited("Reaching definitions did not converge within the iteration budget");
            return Ok(result);
        }
        result.iterations += 1;
        let block = blocks[&index];
        let mut state = entries[&index].clone();
        for statement in &block.statements {
            if control.stopped() {
                result.envelope.stop(control);
                return Ok(result);
            }
            state.transfer(
                &statement.locals,
                &MirPoint::Statement {
                    block: index,
                    index: statement.index,
                },
            );
            if state.facts() > limits.max_facts as usize {
                result.limited("Transfer state exceeds the abstract-state fact budget");
                return Ok(result);
            }
        }
        state.transfer(
            &terminator_effects(block),
            &MirPoint::Terminator { block: index },
        );
        if state.facts() > limits.max_facts as usize {
            result.limited("Terminator state exceeds the abstract-state fact budget");
            return Ok(result);
        }
        let mut edges: Vec<_> = block.terminator.successors.iter().collect();
        edges.sort_by_key(|edge| (edge.target, edge_order(edge.kind), &edge.switch_value));
        for edge in edges {
            if edge.kind == CompilerEdgeKind::Imaginary {
                continue;
            }
            if control.stopped() {
                result.envelope.stop(control);
                return Ok(result);
            }
            let mut outgoing = state.clone();
            if edge.kind == CompilerEdgeKind::Normal {
                for local in &block.terminator.normal_return_defs {
                    outgoing.define(
                        *local,
                        &MirPoint::NormalReturn {
                            block: index,
                            target: edge.target,
                        },
                    );
                }
            }
            let previous = entries.get(&edge.target);
            let previous_facts = previous.map_or(0, State::facts);
            if let Some(previous) = previous {
                let mut merged = previous.clone();
                if !merged.join(&outgoing, control) {
                    result.envelope.stop(control);
                    return Ok(result);
                }
                if merged == *previous {
                    continue;
                }
                outgoing = merged;
            }
            retained_facts = retained_facts - previous_facts + outgoing.facts();
            if retained_facts > limits.max_facts as usize {
                result.limited("Reaching-definition states exceed the retained fact budget");
                return Ok(result);
            }
            entries.insert(edge.target, outgoing);
            worklist.insert(edge.target);
        }
    }

    result.fixed_point = true;
    result.reachable_blocks = entries.keys().copied().collect();
    let mut definitions = BTreeSet::new();
    for local in 1..=body.argument_count {
        definitions.insert(LocalDefinition {
            local,
            point: MirPoint::Entry,
        });
    }
    let mut output_facts = definitions.len() + result.reachable_blocks.len();
    'emit: for (&index, entry) in &entries {
        let mut state = entry.clone();
        let block = blocks[&index];
        let terminator = terminator_effects(block);
        for (point, effects) in block
            .statements
            .iter()
            .map(|statement| {
                (
                    MirPoint::Statement {
                        block: index,
                        index: statement.index,
                    },
                    &statement.locals,
                )
            })
            .chain([(MirPoint::Terminator { block: index }, terminator.as_ref())])
        {
            if control.stopped() {
                result.envelope.stop(control);
                break 'emit;
            }
            let used: BTreeSet<_> = effects.uses.iter().chain(&effects.moves).copied().collect();
            let extra = used
                .iter()
                .map(|local| 1 + state.values.get(local).map_or(0, |v| v.definitions.len()))
                .sum::<usize>()
                + effects.defs.len()
                + effects.unknown_effects.len();
            if output_facts + extra > limits.max_facts as usize {
                result.limited("Reaching-definition output exceeds the fact budget");
                break 'emit;
            }
            output_facts += extra;
            for local in used {
                let value = state.values.get(&local).cloned().unwrap_or_default();
                result.uses.push(LocalUse {
                    local,
                    point: point.clone(),
                    reaching: value.definitions.into_iter().collect(),
                    may_be_uninitialized: value.uninitialized,
                    possibly_changed_by_unknown_memory: !state.unknown.is_empty()
                        || !effects.unknown_effects.is_empty(),
                });
            }
            if !effects.unknown_effects.is_empty() {
                result.unknown_memory_effects.push(UnknownMemoryEffect {
                    point: point.clone(),
                    effects: effects
                        .unknown_effects
                        .iter()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                });
                result.envelope.partial(
                    UnknownReason::UnsupportedConstruct,
                    "Unknown call, pointer, alias, or external memory effects remain unresolved",
                );
            }
            definitions.extend(effects.defs.iter().map(|local| LocalDefinition {
                local: *local,
                point: point.clone(),
            }));
            state.transfer(effects, &point);
        }
        for edge in &block.terminator.successors {
            if edge.kind == CompilerEdgeKind::Normal {
                for local in &block.terminator.normal_return_defs {
                    output_facts += 1;
                    if output_facts > limits.max_facts as usize {
                        result.limited("Reaching-definition output exceeds the fact budget");
                        break 'emit;
                    }
                    definitions.insert(LocalDefinition {
                        local: *local,
                        point: MirPoint::NormalReturn {
                            block: index,
                            target: edge.target,
                        },
                    });
                }
            }
        }
    }
    result.definitions = definitions.into_iter().collect();
    if byte_budget(&result, limits.max_response_bytes as usize).is_err() {
        result.definitions.clear();
        result.uses.clear();
        result.unknown_memory_effects.clear();
        result.reachable_blocks.clear();
        result.limited("Reaching-definition response exceeds the byte budget; claims withheld");
        byte_budget(&result, limits.max_response_bytes as usize)?;
    }
    Ok(result)
}

fn edge_order(kind: CompilerEdgeKind) -> u8 {
    match kind {
        CompilerEdgeKind::Normal => 0,
        CompilerEdgeKind::SwitchValue => 1,
        CompilerEdgeKind::Otherwise => 2,
        CompilerEdgeKind::Unwind => 3,
        CompilerEdgeKind::Resume => 4,
        CompilerEdgeKind::CoroutineDrop => 5,
        CompilerEdgeKind::Imaginary => 6,
    }
}
