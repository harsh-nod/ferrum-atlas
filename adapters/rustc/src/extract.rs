use crate::{capture::Capture, compiler::*};
use anyhow::{Result, ensure};
use rustc_hir::def::DefKind;
use rustc_middle::{
    mir::{
        self,
        visit::{
            MutatingUseContext as Mut, NonMutatingUseContext as Read, NonUseContext as Non,
            PlaceContext, Visitor,
        },
    },
    ty::{self, TyCtxt},
};
use rustc_span::{FileName, Span};

fn def_path(tcx: TyCtxt<'_>, id: rustc_span::def_id::DefId) -> String {
    if id.is_local() {
        format!("{}::{}", tcx.crate_name(id.krate), tcx.def_path_str(id))
    } else {
        tcx.def_path_str(id)
    }
}

const MAX_BODIES: usize = 2000;
const MAX_BLOCKS: usize = 20_000;
const MAX_STATEMENTS: usize = 200_000;
const MAX_LOCALS: usize = 100_000;

pub fn extract(tcx: TyCtxt<'_>, capture: &Capture) -> Result<Vec<CompilerBody>> {
    let mut owners: Vec<_> = tcx
        .mir_keys(())
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                tcx.def_kind(*id),
                DefKind::Fn | DefKind::AssocFn | DefKind::Closure | DefKind::SyntheticCoroutineBody
            )
        })
        .collect();
    ensure!(owners.len() <= MAX_BODIES, "compiler body budget exceeded");
    owners.sort_by_key(|id| tcx.def_path_str(id.to_def_id()));
    let mut output = Vec::new();
    let mut blocks = 0;
    let mut statements = 0;
    let mut locals = 0;
    for owner in owners {
        let body = tcx.optimized_mir(owner.to_def_id());
        ensure!(
            body.phase == mir::MirPhase::Runtime(mir::RuntimePhase::Optimized),
            "unexpected MIR extraction phase"
        );
        blocks += body.basic_blocks.len();
        statements += body
            .basic_blocks
            .iter()
            .map(|block| block.statements.len())
            .sum::<usize>();
        locals += body.local_decls.len();
        ensure!(
            blocks <= MAX_BLOCKS && statements <= MAX_STATEMENTS && locals <= MAX_LOCALS,
            "compiler fact budget exceeded"
        );
        let names = |local| {
            let mut names: Vec<_> = body
                .var_debug_info
                .iter()
                .filter_map(|info| match info.value {
                    mir::VarDebugInfoContents::Place(place)
                        if place.local == local && place.projection.is_empty() =>
                    {
                        Some(info.name.to_string())
                    }
                    _ => None,
                })
                .collect();
            names.sort();
            names.dedup();
            names
        };
        output.push(CompilerBody {
            body_id: String::new(),
            def_path: def_path(tcx, owner.to_def_id()),
            kind: match tcx.def_kind(owner) {
                DefKind::Fn => "function",
                DefKind::AssocFn => "associated_function",
                DefKind::Closure => "closure_or_coroutine",
                _ => "synthetic_coroutine",
            }
            .into(),
            span: mapping(tcx, capture, tcx.def_span(owner)),
            argument_count: body.arg_count as u32,
            locals: body
                .local_decls
                .iter_enumerated()
                .map(|(local, decl)| CompilerLocal {
                    index: local.as_u32(),
                    role: if local.as_u32() == 0 {
                        "return"
                    } else if local.as_usize() <= body.arg_count {
                        "argument"
                    } else {
                        "temporary"
                    }
                    .into(),
                    names: names(local),
                    type_display: decl.ty.to_string(),
                    source_scope: decl.source_info.scope.as_u32(),
                    span: mapping(tcx, capture, decl.source_info.span),
                })
                .collect(),
            source_scopes: body
                .source_scopes
                .iter_enumerated()
                .map(|(scope, data)| CompilerSourceScope {
                    index: scope.as_u32(),
                    parent: data.parent_scope.map(|parent| parent.as_u32()),
                    span: mapping(tcx, capture, data.span),
                    inlined_def_path: data
                        .inlined
                        .map(|(instance, _)| def_path(tcx, instance.def_id())),
                })
                .collect(),
            blocks: body
                .basic_blocks
                .iter_enumerated()
                .map(|(block, data)| CompilerBlock {
                    index: block.as_u32(),
                    is_cleanup: data.is_cleanup,
                    statements: data
                        .statements
                        .iter()
                        .enumerate()
                        .map(|(index, statement)| {
                            let mut effects = Effects::default();
                            effects.visit_statement(
                                statement,
                                mir::Location {
                                    block,
                                    statement_index: index,
                                },
                            );
                            if matches!(statement.kind, mir::StatementKind::Intrinsic(_)) {
                                effects.unknown(CompilerUnknownEffect::Intrinsic);
                            }
                            CompilerStatement {
                                index: index as u32,
                                kind: statement_kind(&statement.kind).into(),
                                source_scope: statement.source_info.scope.as_u32(),
                                span: mapping(tcx, capture, statement.source_info.span),
                                locals: effects.finish(),
                            }
                        })
                        .collect(),
                    terminator: terminator(tcx, capture, body, block),
                })
                .collect(),
        });
    }
    Ok(output)
}

fn mapping(tcx: TyCtxt<'_>, capture: &Capture, span: Span) -> CompilerSourceMapping {
    let unavailable = |reason: &str| CompilerSourceMapping::Unavailable {
        reason: reason.into(),
    };
    if span.is_dummy() {
        return unavailable("compiler-generated span");
    }
    if span.from_expansion() {
        return unavailable("macro expansion mapping is not a direct source span");
    }
    let file = tcx.sess.source_map().lookup_source_file(span.lo());
    if file.is_imported() {
        return unavailable("external source not captured");
    }
    if span.hi() > file.end_position() {
        return unavailable("span crosses source files");
    }
    let FileName::Real(name) = &file.name else {
        return unavailable("virtual source file");
    };
    let Some(path) = name
        .local_path()
        .and_then(|path| capture.relative_path(path))
    else {
        return unavailable("source file is not in captured inputs");
    };
    CompilerSourceMapping::Exact {
        path,
        start_byte: file.original_relative_byte_pos(span.lo()).0,
        end_byte: file.original_relative_byte_pos(span.hi()).0,
    }
}

fn statement_kind(kind: &mir::StatementKind<'_>) -> &'static str {
    match kind {
        mir::StatementKind::Assign(_) => "assign",
        mir::StatementKind::FakeRead(_) => "fake_read",
        mir::StatementKind::SetDiscriminant { .. } => "set_discriminant",
        mir::StatementKind::StorageLive(_) => "storage_live",
        mir::StatementKind::StorageDead(_) => "storage_dead",
        mir::StatementKind::Retag(..) => "retag",
        mir::StatementKind::PlaceMention(_) => "place_mention",
        mir::StatementKind::AscribeUserType(..) => "ascribe_user_type",
        mir::StatementKind::Coverage(_) => "coverage",
        mir::StatementKind::Intrinsic(_) => "intrinsic",
        mir::StatementKind::ConstEvalCounter => "const_eval_counter",
        mir::StatementKind::Nop => "nop",
        mir::StatementKind::BackwardIncompatibleDropHint { .. } => {
            "backward_incompatible_drop_hint"
        }
    }
}

fn terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    capture: &Capture,
    body: &mir::Body<'tcx>,
    block: mir::BasicBlock,
) -> CompilerTerminator {
    use mir::TerminatorKind as T;
    let data = &body.basic_blocks[block];
    let term = data.terminator();
    let mut effects = Effects::default();
    effects.visit_terminator(
        term,
        mir::Location {
            block,
            statement_index: data.statements.len(),
        },
    );
    let normal_return_defs = std::mem::take(&mut effects.call_defs);
    let mut output = CompilerTerminator {
        kind: String::new(),
        source_scope: term.source_info.scope.as_u32(),
        span: mapping(tcx, capture, term.source_info.span),
        locals: CompilerLocalEffects::default(),
        normal_return_defs,
        successors: Vec::new(),
        unwind: None,
        call_target: None,
        assert_expected: None,
    };
    output.kind = match &term.kind {
        T::Goto { target } => {
            edge(&mut output, *target, CompilerEdgeKind::Normal);
            "goto"
        }
        T::SwitchInt { targets, .. } => {
            for (value, target) in targets.iter() {
                output.successors.push(CompilerSuccessor {
                    target: target.as_u32(),
                    kind: CompilerEdgeKind::SwitchValue,
                    switch_value: Some(value.to_string()),
                });
            }
            edge(
                &mut output,
                targets.otherwise(),
                CompilerEdgeKind::Otherwise,
            );
            "switch_int"
        }
        T::Return => {
            effects.locals.uses.push(0);
            "return"
        }
        T::UnwindResume => "unwind_resume",
        T::UnwindTerminate(reason) => {
            output.unwind = Some(CompilerUnwind::Terminate {
                reason: terminate_reason(*reason).into(),
            });
            "unwind_terminate"
        }
        T::Unreachable => "unreachable",
        T::Drop {
            target,
            unwind,
            drop,
            ..
        } => {
            effects.unknown(CompilerUnknownEffect::Drop);
            edge(&mut output, *target, CompilerEdgeKind::Normal);
            if let Some(target) = drop {
                edge(&mut output, *target, CompilerEdgeKind::CoroutineDrop);
            }
            add_unwind(&mut output, *unwind);
            "drop"
        }
        T::Call {
            func,
            target,
            unwind,
            ..
        } => {
            effects.unknown(CompilerUnknownEffect::Call);
            output.call_target = Some(call_target(tcx, body, func, &mut effects));
            if let Some(target) = target {
                edge(&mut output, *target, CompilerEdgeKind::Normal);
            } else {
                output.normal_return_defs.clear();
            }
            add_unwind(&mut output, *unwind);
            "call"
        }
        T::TailCall { func, .. } => {
            effects.unknown(CompilerUnknownEffect::Call);
            output.call_target = Some(call_target(tcx, body, func, &mut effects));
            "tail_call"
        }
        T::Assert {
            expected,
            target,
            unwind,
            ..
        } => {
            output.assert_expected = Some(*expected);
            edge(&mut output, *target, CompilerEdgeKind::Normal);
            add_unwind(&mut output, *unwind);
            "assert"
        }
        T::Yield { resume, drop, .. } => {
            edge(&mut output, *resume, CompilerEdgeKind::Resume);
            if let Some(target) = drop {
                edge(&mut output, *target, CompilerEdgeKind::CoroutineDrop);
            }
            "yield"
        }
        T::CoroutineDrop => "coroutine_drop",
        T::FalseEdge {
            real_target,
            imaginary_target,
        } => {
            edge(&mut output, *real_target, CompilerEdgeKind::Normal);
            edge(&mut output, *imaginary_target, CompilerEdgeKind::Imaginary);
            "false_edge"
        }
        T::FalseUnwind {
            real_target,
            unwind,
        } => {
            edge(&mut output, *real_target, CompilerEdgeKind::Normal);
            add_unwind(&mut output, *unwind);
            "false_unwind"
        }
        T::InlineAsm {
            targets, unwind, ..
        } => {
            effects.unknown(CompilerUnknownEffect::InlineAssembly);
            for target in targets {
                edge(&mut output, *target, CompilerEdgeKind::Normal);
            }
            add_unwind(&mut output, *unwind);
            "inline_asm"
        }
    }
    .into();
    output.locals = effects.finish();
    output.normal_return_defs.sort_unstable();
    output.normal_return_defs.dedup();
    output
}

fn call_target<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mir::Body<'tcx>,
    func: &mir::Operand<'tcx>,
    effects: &mut Effects,
) -> CompilerCallTarget {
    match func.ty(body, tcx).kind() {
        ty::FnDef(id, _) => CompilerCallTarget::FunctionDefinition {
            def_path: def_path(tcx, *id),
            is_local: id.is_local(),
        },
        _ => {
            effects.unknown(CompilerUnknownEffect::IndirectCall);
            CompilerCallTarget::Indirect {
                reason: "function pointer or other indirect callable; runtime target unavailable"
                    .into(),
            }
        }
    }
}

fn edge(output: &mut CompilerTerminator, target: mir::BasicBlock, kind: CompilerEdgeKind) {
    output.successors.push(CompilerSuccessor {
        target: target.as_u32(),
        kind,
        switch_value: None,
    });
}

fn add_unwind(output: &mut CompilerTerminator, unwind: mir::UnwindAction) {
    output.unwind = Some(match unwind {
        mir::UnwindAction::Continue => CompilerUnwind::Continue,
        mir::UnwindAction::Unreachable => CompilerUnwind::Unreachable,
        mir::UnwindAction::Terminate(reason) => CompilerUnwind::Terminate {
            reason: terminate_reason(reason).into(),
        },
        mir::UnwindAction::Cleanup(target) => {
            edge(output, target, CompilerEdgeKind::Unwind);
            CompilerUnwind::Cleanup {
                target: target.as_u32(),
            }
        }
    });
}

fn terminate_reason(reason: mir::UnwindTerminateReason) -> &'static str {
    match reason {
        mir::UnwindTerminateReason::Abi => "abi",
        mir::UnwindTerminateReason::InCleanup => "panic_during_cleanup",
    }
}

#[derive(Default)]
struct Effects {
    locals: CompilerLocalEffects,
    call_defs: Vec<u32>,
}

impl Effects {
    fn unknown(&mut self, effect: CompilerUnknownEffect) {
        self.locals.unknown_effects.push(effect);
    }
    fn finish(mut self) -> CompilerLocalEffects {
        for values in [
            &mut self.locals.defs,
            &mut self.locals.uses,
            &mut self.locals.moves,
            &mut self.locals.storage_live,
            &mut self.locals.storage_dead,
        ] {
            values.sort_unstable();
            values.dedup();
        }
        self.locals.unknown_effects.sort_unstable();
        self.locals.unknown_effects.dedup();
        self.locals
    }
}

impl<'tcx> Visitor<'tcx> for Effects {
    fn visit_rvalue(&mut self, value: &mir::Rvalue<'tcx>, location: mir::Location) {
        if matches!(value, mir::Rvalue::ThreadLocalRef(_)) {
            self.unknown(CompilerUnknownEffect::ExternalState);
        }
        self.super_rvalue(value, location);
    }

    fn visit_place(
        &mut self,
        place: &mir::Place<'tcx>,
        context: PlaceContext,
        location: mir::Location,
    ) {
        if place.projection.is_empty() {
            self.visit_local(place.local, context, location);
        } else {
            self.locals.uses.push(place.local.as_u32());
            if place
                .projection
                .iter()
                .any(|elem| matches!(elem, mir::ProjectionElem::Deref))
            {
                self.unknown(CompilerUnknownEffect::PointerAliasing);
            }
            if matches!(context, PlaceContext::MutatingUse(_)) {
                self.unknown(CompilerUnknownEffect::PartialWrite);
            }
            if context.is_borrow() || context.is_address_of() {
                self.unknown(CompilerUnknownEffect::BorrowAliasing);
            }
            self.visit_projection(place.as_ref(), context, location);
        }
    }

    fn visit_local(&mut self, local: mir::Local, context: PlaceContext, _: mir::Location) {
        let local = local.as_u32();
        match context {
            PlaceContext::MutatingUse(Mut::Store) => self.locals.defs.push(local),
            PlaceContext::MutatingUse(Mut::Call) => self.call_defs.push(local),
            PlaceContext::MutatingUse(Mut::Yield) => {
                self.locals.uses.push(local);
                self.unknown(CompilerUnknownEffect::PartialWrite);
            }
            PlaceContext::MutatingUse(Mut::Drop) => {
                self.locals.uses.push(local);
                self.unknown(CompilerUnknownEffect::Drop);
            }
            PlaceContext::MutatingUse(Mut::Borrow | Mut::RawBorrow)
            | PlaceContext::NonMutatingUse(
                Read::SharedBorrow | Read::RawBorrow | Read::FakeBorrow,
            ) => {
                self.locals.uses.push(local);
                self.unknown(CompilerUnknownEffect::BorrowAliasing);
            }
            PlaceContext::MutatingUse(Mut::SetDiscriminant | Mut::Projection | Mut::AsmOutput) => {
                self.locals.uses.push(local);
                self.unknown(CompilerUnknownEffect::PartialWrite);
            }
            PlaceContext::MutatingUse(Mut::Retag) => {
                self.locals.uses.push(local);
                self.unknown(CompilerUnknownEffect::Retag);
            }
            PlaceContext::NonMutatingUse(Read::Move) => {
                self.locals.uses.push(local);
                self.locals.moves.push(local);
            }
            PlaceContext::NonMutatingUse(_) => self.locals.uses.push(local),
            PlaceContext::NonUse(Non::StorageLive) => self.locals.storage_live.push(local),
            PlaceContext::NonUse(Non::StorageDead) => self.locals.storage_dead.push(local),
            PlaceContext::NonUse(_) => {}
        }
    }
}
