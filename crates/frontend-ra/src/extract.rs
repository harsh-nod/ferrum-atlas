use crate::{AnalysisLevel, PRODUCER, database};
use anyhow::{Result, ensure};
use atlas_model::*;
use ra_ap_hir::{Function, ModuleDef, PathResolution, Semantics};
use ra_ap_ide_db::RootDatabase;
use ra_ap_syntax::{
    AstNode, SyntaxKind, SyntaxNode, TextRange,
    ast::{self, HasName},
};
use ra_ap_vfs::FileId as RaFileId;
use std::collections::HashMap;

struct Item {
    definition: Definition,
    node: SyntaxNode,
    body: Option<SyntaxNode>,
    function: Option<Function>,
}

pub(super) fn analyze(
    source: SourceSnapshot,
    context: BuildContext,
    level: AnalysisLevel,
) -> Result<FactBatch> {
    let db = database::load(&source, &context)?;
    ra_ap_hir::attach_db(&db, || extract(source, context, level, &db))
}

fn extract(
    source: SourceSnapshot,
    context: BuildContext,
    level: AnalysisLevel,
    db: &RootDatabase,
) -> Result<FactBatch> {
    let sema = Semantics::new(db);
    let mut coverage = context.coverage.clone();
    add_gap(
        &mut coverage,
        UnknownReason::UnsupportedConstruct,
        "Source-level extraction excludes implicit drops, compiler lowering, macro expansions, and runtime dispatch targets.",
    );
    if level == AnalysisLevel::Syntax {
        add_gap(
            &mut coverage,
            UnknownReason::SyntaxOnly,
            "Syntax analysis makes no semantic target claims.",
        );
    }
    let mut diagnostics = Vec::new();
    let mut items = Vec::<Item>::new();
    let mut item_indices = HashMap::<SyntaxNode, usize>::new();
    let mut has_cfg = false;
    for (index, file) in source.files.iter().enumerate() {
        ensure!(
            file.text.len() <= 4 * 1024 * 1024,
            "analysis file byte budget exceeded"
        );
        let syntax = sema.parse_guess_edition(RaFileId::from_raw(index as u32));
        for error in syntax
            .syntax()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::ERROR)
        {
            diagnostics.push(Diagnostic {
                path: Some(file.path.clone()),
                severity: "error".into(),
                message: format!(
                    "syntax error at byte {}",
                    u32::from(error.text_range().start())
                ),
            });
        }
        // The parse API reports missing tokens separately from ERROR nodes.
        let parsed = ra_ap_syntax::SourceFile::parse(&file.text, ra_ap_syntax::Edition::CURRENT);
        for error in parsed.errors() {
            diagnostics.push(Diagnostic {
                path: Some(file.path.clone()),
                severity: "error".into(),
                message: error.to_string(),
            });
        }
        for attr in syntax.syntax().descendants().filter_map(ast::Attr::cast) {
            let path = attr
                .simple_name()
                .map(|name| name.to_string())
                .unwrap_or_default();
            if path == "cfg" || path == "cfg_attr" {
                has_cfg = true;
            }
            if path == "derive"
                || !matches!(
                    path.as_str(),
                    "cfg"
                        | "cfg_attr"
                        | "doc"
                        | "allow"
                        | "warn"
                        | "deny"
                        | "forbid"
                        | "inline"
                        | "repr"
                        | "test"
                        | "must_use"
                        | "no_mangle"
                        | "export_name"
                        | "link"
                        | "path"
                        | "cold"
                        | "deprecated"
                        | "no_std"
                        | "no_main"
                        | "non_exhaustive"
                )
            {
                add_gap(
                    &mut coverage,
                    UnknownReason::MacroUnavailable,
                    "Attribute and derive macro output is not captured.",
                );
            }
        }
        for node in syntax.syntax().descendants() {
            if ast::MacroCall::can_cast(node.kind()) {
                add_gap(
                    &mut coverage,
                    UnknownReason::MacroUnavailable,
                    "Macro invocations are recorded without extracting expanded output.",
                );
            }
            let Some((kind, name, body)) = item_parts(&node) else {
                continue;
            };
            ensure!(items.len() < 200_000, "definition budget exceeded");
            let parent = node
                .ancestors()
                .skip(1)
                .find_map(|ancestor| item_indices.get(&ancestor))
                .map(|index| &items[*index]);
            let owner = parent
                .map(|item| item.definition.qualified_name.clone())
                .unwrap_or_else(|| file.path.trim_end_matches(".rs").replace('/', "::"));
            let qualified_name = format!("{owner}::{name}");
            let range = node.text_range();
            let signature_end = body
                .as_ref()
                .map(|body| body.text_range().start())
                .unwrap_or(range.end());
            let signature = file.text
                [u32::from(range.start()) as usize..u32::from(signature_end) as usize]
                .trim()
                .to_owned();
            let body_text = body.as_ref().map(ToString::to_string).unwrap_or_default();
            let cfg = node
                .ancestors()
                .flat_map(|ancestor| {
                    ancestor
                        .children()
                        .filter_map(ast::Attr::cast)
                        .collect::<Vec<_>>()
                })
                .filter(|attr| {
                    attr.simple_name()
                        .is_some_and(|name| matches!(name.as_str(), "cfg" | "cfg_attr"))
                })
                .map(|attr| attr.to_string())
                .collect();
            let id = DefinitionId(digest(
                "definition",
                &(
                    &source.id,
                    &context.id,
                    &file.path,
                    &qualified_name,
                    kind,
                    u32::from(range.start()),
                ),
            ));
            let function = ast::Fn::cast(node.clone()).and_then(|function| sema.to_def(&function));
            let definition = Definition {
                id,
                context_id: context.id.clone(),
                file_id: file.id.clone(),
                name,
                qualified_name,
                kind: kind.into(),
                parent_id: parent.map(|item| item.definition.id.clone()),
                signature_hash: digest("signature", &signature),
                body_hash: digest("body", &body_text),
                signature,
                span: span(file, range),
                body_span: body.as_ref().map(|body| span(file, body.text_range())),
                cfg,
                visibility: node
                    .children()
                    .find_map(ast::Visibility::cast)
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "private".into()),
                metrics: Metrics::default(),
            };
            item_indices.insert(node.clone(), items.len());
            items.push(Item {
                definition,
                node,
                body,
                function,
            });
        }
    }
    if has_cfg {
        add_gap(
            &mut coverage,
            UnknownReason::CfgUnknown,
            "Raw cfg is retained. Conditional source requires a fully resolved per-crate configuration; semantic call targets are withheld for this snapshot.",
        );
    }
    if !diagnostics.is_empty() {
        add_gap(
            &mut coverage,
            UnknownReason::AnalysisFailed,
            "Broken source remains browsable; syntax errors can limit resolution.",
        );
    }
    let function_ids = items
        .iter()
        .filter_map(|item| {
            item.function.map(|function| {
                let target = if item
                    .node
                    .ancestors()
                    .any(|node| ast::Trait::can_cast(node.kind()))
                {
                    Err(UnknownReason::IndirectTargetUnknown)
                } else if item.body.is_none() {
                    Err(UnknownReason::ExternalBoundary)
                } else {
                    Ok(item.definition.id.clone())
                };
                (function, target)
            })
        })
        .collect::<HashMap<_, _>>();
    let files_by_id = source
        .files
        .iter()
        .map(|file| (&file.id, file))
        .collect::<HashMap<_, _>>();
    let mut relations = Vec::new();
    let mut evidence = Vec::new();
    let mut flows = Vec::new();
    for item in &mut items {
        let Some(body) = &item.body else {
            continue;
        };
        if !matches!(item.definition.kind.as_str(), "function" | "closure") {
            continue;
        }
        let file = files_by_id[&item.definition.file_id];
        let mut points = vec![FlowPoint {
            kind: "entry".into(),
            label: item.definition.name.clone(),
            span: span(file, body.text_range()),
        }];
        let mut metrics = Metrics {
            lines: body.to_string().lines().count() as u32,
            ..Metrics::default()
        };
        for node in body
            .descendants()
            .filter(|node| belongs_to_body(node, &item.node))
        {
            let flow_kind = match node.kind() {
                SyntaxKind::IF_EXPR
                | SyntaxKind::MATCH_EXPR
                | SyntaxKind::FOR_EXPR
                | SyntaxKind::WHILE_EXPR
                | SyntaxKind::LOOP_EXPR => {
                    metrics.branches += 1;
                    Some("branch")
                }
                SyntaxKind::RETURN_EXPR => {
                    metrics.returns += 1;
                    Some("return")
                }
                SyntaxKind::AWAIT_EXPR => {
                    metrics.awaits += 1;
                    Some("await")
                }
                SyntaxKind::TRY_EXPR => Some("error_propagation"),
                SyntaxKind::BREAK_EXPR => Some("break"),
                SyntaxKind::CONTINUE_EXPR => Some("continue"),
                SyntaxKind::BLOCK_EXPR
                    if ast::BlockExpr::cast(node.clone())
                        .is_some_and(|block| block.unsafe_token().is_some()) =>
                {
                    metrics.unsafe_blocks += 1;
                    Some("unsafe")
                }
                _ => None,
            };
            if let Some(kind) = flow_kind {
                points.push(FlowPoint {
                    kind: kind.into(),
                    label: short_label(&node.to_string()),
                    span: span(file, node.text_range()),
                });
            }
            let call = ast::CallExpr::cast(node.clone());
            let method = ast::MethodCallExpr::cast(node.clone());
            let mac = ast::MacroCall::cast(node.clone());
            if call.is_none() && method.is_none() && mac.is_none() {
                continue;
            }
            ensure!(relations.len() < 500_000, "relation budget exceeded");
            let label = call
                .as_ref()
                .and_then(ast::CallExpr::expr)
                .map(|expr| expr.to_string())
                .or_else(|| {
                    method
                        .as_ref()
                        .and_then(ast::MethodCallExpr::name_ref)
                        .map(|name| name.to_string())
                })
                .or_else(|| {
                    mac.as_ref()
                        .and_then(ast::MacroCall::path)
                        .map(|path| format!("{path}!"))
                })
                .unwrap_or_else(|| "<incomplete call>".into());
            let target = if mac.is_some() {
                unknown(UnknownReason::MacroUnavailable, &label)
            } else if level == AnalysisLevel::Syntax {
                unknown(UnknownReason::SyntaxOnly, &label)
            } else if has_cfg {
                unknown(UnknownReason::CfgUnknown, &label)
            } else {
                resolve(&sema, call, method, &function_ids, &label)
            };
            let basis = if matches!(target, Target::Resolved { .. }) {
                "resolved"
            } else {
                "source"
            };
            let limitations = match &target {
                Target::Unknown { reason, .. } => {
                    add_gap(
                        &mut coverage,
                        reason.clone(),
                        &format!("Unresolved {label}: {reason:?}"),
                    );
                    vec![format!("{reason:?}")]
                }
                _ => vec![],
            };
            let call_span = span(file, node.text_range());
            let id = RelationId(digest(
                "relation",
                &(&item.definition.id, &target, &call_span),
            ));
            let evidence_id = EvidenceId(digest("evidence", &(&id, basis, PRODUCER)));
            evidence.push(Evidence { id: evidence_id.clone(), basis: basis.into(), producer: PRODUCER.into(), inputs: vec![source.id.0.clone(), context.id.0.clone(), file.content_hash.clone()], assumptions: vec!["Captured source and manifest-only crate graph; no sysroot or executable producers.".into()], limitations });
            relations.push(Relation {
                id,
                source: item.definition.id.clone(),
                target,
                kind: if mac.is_some() {
                    "macro_invocation"
                } else {
                    "call"
                }
                .into(),
                span: call_span,
                evidence_id,
            });
        }
        item.definition.metrics = metrics;
        points.sort_by_key(|point| (point.span.start, point.kind.clone()));
        flows.push(FunctionFlow { definition_id: item.definition.id.clone(), phase: "source".into(), points,
            coverage: Coverage::partial(UnknownReason::UnsupportedConstruct, "Source structure only; not a compiler CFG. Implicit drop, unwind, and macro-generated operations are unavailable.") });
    }
    let mut definitions = items
        .into_iter()
        .map(|item| item.definition)
        .collect::<Vec<_>>();
    definitions.sort_by(|a, b| a.id.cmp(&b.id));
    relations.sort_by(|a, b| a.id.cmp(&b.id));
    evidence.sort_by(|a, b| a.id.cmp(&b.id));
    flows.sort_by(|a, b| a.definition_id.cmp(&b.definition_id));
    coverage.reasons.sort_by(|a, b| a.reason.cmp(&b.reason));
    coverage.limitations.sort();
    coverage.limitations.dedup();
    diagnostics.sort_by(|a, b| (&a.path, &a.message).cmp(&(&b.path, &b.message)));
    diagnostics.dedup();
    Ok(FactBatch {
        schema_version: SCHEMA_VERSION,
        source,
        context,
        producer: PRODUCER.into(),
        definitions,
        relations,
        evidence,
        flows,
        coverage,
        diagnostics,
    })
}

fn resolve(
    sema: &Semantics<'_, RootDatabase>,
    call: Option<ast::CallExpr>,
    method: Option<ast::MethodCallExpr>,
    functions: &HashMap<Function, Result<DefinitionId, UnknownReason>>,
    label: &str,
) -> Target {
    let function = if let Some(method) = method {
        sema.resolve_method_call(&method)
    } else if let Some(ast::Expr::PathExpr(expr)) = call.and_then(|call| call.expr()) {
        match expr.path().and_then(|path| sema.resolve_path(&path)) {
            Some(PathResolution::Def(ModuleDef::Function(function))) => Some(function),
            Some(PathResolution::Local(_)) => {
                return unknown(UnknownReason::IndirectTargetUnknown, label);
            }
            _ => None,
        }
    } else {
        return unknown(UnknownReason::IndirectTargetUnknown, label);
    };
    match function.and_then(|function| functions.get(&function)) {
        Some(Ok(id)) => Target::Resolved { id: id.clone() },
        Some(Err(reason)) => unknown(reason.clone(), label),
        None => unknown(UnknownReason::MissingDependency, label),
    }
}

fn unknown(reason: UnknownReason, label: &str) -> Target {
    Target::Unknown {
        reason,
        label: label.into(),
    }
}
fn span(file: &SourceFile, range: TextRange) -> Span {
    Span {
        file_id: file.id.clone(),
        start: range.start().into(),
        end: range.end().into(),
    }
}
fn short_label(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}
fn add_gap(coverage: &mut Coverage, reason: UnknownReason, limitation: &str) {
    coverage.status = Status::Partial;
    if let Some(count) = coverage
        .reasons
        .iter_mut()
        .find(|entry| entry.reason == reason)
    {
        count.count += 1;
    } else {
        coverage.reasons.push(ReasonCount { reason, count: 1 });
    }
    if !coverage.limitations.iter().any(|value| value == limitation) {
        coverage.limitations.push(limitation.into());
    }
}

fn belongs_to_body(node: &SyntaxNode, owner: &SyntaxNode) -> bool {
    node.ancestors()
        .skip(1)
        .find(|ancestor| {
            matches!(
                ancestor.kind(),
                SyntaxKind::FN | SyntaxKind::CLOSURE_EXPR | SyntaxKind::CONST | SyntaxKind::STATIC
            )
        })
        .is_some_and(|ancestor| ancestor == *owner)
}

fn item_parts(node: &SyntaxNode) -> Option<(&'static str, String, Option<SyntaxNode>)> {
    macro_rules! named {
        ($ty:ident, $kind:literal) => {
            if let Some(item) = ast::$ty::cast(node.clone()) {
                return Some(($kind, item.name()?.to_string(), None));
            }
        };
    }
    if let Some(item) = ast::Fn::cast(node.clone()) {
        return Some((
            "function",
            item.name()?.to_string(),
            item.body().map(|body| body.syntax().clone()),
        ));
    }
    if let Some(item) = ast::ClosureExpr::cast(node.clone()) {
        return Some((
            "closure",
            format!("closure@{}", u32::from(node.text_range().start())),
            item.body().map(|body| body.syntax().clone()),
        ));
    }
    if let Some(item) = ast::Impl::cast(node.clone()) {
        return Some((
            "impl",
            format!(
                "impl {}",
                item.self_ty().map(|ty| ty.to_string()).unwrap_or_default()
            ),
            None,
        ));
    }
    named!(Module, "module");
    named!(Struct, "struct");
    named!(Enum, "enum");
    named!(Union, "union");
    named!(Trait, "trait");
    named!(TypeAlias, "type_alias");
    named!(Const, "const");
    named!(Static, "static");
    named!(MacroRules, "macro");
    named!(Variant, "variant");
    None
}
