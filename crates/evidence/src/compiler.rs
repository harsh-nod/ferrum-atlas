//! Import compiler records without executing a toolchain or analyzed source.
use super::ScopedDirectory;
use anyhow::{Context, Result, ensure};
use atlas_model::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: u64 = 32 * 1024 * 1024;
const COMMIT: &str = "55e86c996809902e8bbad512cfb4d2c18be446d9";
const TARGET: &str = "x86_64-unknown-linux-gnu";
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn folder(root: &Path, snapshot: &SnapshotId) -> PathBuf {
    root.join(
        digest("compiler-scope", snapshot)
            .split_once(':')
            .unwrap()
            .1,
    )
}

pub fn validate(
    bundle: &CompilerBundle,
    snapshot: &Snapshot,
    files: &[SourceFile],
    definitions: &[Definition],
) -> Result<CompilerImport> {
    ensure!(
        serde_json::to_vec(bundle)?.len() <= MAX_BYTES as usize,
        "compiler bundle exceeds 32 MiB"
    );
    ensure!(
        bundle.schema_version == COMPILER_SCHEMA_VERSION,
        "unsupported compiler schema"
    );
    ensure!(
        bundle.compiler.adapter == "atlas-rustc"
            && bundle.compiler.adapter_version == "0.1.0"
            && bundle.compiler.commit_hash == COMMIT
            && bundle.compiler.host == TARGET
            && bundle.phase == "runtime_optimized",
        "unsupported compiler producer or MIR phase"
    );
    ensure!(
        bundle.inputs.target == TARGET && bundle.inputs.target == snapshot.context.target,
        "compiler target does not match the selected context; select an explicit target"
    );
    ensure!(
        !snapshot.context.default_features,
        "compiler import requires explicit --no-default-features and explicit selected feature cfg values"
    );
    ensure!(
        bundle.inputs.trust == "trusted_local" && bundle.inputs.environment_policy == "empty",
        "unsupported compiler input policy"
    );
    ensure!(
        bundle.inputs.mir_opt_level == 0
            && matches!(bundle.inputs.panic_strategy.as_str(), "abort" | "unwind")
            && bundle.inputs.compiled_artifact.is_none(),
        "unsupported compiler phase settings"
    );
    ensure!(
        snapshot
            .context
            .crates
            .iter()
            .any(|krate| krate.root_file == bundle.inputs.crate_root
                && krate.name.replace('-', "_") == bundle.inputs.crate_name
                && krate.edition == bundle.inputs.edition),
        "compiler crate root/name/edition does not match the selected context"
    );
    let mut expected: BTreeSet<String> = snapshot
        .context
        .cfg
        .iter()
        .map(|(key, value)| match value {
            Some(value) => format!("{key}={}", serde_json::to_string(value).unwrap()),
            None => key.clone(),
        })
        .collect();
    expected.extend(
        snapshot
            .context
            .features
            .iter()
            .map(|feature| format!("feature={}", serde_json::to_string(feature).unwrap())),
    );
    ensure!(
        expected.len() <= 256 && expected.iter().all(|value| value.len() <= 4096),
        "compiler cfg argument budget exceeded"
    );
    ensure!(
        bundle.inputs.rustc_args == canonical_arguments(bundle, &expected),
        "compiler arguments do not match the selected context and supported canonical invocation"
    );
    let mut inputs = bundle.inputs.clone();
    inputs.manifest_hash.clear();
    ensure!(
        bundle.inputs.manifest_hash
            == sha(&serde_json::to_vec(&(
                &bundle.compiler,
                &bundle.phase,
                &inputs
            ))?),
        "compiler input manifest checksum mismatch"
    );
    ensure!(
        bundle.inputs.files.len() <= 20_000 && bundle.bodies.len() <= 10_000,
        "compiler input count exceeds budget"
    );
    let mut sources = BTreeMap::new();
    for file in &bundle.inputs.files {
        ensure!(
            !file.path.is_empty()
                && Path::new(&file.path)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "compiler source path is not relative"
        );
        let source = files
            .iter()
            .find(|source| source.path == file.path)
            .context("compiler source is not captured in the selected snapshot")?;
        ensure!(
            file.byte_length as usize == source.text.len()
                && file.sha256 == sha(source.text.as_bytes()),
            "compiler source bytes do not match the selected snapshot"
        );
        ensure!(
            sources.insert(file.path.as_str(), source).is_none(),
            "duplicate compiler source path"
        );
    }
    ensure!(
        sources.contains_key(bundle.inputs.crate_root.as_str()),
        "compiler crate root is absent from manifest"
    );
    let mut ids = BTreeSet::new();
    let mut mappings = Vec::new();
    let mut statements = 0usize;
    for body in &bundle.bodies {
        ensure!(
            ids.insert(&body.body_id)
                && body.body_id
                    == format!(
                        "mir:{}",
                        sha(&serde_json::to_vec(&(
                            &bundle.inputs.manifest_hash,
                            &body.def_path
                        ))?)
                    ),
            "compiler body identity mismatch"
        );
        validate_body(body, &sources, &mut statements)?;
        let mut candidates = Vec::new();
        if let CompilerSourceMapping::Exact {
            path,
            start_byte,
            end_byte,
        } = &body.span
        {
            let source = sources[path.as_str()];
            for definition in definitions {
                let compatible_kind = match body.kind.as_str() {
                    "function" | "associated_function" => {
                        matches!(definition.kind.as_str(), "function" | "method")
                    }
                    "closure_or_coroutine" => definition.kind == "closure",
                    _ => false,
                };
                if compatible_kind
                    && definition.file_id == source.id
                    && definition.cfg_status != CfgStatus::Inactive
                    && definition.span.start <= *start_byte
                    && definition.span.end >= *end_byte
                {
                    candidates.push(definition);
                }
            }
        }
        candidates.sort_by_key(|definition| definition.span.end - definition.span.start);
        let mapping = match candidates.as_slice() {
            [only, rest @ ..] if rest.first().is_none_or(|next| (next.span.end-next.span.start) > (only.span.end-only.span.start)) => CompilerDefinitionMapping::Matched { definition_id: only.id.clone() },
            _ => CompilerDefinitionMapping::Unmapped { reason: "No unique compatible source definition for the exact compiler span; generated or ambiguous bodies are not guessed.".into() },
        };
        mappings.push(CompilerBodyMapping {
            body_id: body.body_id.clone(),
            mapping,
        });
    }
    let mut coverage = Coverage::partial(
        UnknownReason::UnsupportedConstruct,
        "Compiler imports describe one selected producer/phase, not all source bodies or runtime behavior. Producer claims are user supplied; matching hashes establish identity consistency, not authenticity.",
    );
    coverage.limitations.extend(bundle.limitations.clone());
    Ok(CompilerImport {
        snapshot_id: snapshot.id.clone(),
        context_id: snapshot.context.id.clone(),
        bundle: bundle.clone(),
        mappings,
        coverage,
    })
}

fn canonical_arguments(bundle: &CompilerBundle, cfg: &BTreeSet<String>) -> Vec<String> {
    let mut arguments = vec![
        "atlas-rustc".into(),
        format!("./{}", bundle.inputs.crate_root),
        "--crate-name".into(),
        bundle.inputs.crate_name.clone(),
        "--crate-type=lib".into(),
        format!("--edition={}", bundle.inputs.edition),
        format!("--target={}", bundle.inputs.target),
        format!("-Cpanic={}", bundle.inputs.panic_strategy),
        "-Copt-level=0".into(),
        "-Coverflow-checks=yes".into(),
        "-Zmir-opt-level=0".into(),
        "--emit=metadata".into(),
        "--sysroot".into(),
        format!(
            "compiler:{}:{}",
            bundle.compiler.commit_hash, bundle.compiler.host
        ),
        "--error-format=json".into(),
    ];
    for value in cfg {
        arguments.push("--cfg".into());
        arguments.push(value.clone());
    }
    arguments
}

fn span(span: &CompilerSourceMapping, sources: &BTreeMap<&str, &SourceFile>) -> Result<()> {
    if let CompilerSourceMapping::Exact {
        path,
        start_byte,
        end_byte,
    } = span
    {
        let source = sources
            .get(path.as_str())
            .context("compiler span refers to an uncaptured file")?;
        let (start, end) = (*start_byte as usize, *end_byte as usize);
        ensure!(
            start <= end
                && end <= source.text.len()
                && source.text.is_char_boundary(start)
                && source.text.is_char_boundary(end),
            "invalid compiler UTF-8 byte span"
        );
    }
    Ok(())
}
fn effects(effects: &CompilerLocalEffects, locals: usize) -> Result<()> {
    for list in [
        &effects.defs,
        &effects.uses,
        &effects.moves,
        &effects.storage_live,
        &effects.storage_dead,
    ] {
        ensure!(
            list.iter().all(|index| (*index as usize) < locals)
                && list.windows(2).all(|pair| pair[0] < pair[1]),
            "compiler local references must be in range, unique, and sorted"
        );
    }
    Ok(())
}
fn validate_body(
    body: &CompilerBody,
    sources: &BTreeMap<&str, &SourceFile>,
    statements: &mut usize,
) -> Result<()> {
    ensure!(
        !body.locals.is_empty()
            && body.locals.len() <= 10_000
            && body.argument_count < body.locals.len() as u32,
        "invalid compiler locals"
    );
    ensure!(
        matches!(
            body.kind.as_str(),
            "function" | "associated_function" | "closure_or_coroutine" | "synthetic_coroutine"
        ),
        "unsupported compiler body kind"
    );
    ensure!(
        !body.blocks.is_empty()
            && body.blocks.len() <= 10_000
            && !body.source_scopes.is_empty()
            && body.source_scopes.len() <= 10_000,
        "compiler body exceeds block/scope budget"
    );
    span(&body.span, sources)?;
    for (index, scope) in body.source_scopes.iter().enumerate() {
        ensure!(
            scope.index == index as u32 && scope.parent.is_none_or(|parent| parent < index as u32),
            "invalid compiler scope tree"
        );
        span(&scope.span, sources)?;
    }
    for (index, local) in body.locals.iter().enumerate() {
        ensure!(
            local.index == index as u32 && (local.source_scope as usize) < body.source_scopes.len(),
            "invalid compiler local/scope identity"
        );
        let role = if index == 0 {
            "return"
        } else if index <= body.argument_count as usize {
            "argument"
        } else {
            "temporary"
        };
        ensure!(local.role == role, "invalid compiler local role");
        span(&local.span, sources)?;
    }
    for (index, block) in body.blocks.iter().enumerate() {
        ensure!(
            block.index == index as u32,
            "invalid compiler block identity"
        );
        *statements = statements.saturating_add(block.statements.len() + 1);
        ensure!(*statements <= 200_000, "compiler statement budget exceeded");
        for (index, statement) in block.statements.iter().enumerate() {
            ensure!(
                statement.index == index as u32
                    && (statement.source_scope as usize) < body.source_scopes.len(),
                "invalid statement/scope identity"
            );
            ensure!(
                matches!(
                    statement.kind.as_str(),
                    "assign"
                        | "fake_read"
                        | "set_discriminant"
                        | "storage_live"
                        | "storage_dead"
                        | "retag"
                        | "place_mention"
                        | "ascribe_user_type"
                        | "coverage"
                        | "intrinsic"
                        | "const_eval_counter"
                        | "nop"
                        | "backward_incompatible_drop_hint"
                ),
                "unsupported compiler statement kind"
            );
            span(&statement.span, sources)?;
            effects(&statement.locals, body.locals.len())?;
        }
        let term = &block.terminator;
        ensure!(
            (term.source_scope as usize) < body.source_scopes.len(),
            "invalid terminator scope"
        );
        span(&term.span, sources)?;
        effects(&term.locals, body.locals.len())?;
        ensure!(
            term.normal_return_defs
                .iter()
                .all(|index| (*index as usize) < body.locals.len())
                && term
                    .normal_return_defs
                    .windows(2)
                    .all(|pair| pair[0] < pair[1]),
            "invalid call destination"
        );
        for edge in &term.successors {
            ensure!(
                (edge.target as usize) < body.blocks.len(),
                "compiler successor is outside its body"
            );
            if let Some(value) = &edge.switch_value {
                ensure!(
                    !value.is_empty()
                        && value.bytes().all(|c| c.is_ascii_digit())
                        && value.parse::<u128>().is_ok(),
                    "invalid switch value"
                );
            }
        }
        if let Some(CompilerUnwind::Cleanup { target }) = &term.unwind {
            ensure!(body.blocks.get(*target as usize).is_some_and(|block| block.is_cleanup)
                && term.successors.iter().any(|edge| edge.target == *target && edge.kind == CompilerEdgeKind::Unwind), "invalid cleanup edge");
        }
        validate_terminator(term)?;
    }
    Ok(())
}

fn validate_terminator(term: &CompilerTerminator) -> Result<()> {
    use CompilerEdgeKind as Edge;
    let count = |kind| {
        term.successors
            .iter()
            .filter(|edge| edge.kind == kind)
            .count()
    };
    let normal = count(Edge::Normal);
    let unwind = count(Edge::Unwind);
    let coroutine_drop = count(Edge::CoroutineDrop);
    let mut switches = BTreeSet::new();
    for edge in &term.successors {
        match (&edge.kind, &edge.switch_value) {
            (Edge::SwitchValue, Some(text)) => {
                let value = text.parse::<u128>().context("invalid switch value")?;
                ensure!(
                    value.to_string() == *text && switches.insert(value),
                    "switch values must be canonical and unique"
                );
            }
            (Edge::SwitchValue, None) => anyhow::bail!("switch-value edge lacks a value"),
            (_, Some(_)) => anyhow::bail!("non-switch edge contains a switch value"),
            (_, None) => {}
        }
    }
    ensure!(
        term.assert_expected.is_some() == (term.kind == "assert"),
        "assert metadata does not match terminator kind"
    );
    ensure!(
        term.call_target.is_some() == matches!(term.kind.as_str(), "call" | "tail_call"),
        "call metadata does not match terminator kind"
    );
    ensure!(
        term.normal_return_defs.is_empty() || (term.kind == "call" && normal == 1),
        "normal-return definitions require a returning call"
    );
    match &term.unwind {
        Some(CompilerUnwind::Cleanup { target }) => ensure!(
            unwind == 1
                && term
                    .successors
                    .iter()
                    .any(|edge| edge.kind == Edge::Unwind && edge.target == *target),
            "unwind metadata and successor disagree"
        ),
        Some(CompilerUnwind::Terminate { reason }) => {
            ensure!(
                matches!(reason.as_str(), "abi" | "panic_during_cleanup"),
                "unsupported unwind termination reason"
            );
            ensure!(unwind == 0, "non-cleanup unwind has a successor");
        }
        _ => ensure!(unwind == 0, "unwind successor requires cleanup metadata"),
    }
    let total = term.successors.len();
    let is_valid = match term.kind.as_str() {
        "goto" => normal == 1 && total == 1 && term.unwind.is_none(),
        "switch_int" => {
            count(Edge::Otherwise) == 1
                && total == switches.len() + 1
                && term
                    .successors
                    .last()
                    .is_some_and(|edge| edge.kind == Edge::Otherwise)
                && term.unwind.is_none()
        }
        "return" | "unwind_resume" | "unreachable" | "coroutine_drop" | "tail_call" => {
            total == 0 && term.unwind.is_none()
        }
        "unwind_terminate" => {
            total == 0 && matches!(term.unwind, Some(CompilerUnwind::Terminate { .. }))
        }
        "drop" => {
            normal == 1
                && coroutine_drop <= 1
                && total == normal + coroutine_drop + unwind
                && term.unwind.is_some()
        }
        "call" => normal <= 1 && total == normal + unwind && term.unwind.is_some(),
        "assert" | "false_unwind" => {
            normal == 1 && total == normal + unwind && term.unwind.is_some()
        }
        "yield" => {
            count(Edge::Resume) == 1
                && coroutine_drop <= 1
                && total == 1 + coroutine_drop
                && term.unwind.is_none()
        }
        "false_edge" => {
            normal == 1 && count(Edge::Imaginary) == 1 && total == 2 && term.unwind.is_none()
        }
        "inline_asm" => total == normal + unwind && term.unwind.is_some(),
        _ => false,
    };
    ensure!(is_valid, "terminator kind and successor shape disagree");
    let required_effect = match term.kind.as_str() {
        "call" | "tail_call" => Some(CompilerUnknownEffect::Call),
        "drop" => Some(CompilerUnknownEffect::Drop),
        "inline_asm" => Some(CompilerUnknownEffect::InlineAssembly),
        _ => None,
    };
    ensure!(
        required_effect.is_none_or(|effect| term.locals.unknown_effects.contains(&effect)),
        "required conservative terminator effect is absent"
    );
    ensure!(
        !matches!(term.call_target, Some(CompilerCallTarget::Indirect { .. }))
            || term
                .locals
                .unknown_effects
                .contains(&CompilerUnknownEffect::IndirectCall),
        "indirect call is missing its conservative effect"
    );
    Ok(())
}

pub fn import(
    root: &Path,
    bundle: &CompilerBundle,
    snapshot: &Snapshot,
    files: &[SourceFile],
    definitions: &[Definition],
) -> Result<CompilerImportSummary> {
    let imported = validate(bundle, snapshot, files, definitions)?;
    ensure!(
        serde_json::to_vec(&imported)?.len() <= (MAX_BYTES + 1024 * 1024) as usize,
        "compiler mapping exceeds object byte budget"
    );
    let id = digest("compiler-import", &imported);
    let folder = ScopedDirectory::open(&folder(root, &snapshot.id), true)?
        .context("compiler scope unavailable")?;
    let _lock = folder.import_lock()?;
    let path = PathBuf::from(format!("{}.json", id.split_once(':').unwrap().1));
    let names = folder.json_names(16)?;
    if !folder.exists(&path)? {
        ensure!(names.len() < 16, "compiler import limit reached");
        folder.publish(&path, &serde_json::to_vec(&imported)?)?;
    }
    ensure!(
        digest("compiler-import", &read_from(&folder, &path)?) == id,
        "compiler import checksum mismatch"
    );
    Ok(summary(&id, &imported))
}
fn read_from(folder: &ScopedDirectory, name: &Path) -> Result<CompilerImport> {
    let value: CompilerImport =
        serde_json::from_slice(&folder.read(name, MAX_BYTES + 1024 * 1024)?)?;
    let id = digest("compiler-import", &value);
    ensure!(
        name.file_stem().and_then(|s| s.to_str()) == id.split_once(':').map(|(_, id)| id),
        "compiler object checksum mismatch"
    );
    Ok(value)
}
#[cfg(test)]
fn read(path: &Path) -> Result<CompilerImport> {
    let directory = ScopedDirectory::open(
        path.parent().context("compiler object has no parent")?,
        false,
    )?
    .context("compiler scope unavailable")?;
    read_from(
        &directory,
        Path::new(path.file_name().context("compiler object has no name")?),
    )
}
fn summary(id: &str, import: &CompilerImport) -> CompilerImportSummary {
    CompilerImportSummary {
        id: id.into(),
        snapshot_id: import.snapshot_id.clone(),
        context_id: import.context_id.clone(),
        compiler: import.bundle.compiler.clone(),
        phase: import.bundle.phase.clone(),
        body_count: import.bundle.bodies.len() as u32,
        mapped_count: import
            .mappings
            .iter()
            .filter(|mapping| matches!(mapping.mapping, CompilerDefinitionMapping::Matched { .. }))
            .count() as u32,
        coverage: import.coverage.clone(),
    }
}
pub fn list(
    root: &Path,
    snapshot: &SnapshotId,
    context: &ContextId,
) -> Result<Vec<CompilerImportSummary>> {
    let Some(directory) = ScopedDirectory::open(&folder(root, snapshot), false)? else {
        return Ok(vec![]);
    };
    directory
        .json_names(16)?
        .into_iter()
        .map(|path| {
            let imported = read_from(&directory, &path)?;
            ensure!(
                &imported.snapshot_id == snapshot && &imported.context_id == context,
                "compiler scope mismatch"
            );
            Ok(summary(&digest("compiler-import", &imported), &imported))
        })
        .collect()
}
pub fn flow(
    root: &Path,
    snapshot: &SnapshotId,
    context: &ContextId,
    definition: &DefinitionId,
    import_id: &str,
    window: (u32, u32),
) -> Result<CompilerFlowPage> {
    let (offset, limit) = window;
    ensure!(
        (1..=200).contains(&limit),
        "compiler block window must be 1..200"
    );
    let suffix = import_id
        .strip_prefix("compiler-import:")
        .context("invalid compiler import identity")?;
    ensure!(
        suffix.len() == 64 && suffix.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid compiler import identity"
    );
    let directory = ScopedDirectory::open(&folder(root, snapshot), false)?
        .context("compiler scope unavailable")?;
    let imported = read_from(&directory, Path::new(&format!("{suffix}.json")))?;
    ensure!(
        &imported.snapshot_id == snapshot && &imported.context_id == context,
        "compiler scope mismatch"
    );
    let matched: Vec<_> = imported.mappings.iter().filter(|mapping| matches!(&mapping.mapping, CompilerDefinitionMapping::Matched { definition_id } if definition_id == definition)).collect();
    ensure!(
        matched.len() == 1,
        "no unique compiler body for this definition"
    );
    let mut body = imported
        .bundle
        .bodies
        .iter()
        .find(|body| body.body_id == matched[0].body_id)
        .context("compiler body unavailable")?
        .clone();
    let total_blocks = body.blocks.len() as u32;
    ensure!(
        offset < total_blocks,
        "compiler block offset is outside body"
    );
    let end = offset.saturating_add(limit).min(total_blocks);
    body.blocks = body.blocks[offset as usize..end as usize].to_vec();
    Ok(CompilerFlowPage {
        snapshot_id: snapshot.clone(),
        context_id: context.clone(),
        import_id: import_id.into(),
        definition_id: definition.clone(),
        compiler: imported.bundle.compiler,
        phase: imported.bundle.phase,
        input_manifest_hash: imported.bundle.inputs.manifest_hash,
        panic_strategy: imported.bundle.inputs.panic_strategy,
        body,
        offset,
        total_blocks,
        next_offset: (end < total_blocks).then_some(end),
        coverage: imported.coverage,
    })
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    include!("compiler/tests.rs");
    mod io {
        use super::*;
        include!("compiler/io_tests.rs");
    }
}
