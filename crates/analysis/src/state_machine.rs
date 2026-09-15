//! Opt-in syntax candidates, deliberately separate from human review declarations.
use crate::{AnalysisControl, AnalysisEnvelope, AnalysisError, Result, byte_budget};
use atlas_model::{
    CfgStatus, Coverage, Definition, DefinitionId, SourceFile, Span, UnknownReason, digest,
};
use ra_ap_syntax::{AstNode, Edition, SyntaxNode, ast, ast::HasGenericArgs, ast::HasName};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const VERSION: &str = "state-machine-syntax-v1";
const SYNTAX_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StateMachineLimits {
    pub max_body_bytes: u32,
    pub max_nodes: u32,
    pub max_transitions: u32,
    pub max_unknowns: u32,
    pub max_response_bytes: u32,
}
impl Default for StateMachineLimits {
    fn default() -> Self {
        Self {
            max_body_bytes: 262_144,
            max_nodes: 20_000,
            max_transitions: 200,
            max_unknowns: 100,
            max_response_bytes: 1_048_576,
        }
    }
}
impl StateMachineLimits {
    fn validate(self) -> Result<()> {
        if !(1..=1_048_576).contains(&self.max_body_bytes)
            || !(1..=100_000).contains(&self.max_nodes)
            || !(1..=500).contains(&self.max_transitions)
            || !(1..=500).contains(&self.max_unknowns)
            || !(1024..=2_097_152).contains(&self.max_response_bytes)
        {
            return Err(invalid("state-machine limits outside supported bounds"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StateMachineInference {
    pub envelope: AnalysisEnvelope,
    pub input_digest: String,
    pub definition_id: DefinitionId,
    pub enum_path: String,
    pub state_place: String,
    pub body_span: Span,
    pub visited_nodes: u32,
    pub candidates: Vec<StateTransitionCandidate>,
    pub unknowns: Vec<StateMachineUnknown>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StateTransitionCandidate {
    pub id: String,
    pub from_variant: String,
    pub to_variant: String,
    pub match_span: Span,
    pub arm_span: Span,
    pub assignment_span: Span,
    pub guard: Option<StateMachineSyntax>,
    pub action: StateMachineSyntax,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StateMachineSyntax {
    pub span: Span,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StateMachineUnknown {
    pub span: Span,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StateTransitionDecision {
    Accepted,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StateTransitionReview {
    pub input_digest: String,
    pub candidate_id: String,
    pub reviewer: String,
    pub note: String,
    pub decision: StateTransitionDecision,
}

fn invalid(message: &str) -> AnalysisError {
    AnalysisError::InvalidInput(message.into())
}
fn ident(text: &str) -> &str {
    text.strip_prefix("r#").unwrap_or(text)
}
fn path_parts(path: ast::Path) -> Option<Vec<String>> {
    if path.first_qualifier_or_self().coloncolon_token().is_some() {
        return None;
    }
    path.segments()
        .map(|segment| {
            if segment.generic_arg_list().is_some() || segment.type_anchor().is_some() {
                return None;
            }
            let name = segment.name_ref()?.text().to_string();
            Some(ident(&name).to_owned())
        })
        .collect()
}
fn expr_path(expr: ast::Expr) -> Option<Vec<String>> {
    path_parts(ast::PathExpr::cast(expr.syntax().clone())?.path()?)
}
fn selection(text: &str) -> Result<Vec<String>> {
    if text.is_empty() || text.len() > 512 {
        return Err(invalid("invalid selection length"));
    }
    let parsed = ast::Expr::parse(text, Edition::CURRENT);
    if !parsed.errors().is_empty() {
        return Err(invalid("selection must be a plain Rust path"));
    }
    let path = ast::PathExpr::cast(parsed.syntax_node())
        .and_then(|expr| expr.path())
        .and_then(path_parts)
        .filter(|parts| !parts.is_empty())
        .ok_or_else(|| invalid("selection must be a plain relative Rust path without generics"))?;
    Ok(path)
}
fn span(node: &SyntaxNode, definition: &Definition) -> Span {
    let range = node.text_range();
    Span {
        file_id: definition.file_id.clone(),
        start: definition.span.start + u32::from(range.start()),
        end: definition.span.start + u32::from(range.end()),
    }
}
fn syntax(node: &SyntaxNode, definition: &Definition) -> Option<StateMachineSyntax> {
    if u32::from(node.text_range().len()) as usize > SYNTAX_BYTES {
        return None;
    }
    Some(StateMachineSyntax {
        span: span(node, definition),
        text: node.text().to_string(),
    })
}
fn unknown(report: &mut StateMachineInference, at: Span, reason: &str, limits: StateMachineLimits) {
    if report.unknowns.len() < limits.max_unknowns as usize {
        report.unknowns.push(StateMachineUnknown {
            span: at,
            reason: reason.into(),
        });
    } else {
        report.envelope.truncated = true;
        report.envelope.partial(
            UnknownReason::BudgetExhausted,
            "Unknown records were capped",
        );
    }
}
fn finish(
    mut report: StateMachineInference,
    limits: StateMachineLimits,
) -> Result<StateMachineInference> {
    report.unknowns.sort_by(|a, b| {
        (a.span.start, a.span.end, &a.reason).cmp(&(b.span.start, b.span.end, &b.reason))
    });
    report.unknowns.dedup();
    if byte_budget(&report, limits.max_response_bytes as usize).is_err() {
        report.candidates.clear();
        report.unknowns.clear();
        report.envelope.truncated = true;
        report.envelope.partial(
            UnknownReason::BudgetExhausted,
            "Candidate and unknown records withheld by response byte budget",
        );
        byte_budget(&report, limits.max_response_bytes as usize)?;
    }
    Ok(report)
}
fn variant(path: Vec<String>, enum_parts: &[String]) -> Option<String> {
    (path.len() == enum_parts.len() + 1 && path[..enum_parts.len()] == *enum_parts)
        .then(|| path.last().unwrap().clone())
}
fn assignment(expr: ast::Expr) -> Option<ast::BinExpr> {
    let expression = if let ast::Expr::BlockExpr(block) = expr {
        if block
            .syntax()
            .children()
            .any(|node| ast::Attr::can_cast(node.kind()))
            || block.label().is_some()
            || block.async_token().is_some()
            || block.unsafe_token().is_some()
            || block.const_token().is_some()
        {
            return None;
        }
        let list = block.stmt_list()?;
        let mut expressions = list
            .statements()
            .map(|stmt| match stmt {
                ast::Stmt::ExprStmt(stmt) => stmt.expr(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        expressions.extend(list.tail_expr());
        if expressions.len() != 1 {
            return None;
        }
        expressions.pop()?
    } else {
        expr
    };
    let binary = ast::BinExpr::cast(expression.syntax().clone())?;
    (binary.op_kind() == Some(ast::BinaryOp::Assignment { op: None })).then_some(binary)
}
fn guard_supported(guard: &ast::MatchGuard) -> bool {
    guard
        .syntax()
        .descendants()
        .filter_map(ast::Expr::cast)
        .all(|expr| match expr {
            ast::Expr::PathExpr(_) | ast::Expr::Literal(_) | ast::Expr::ParenExpr(_) => true,
            ast::Expr::PrefixExpr(_) => true,
            ast::Expr::BinExpr(binary) => {
                !matches!(binary.op_kind(), Some(ast::BinaryOp::Assignment { .. }))
            }
            _ => false,
        })
}
fn candidate_id(input: &str, candidate: &StateTransitionCandidate) -> String {
    digest(
        "atlas-state-transition-v1",
        &(
            input,
            &candidate.from_variant,
            &candidate.to_variant,
            &candidate.match_span,
            &candidate.arm_span,
            &candidate.assignment_span,
            &candidate.guard,
            &candidate.action,
        ),
    )
}

/// This is a syntactic candidate search, not exhaustiveness, name resolution, or a proof.
pub fn infer_state_machine(
    source: &SourceFile,
    definition: &Definition,
    enum_path: &str,
    state_place: &str,
    limits: StateMachineLimits,
    control: &AnalysisControl,
) -> Result<StateMachineInference> {
    limits.validate()?;
    byte_budget(definition, 65_536)?;
    let enum_parts = selection(enum_path)?;
    let state_parts = selection(state_place)?;
    if state_parts.len() != 1
        || matches!(state_parts[0].as_str(), "self" | "Self" | "crate" | "super")
    {
        return Err(invalid("state place must be a simple local identifier"));
    }
    if source.id != definition.file_id
        || definition.span.file_id != source.id
        || !matches!(definition.kind.as_str(), "function" | "method")
    {
        return Err(invalid(
            "selection must be a function in the exact source file",
        ));
    }
    let start = definition.span.start as usize;
    let end = definition.span.end as usize;
    let fragment = source
        .text
        .get(start..end)
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
    let input_digest = digest(
        "atlas-state-machine-input-v1",
        &(
            VERSION,
            fragment,
            definition,
            enum_path,
            state_place,
            limits,
        ),
    );
    let mut report = StateMachineInference {
        envelope: AnalysisEnvelope::new(
            VERSION,
            Coverage::partial(
                UnknownReason::UnsupportedConstruct,
                "Syntax candidates only; absent transitions are unknown, not impossible",
            ),
        ),
        input_digest,
        definition_id: definition.id.clone(),
        enum_path: enum_path.into(),
        state_place: state_place.into(),
        body_span: definition
            .body_span
            .clone()
            .unwrap_or_else(|| definition.span.clone()),
        visited_nodes: 0,
        candidates: vec![],
        unknowns: vec![],
    };
    report.envelope.assumptions = vec![
        "Selected enum and local paths are syntactic spellings, not resolved type or binding identities".into(),
        "Only direct top-level matches and single-assignment arms are inspected; guards and actions require human review".into(),
        "Candidates do not establish feasible execution, exhaustive states, complete transitions, or whole-repository behavior".into(),
        "Parsing uses Rust edition 2024; context edition and macro expansion are not available to this syntax-only API".into(),
    ];
    if control.stopped() {
        report.envelope.stop(control);
        return finish(report, limits);
    }
    if fragment.len() > limits.max_body_bytes as usize {
        report.envelope.truncated = true;
        report.envelope.partial(
            UnknownReason::BudgetExhausted,
            "Selected definition exceeds source byte budget",
        );
        return finish(report, limits);
    }
    if definition.cfg_status != CfgStatus::Active {
        unknown(
            &mut report,
            definition.span.clone(),
            "Selected definition is not known active in this context",
            limits,
        );
        return finish(report, limits);
    }
    let parsed = ast::SourceFile::parse(fragment, Edition::CURRENT);
    if !parsed.errors().is_empty() {
        unknown(
            &mut report,
            definition.span.clone(),
            "Selected function has parser errors",
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
    if function
        .name()
        .is_none_or(|name| ident(name.text()) != ident(&definition.name))
    {
        return Err(invalid("function name does not match selected definition"));
    }
    let body = function
        .syntax()
        .children()
        .find_map(ast::BlockExpr::cast)
        .ok_or_else(|| invalid("selected function has no body"))?;
    report.body_span = span(body.syntax(), definition);
    if definition
        .body_span
        .as_ref()
        .is_some_and(|expected| expected != &report.body_span)
    {
        return Err(invalid("body span does not match parsed function body"));
    }
    let statements = body
        .stmt_list()
        .ok_or_else(|| invalid("selected function has no statement list"))?;
    let state = &state_parts[0];
    let mut bindings = vec![];
    let mut references = vec![];
    let mut selected_paths = vec![];
    let mut unsafe_binding = false;
    // Finish the bounded binding prepass before emitting candidates: a later shadow or macro
    // must not retroactively invalidate an already-emitted local-identity assumption.
    for node in function.syntax().descendants() {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        if report.visited_nodes == limits.max_nodes {
            report.envelope.truncated = true;
            report.envelope.partial(
                UnknownReason::BudgetExhausted,
                "Node budget exhausted before binding checks completed",
            );
            return finish(report, limits);
        }
        report.visited_nodes += 1;
        if let Some(binding) = ast::IdentPat::cast(node.clone())
            && binding
                .name()
                .is_some_and(|name| ident(name.text()) == state)
        {
            bindings.push(binding);
        }
        if ast::MacroCall::can_cast(node.kind()) {
            unsafe_binding = true;
            unknown(
                &mut report,
                span(&node, definition),
                "Macros may introduce or mutate bindings; this function is unsupported",
                limits,
            );
        }
        if ast::Attr::can_cast(node.kind()) {
            unsafe_binding = true;
            unknown(
                &mut report,
                span(&node, definition),
                "Attributes may rewrite or conditionally remove syntax; this function is unsupported",
                limits,
            );
        }
        if ast::RefExpr::can_cast(node.kind()) {
            references.push(span(&node, definition));
        }
        if let Some(path) = ast::PathExpr::cast(node.clone())
            && path.path().and_then(path_parts).as_ref() == Some(&state_parts)
        {
            selected_paths.push(span(&node, definition));
        }
        if let Some(local) = ast::LetStmt::cast(node.clone())
            && local.initializer().and_then(expr_path).as_ref() == Some(&state_parts)
        {
            unsafe_binding = true;
            unknown(
                &mut report,
                span(&node, definition),
                "Local copies or aliases of the selected state are unsupported",
                limits,
            );
        }
    }
    // Interval lookup avoids walking the same subtree for every nested reference.
    selected_paths.sort_by_key(|path| (path.start, path.end));
    for reference in references {
        if control.stopped() {
            report.envelope.stop(control);
            return finish(report, limits);
        }
        let index = selected_paths.partition_point(|path| path.start < reference.start);
        if selected_paths
            .get(index)
            .is_some_and(|path| path.end <= reference.end)
        {
            unsafe_binding = true;
            unknown(
                &mut report,
                reference,
                "References or aliases of the selected local are unsupported",
                limits,
            );
        }
    }
    if bindings.len() != 1 {
        let body_span = report.body_span.clone();
        unknown(
            &mut report,
            body_span,
            "Selected local must have exactly one explicit binding; missing or shadowed bindings are unsupported",
            limits,
        );
        return finish(report, limits);
    }
    let binding = &bindings[0];
    let parent = binding.syntax().parent();
    let direct_parameter = parent.clone().and_then(ast::Param::cast).filter(|param| {
        param.syntax().parent().and_then(|p| p.parent()).as_ref() == Some(function.syntax())
    });
    let direct_local = parent
        .and_then(ast::LetStmt::cast)
        .filter(|local| local.syntax().parent().as_ref() == Some(statements.syntax()));
    if binding.ref_token().is_some()
        || binding.at_token().is_some()
        || binding.pat().is_some()
        || (direct_parameter.is_none() && direct_local.is_none())
        || direct_parameter
            .as_ref()
            .and_then(|param| param.ty())
            .is_some_and(|ty| matches!(ty, ast::Type::RefType(_)))
        || direct_local
            .as_ref()
            .and_then(|local| local.ty())
            .is_some_and(|ty| matches!(ty, ast::Type::RefType(_)))
    {
        unsafe_binding = true;
        unknown(
            &mut report,
            span(binding.syntax(), definition),
            "Selected binding must be a direct, non-reference parameter or top-level local",
            limits,
        );
    }
    if unsafe_binding {
        return finish(report, limits);
    }
    let binding_end = u32::from(binding.syntax().text_range().end());
    let mut expressions = vec![];
    for statement in statements.statements() {
        match statement {
            ast::Stmt::ExprStmt(statement) => expressions.extend(statement.expr()),
            ast::Stmt::LetStmt(local) if direct_local.as_ref() == Some(&local) => {}
            other => unknown(
                &mut report,
                span(other.syntax(), definition),
                "Non-match top-level statement is outside the transition model",
                limits,
            ),
        }
    }
    expressions.extend(statements.tail_expr());
    for expression in expressions {
        if control.stopped() {
            report.envelope.stop(control);
            break;
        }
        let Some(matched) = ast::MatchExpr::cast(expression.syntax().clone()) else {
            unknown(
                &mut report,
                span(expression.syntax(), definition),
                "Only direct top-level match expressions are supported",
                limits,
            );
            continue;
        };
        if matched.expr().and_then(expr_path).as_ref() != Some(&state_parts)
            || u32::from(matched.syntax().text_range().start()) < binding_end
        {
            unknown(
                &mut report,
                span(matched.syntax(), definition),
                "Match scrutinee is not the selected bound local",
                limits,
            );
            continue;
        }
        let Some(arms) = matched.match_arm_list() else {
            continue;
        };
        for arm in arms.arms() {
            if control.stopped() {
                report.envelope.stop(control);
                break;
            }
            if report.candidates.len() == limits.max_transitions as usize {
                report.envelope.truncated = true;
                report.envelope.partial(
                    UnknownReason::BudgetExhausted,
                    "Transition candidate budget exhausted",
                );
                return finish(report, limits);
            }
            let from = arm
                .pat()
                .and_then(|pat| ast::PathPat::cast(pat.syntax().clone()))
                .and_then(|pat| pat.path())
                .and_then(path_parts)
                .and_then(|path| variant(path, &enum_parts));
            let action = arm.expr();
            let assigned = action.clone().and_then(assignment);
            let to = assigned
                .as_ref()
                .and_then(|binary| binary.rhs())
                .and_then(expr_path)
                .and_then(|path| variant(path, &enum_parts));
            let lhs_matches = assigned
                .as_ref()
                .and_then(|binary| binary.lhs())
                .and_then(expr_path)
                .as_ref()
                == Some(&state_parts);
            let guard = arm.guard();
            let attributed = arm
                .syntax()
                .descendants()
                .any(|node| ast::Attr::can_cast(node.kind()));
            if from.is_none()
                || to.is_none()
                || !lhs_matches
                || attributed
                || guard.as_ref().is_some_and(|guard| !guard_supported(guard))
            {
                unknown(
                    &mut report,
                    span(arm.syntax(), definition),
                    "Arm requires an explicit enum variant and one direct assignment; compound patterns or actions, member places, attributes and explicit effectful guard syntax are unsupported",
                    limits,
                );
                continue;
            }
            let action = action.and_then(|expr| syntax(expr.syntax(), definition));
            let guard_syntax = guard
                .as_ref()
                .and_then(|guard| syntax(guard.syntax(), definition));
            if action.is_none() || (guard.is_some() && guard_syntax.is_none()) {
                unknown(
                    &mut report,
                    span(arm.syntax(), definition),
                    "Guard or action exceeds the exact syntax excerpt budget",
                    limits,
                );
                continue;
            }
            let mut candidate = StateTransitionCandidate {
                id: String::new(),
                from_variant: from.unwrap(),
                to_variant: to.unwrap(),
                match_span: span(matched.syntax(), definition),
                arm_span: span(arm.syntax(), definition),
                assignment_span: span(assigned.unwrap().syntax(), definition),
                guard: guard_syntax,
                action: action.unwrap(),
            };
            candidate.id = candidate_id(&report.input_digest, &candidate);
            report.candidates.push(candidate);
        }
    }
    finish(report, limits)
}

/// Review acceptance is a declaration about one exact candidate, never a proof upgrade.
pub fn validate_state_review(
    inference: &StateMachineInference,
    review: &StateTransitionReview,
) -> Result<()> {
    if review.reviewer.trim().is_empty()
        || review.reviewer.len() > 128
        || review.reviewer.chars().any(char::is_control)
    {
        return Err(invalid(
            "reviewer must be nonempty, at most 128 bytes, and contain no control characters",
        ));
    }
    if review.note.len() > 4096
        || review
            .note
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            "review note exceeds 4096 bytes or contains unsupported control characters",
        ));
    }
    if review.input_digest != inference.input_digest {
        return Err(invalid("review inference digest does not match"));
    }
    let candidate = inference
        .candidates
        .iter()
        .find(|candidate| candidate.id == review.candidate_id)
        .ok_or_else(|| invalid("review candidate does not exist in this inference"))?;
    if candidate_id(&inference.input_digest, candidate) != candidate.id {
        return Err(invalid("candidate fields do not match its digest"));
    }
    Ok(())
}
