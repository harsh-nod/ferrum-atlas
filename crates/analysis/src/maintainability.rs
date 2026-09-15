//! Versioned measurements of captured syntax, never a universal quality score.
use crate::{AnalysisControl, AnalysisEnvelope, AnalysisError, Result, byte_budget};
use atlas_model::{
    CfgStatus, Coverage, Definition, DefinitionId, SourceFile, Span, UnknownReason, digest,
};
use ra_ap_parser::LexedStr;
use ra_ap_syntax::{
    AstNode, Edition, SyntaxKind, SyntaxNode, SyntaxToken, TextRange, WalkEvent, ast, ast::HasName,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const VERSION: &str = "source-maintainability-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MaintainabilityLimits {
    pub max_source_bytes: u32,
    pub max_nodes: u32,
    pub max_tokens: u32,
    /// Independently bounds each collection of source locations.
    pub max_locations: u32,
    pub max_response_bytes: u32,
}
impl Default for MaintainabilityLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 262_144,
            max_nodes: 20_000,
            max_tokens: 50_000,
            max_locations: 1000,
            max_response_bytes: 1_048_576,
        }
    }
}
impl MaintainabilityLimits {
    fn validate(self) -> Result<()> {
        if !(1..=1_048_576).contains(&self.max_source_bytes)
            || !(1..=100_000).contains(&self.max_nodes)
            || !(1..=200_000).contains(&self.max_tokens)
            || !(1..=5000).contains(&self.max_locations)
            || !(1024..=2_097_152).contains(&self.max_response_bytes)
        {
            return Err(invalid("maintainability limits outside supported bounds"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SyntaxMetrics {
    pub source_lines: u32,
    pub lexical_tokens: u32,
    pub max_nesting: u32,
    pub unsafe_boundaries: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct NestingSite {
    pub kind: String,
    pub depth: u32,
    pub span: Span,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UnsafeBoundary {
    pub kind: String,
    pub keyword_span: Span,
    pub boundary_span: Span,
    /// Nearest lexical unsafe boundary, not dynamic scope or a safety proof.
    pub enclosing_boundary: Option<Span>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MetricUnknown {
    pub span: Span,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceMaintainability {
    pub envelope: AnalysisEnvelope,
    pub input_digest: String,
    pub definition_id: DefinitionId,
    pub span: Span,
    pub body_span: Span,
    pub visited_nodes: u32,
    pub visited_tokens: u32,
    /// Present only after both complete lexical and syntax traversals.
    pub metrics: Option<SyntaxMetrics>,
    pub locations_truncated: bool,
    pub code_lines: Vec<Span>,
    pub nesting_sites: Vec<NestingSite>,
    pub unsafe_sites: Vec<UnsafeBoundary>,
    pub unknowns: Vec<MetricUnknown>,
}

fn invalid(message: &str) -> AnalysisError {
    AnalysisError::InvalidInput(message.into())
}
fn absolute(range: TextRange, definition: &Definition) -> Span {
    Span {
        file_id: definition.file_id.clone(),
        start: definition.span.start + u32::from(range.start()),
        end: definition.span.start + u32::from(range.end()),
    }
}
fn keep<T>(items: &mut Vec<T>, item: T, cap: u32, truncated: &mut bool) {
    if items.len() < cap as usize {
        items.push(item);
    } else {
        *truncated = true;
    }
}
fn unknown(
    report: &mut SourceMaintainability,
    span: Span,
    reason: &str,
    limits: MaintainabilityLimits,
) {
    keep(
        &mut report.unknowns,
        MetricUnknown {
            span,
            reason: reason.into(),
        },
        limits.max_locations,
        &mut report.locations_truncated,
    );
}
fn finish(
    mut report: SourceMaintainability,
    limits: MaintainabilityLimits,
) -> Result<SourceMaintainability> {
    if report.locations_truncated {
        report.envelope.partial(
            UnknownReason::BudgetExhausted,
            "Source location records were capped; completed metric counts remain separate",
        );
    }
    if byte_budget(&report, limits.max_response_bytes as usize).is_err() {
        report.code_lines.clear();
        report.nesting_sites.clear();
        report.unsafe_sites.clear();
        report.unknowns.clear();
        report.locations_truncated = true;
        report.envelope.partial(UnknownReason::BudgetExhausted, "Source location records withheld by response byte budget; counts are present only if counting finished");
        byte_budget(&report, limits.max_response_bytes as usize)?;
    }
    Ok(report)
}
fn limited(report: &mut SourceMaintainability, message: &str) {
    report.envelope.truncated = true;
    report
        .envelope
        .partial(UnknownReason::BudgetExhausted, message);
}

// Ordinary, unsafe and const blocks are not nesting increments. Else-if follows
// the literal AST, so it adds another level; closures and local functions do too.
fn nesting_kind(node: &SyntaxNode, root: &SyntaxNode) -> Option<&'static str> {
    if node == root {
        return None;
    }
    if ast::IfExpr::can_cast(node.kind()) {
        return Some("if");
    }
    if ast::MatchExpr::can_cast(node.kind()) {
        return Some("match");
    }
    if ast::LoopExpr::can_cast(node.kind()) {
        return Some("loop");
    }
    if ast::WhileExpr::can_cast(node.kind()) {
        return Some("while");
    }
    if ast::ForExpr::can_cast(node.kind()) {
        return Some("for");
    }
    if ast::ClosureExpr::can_cast(node.kind()) {
        return Some("closure");
    }
    if ast::Fn::can_cast(node.kind()) {
        return Some("nested_function");
    }
    if let Some(block) = ast::BlockExpr::cast(node.clone()) {
        if block.async_token().is_some() {
            return Some("async_block");
        }
        if block.try_block_modifier().is_some() {
            return Some("try_block");
        }
        if block.gen_token().is_some() {
            return Some("gen_block");
        }
    }
    None
}
fn unsafe_keyword(node: &SyntaxNode) -> Option<(&'static str, SyntaxToken)> {
    macro_rules! boundary {
        ($type:ty, $label:literal) => {
            if let Some(value) = <$type>::cast(node.clone()) {
                return value.unsafe_token().map(|token| ($label, token));
            }
        };
    }
    boundary!(ast::Fn, "function");
    boundary!(ast::BlockExpr, "block");
    boundary!(ast::Impl, "impl");
    boundary!(ast::Trait, "trait");
    boundary!(ast::ExternBlock, "extern_block");
    boundary!(ast::UnsafeMeta, "attribute");
    boundary!(ast::FnPtrType, "function_pointer_type");
    None
}

/// Measures the exact selected source definition, including its header and local
/// declarations. Expanded/generated syntax is not available and is never guessed.
pub fn analyze_maintainability(
    source: &SourceFile,
    definition: &Definition,
    limits: MaintainabilityLimits,
    control: &AnalysisControl,
) -> Result<SourceMaintainability> {
    limits.validate()?;
    byte_budget(definition, 65_536)?;
    if source.id != definition.file_id
        || definition.span.file_id != source.id
        || !matches!(definition.kind.as_str(), "function" | "method")
    {
        return Err(invalid(
            "maintainability selection must be a function in the exact source file",
        ));
    }
    let fragment = source
        .text
        .get(definition.span.start as usize..definition.span.end as usize)
        .ok_or_else(|| invalid("definition span is not an exact UTF-8 source range"))?;
    if fragment.len() > 1_048_576 {
        return Err(AnalysisError::BudgetExhausted);
    }
    if let Some(body) = &definition.body_span
        && (body.file_id != source.id
            || body.start < definition.span.start
            || body.end > definition.span.end
            || source
                .text
                .get(body.start as usize..body.end as usize)
                .is_none())
    {
        return Err(invalid("body span is outside the selected definition"));
    }
    let mut report = SourceMaintainability {
        envelope: AnalysisEnvelope::new(
            VERSION,
            Coverage::partial(
                UnknownReason::SyntaxOnly,
                "Captured syntax measurements, not semantic complexity or code quality",
            ),
        ),
        input_digest: digest(
            "atlas-source-maintainability-v1",
            &(VERSION, fragment, definition, limits),
        ),
        definition_id: definition.id.clone(),
        span: definition.span.clone(),
        body_span: definition
            .body_span
            .clone()
            .unwrap_or_else(|| definition.span.clone()),
        visited_nodes: 0,
        visited_tokens: 0,
        metrics: None,
        locations_truncated: false,
        code_lines: vec![],
        nesting_sites: vec![],
        unsafe_sites: vec![],
        unknowns: vec![],
    };
    report.envelope.assumptions = vec![
        "Exact captured function text including header, attributes and nested declarations; generated-origin provenance is unknown".into(),
        "Rust edition 2024, pinned rust-analyzer lexer tokens excluding whitespace/comments; compound punctuation retains lexical token boundaries".into(),
        "Code lines are LF-delimited lines intersecting non-whitespace non-comment token characters; CRLF is preserved and blank literal lines are excluded".into(),
        "Nesting increments for if/match/loop/while/for, closures, local functions, async/try/gen blocks; else-if is nested; ordinary/unsafe/const blocks do not increment".into(),
        "Unsafe locations are explicit syntax boundaries and nearest lexical enclosures, not unsafe operations, dynamic permissions or a safety score".into(),
        "Macro expansion, generated output, active cfg behavior and compiler CFG complexity are separate and not inferred from source counts".into(),
    ];
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    if fragment.len() > limits.max_source_bytes as usize {
        limited(
            &mut report,
            "Selected definition exceeds source byte budget",
        );
        return finish(report, limits);
    }
    let parsed = ast::SourceFile::parse(fragment, Edition::CURRENT);
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    if !parsed.errors().is_empty() {
        report.envelope.partial(
            UnknownReason::AnalysisFailed,
            "Parser errors prevent complete syntax measurements",
        );
        unknown(
            &mut report,
            definition.span.clone(),
            "Selected definition has parser errors; metric values are withheld",
            limits,
        );
        return finish(report, limits);
    }
    let root = parsed.syntax_node();
    let mut items = root.children();
    let function = items
        .next()
        .and_then(ast::Fn::cast)
        .filter(|_| items.next().is_none())
        .ok_or_else(|| invalid("definition span must contain exactly one function"))?;
    if function.name().is_none_or(|name| {
        name.text().strip_prefix("r#").unwrap_or(name.text())
            != definition
                .name
                .strip_prefix("r#")
                .unwrap_or(&definition.name)
    }) {
        return Err(invalid(
            "parsed function name differs from selected definition",
        ));
    }
    let body = function
        .syntax()
        .children()
        .find_map(ast::BlockExpr::cast)
        .ok_or_else(|| invalid("selected function has no body"))?;
    report.body_span = absolute(body.syntax().text_range(), definition);
    if definition
        .body_span
        .as_ref()
        .is_some_and(|expected| expected != &report.body_span)
    {
        return Err(invalid("body span differs from parsed function body"));
    }
    if definition.cfg_status != CfgStatus::Active {
        report.envelope.partial(
            UnknownReason::CfgUnknown,
            "Selected syntax is not known active in the context",
        );
        unknown(
            &mut report,
            definition.span.clone(),
            "Counts describe captured syntax, not active compiled code",
            limits,
        );
    }
    let mut metrics = SyntaxMetrics {
        source_lines: 0,
        lexical_tokens: 0,
        max_nesting: 0,
        unsafe_boundaries: 0,
    };
    let mut depth = 0;
    let mut unsafe_stack = vec![];
    for event in function.syntax().preorder() {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        match event {
            WalkEvent::Enter(node) => {
                if report.visited_nodes == limits.max_nodes {
                    limited(
                        &mut report,
                        "AST node budget exhausted before counting completed",
                    );
                    return finish(report, limits);
                }
                report.visited_nodes += 1;
                if let Some(kind) = nesting_kind(&node, function.syntax()) {
                    depth += 1;
                    metrics.max_nesting = metrics.max_nesting.max(depth);
                    keep(
                        &mut report.nesting_sites,
                        NestingSite {
                            kind: kind.into(),
                            depth,
                            span: absolute(node.text_range(), definition),
                        },
                        limits.max_locations,
                        &mut report.locations_truncated,
                    );
                }
                if let Some((kind, token)) = unsafe_keyword(&node) {
                    metrics.unsafe_boundaries += 1;
                    let boundary_span = absolute(node.text_range(), definition);
                    keep(
                        &mut report.unsafe_sites,
                        UnsafeBoundary {
                            kind: kind.into(),
                            keyword_span: absolute(token.text_range(), definition),
                            boundary_span: boundary_span.clone(),
                            enclosing_boundary: unsafe_stack.last().cloned(),
                        },
                        limits.max_locations,
                        &mut report.locations_truncated,
                    );
                    unsafe_stack.push(boundary_span);
                }
                if ast::MacroCall::can_cast(node.kind())
                    || ast::MacroRules::can_cast(node.kind())
                    || ast::MacroDef::can_cast(node.kind())
                {
                    report.envelope.partial(UnknownReason::MacroUnavailable, "Macro source tokens are counted, but expansion nesting and unsafe boundaries are unknown");
                    unknown(
                        &mut report,
                        absolute(node.text_range(), definition),
                        "Macro expansion and generated syntax are not measured",
                        limits,
                    );
                }
                if ast::Attr::can_cast(node.kind()) {
                    report.envelope.partial(UnknownReason::MacroUnavailable, "Attributes may transform or conditionally remove the measured source syntax");
                    unknown(
                        &mut report,
                        absolute(node.text_range(), definition),
                        "Attribute effects and generated output are not measured",
                        limits,
                    );
                }
            }
            WalkEvent::Leave(node) => {
                if nesting_kind(&node, function.syntax()).is_some() {
                    depth -= 1;
                }
                if unsafe_keyword(&node).is_some() {
                    unsafe_stack.pop();
                }
            }
        }
    }
    let lexed = LexedStr::new(Edition::CURRENT, fragment);
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    if lexed.errors().next().is_some() {
        report.envelope.partial(
            UnknownReason::AnalysisFailed,
            "Lexer errors prevent complete token measurements",
        );
        unknown(
            &mut report,
            definition.span.clone(),
            "Selected definition has lexer errors; metric values are withheld",
            limits,
        );
        return finish(report, limits);
    }
    let mut line_start = 0;
    let mut line_has_code = false;
    for index in 0..lexed.len() {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        if report.visited_tokens == limits.max_tokens {
            limited(
                &mut report,
                "Lexical token budget exhausted before counting completed",
            );
            return finish(report, limits);
        }
        report.visited_tokens += 1;
        let code = !matches!(
            lexed.kind(index),
            SyntaxKind::WHITESPACE | SyntaxKind::COMMENT
        );
        if code {
            metrics.lexical_tokens += 1;
        }
        let token_start = lexed.text_start(index);
        for (character, (offset, ch)) in lexed.text(index).char_indices().enumerate() {
            if character.is_multiple_of(1024) && control.stopped() {
                report.envelope.stop(control);
                return finish(report, limits);
            }
            if ch == '\n' {
                if line_has_code {
                    metrics.source_lines += 1;
                    let range = TextRange::new(
                        (line_start as u32).into(),
                        ((token_start + offset) as u32).into(),
                    );
                    keep(
                        &mut report.code_lines,
                        absolute(range, definition),
                        limits.max_locations,
                        &mut report.locations_truncated,
                    );
                }
                line_start = token_start + offset + 1;
                line_has_code = false;
            } else if code && !ch.is_whitespace() {
                line_has_code = true;
            }
        }
    }
    if line_has_code {
        metrics.source_lines += 1;
        keep(
            &mut report.code_lines,
            absolute(
                TextRange::new((line_start as u32).into(), (fragment.len() as u32).into()),
                definition,
            ),
            limits.max_locations,
            &mut report.locations_truncated,
        );
    }
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    report.metrics = Some(metrics);
    finish(report, limits)
}
