//! Import compiler records without executing a toolchain or analyzed source.
use anyhow::{Context, Result, ensure};
use atlas_model::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: u64 = 32 * 1024 * 1024;
const COMMIT: &str = "55e86c996809902e8bbad512cfb4d2c18be446d9";
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
            && bundle.compiler.commit_hash == COMMIT
            && bundle.phase == "runtime_optimized",
        "unsupported compiler producer or MIR phase"
    );
    ensure!(
        bundle.inputs.target == snapshot.context.target,
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
            && matches!(bundle.inputs.panic_strategy.as_str(), "abort" | "unwind"),
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
    let mut cfg = BTreeSet::new();
    let mut args = bundle.inputs.rustc_args.iter();
    while let Some(arg) = args.next() {
        if arg == "--cfg" {
            cfg.insert(args.next().context("missing compiler cfg value")?.clone());
        }
        ensure!(
            !arg.starts_with("--cfg="),
            "compiler cfg must use the supported paired argument contract"
        );
    }
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
        cfg == expected,
        "compiler cfg/features do not match the selected context"
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
            list.iter().all(|index| (*index as usize) < locals),
            "compiler local reference out of range"
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
                .all(|index| (*index as usize) < body.locals.len()),
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
    }
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
    let folder = folder(root, &snapshot.id);
    fs::create_dir_all(&folder)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join(".import.lock"))?;
    lock.lock()?;
    let path = folder.join(format!("{}.json", id.split_once(':').unwrap().1));
    if !path.exists() {
        ensure!(
            fs::read_dir(&folder)?
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json"))
                .count()
                < 16,
            "compiler import limit reached"
        );
        let mut file = tempfile::NamedTempFile::new_in(&folder)?;
        serde_json::to_writer(&mut file, &imported)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&path)?;
        File::open(&folder)?.sync_all()?;
    }
    ensure!(
        digest("compiler-import", &read(&path)?) == id,
        "compiler import checksum mismatch"
    );
    Ok(summary(&id, &imported))
}
fn read(path: &Path) -> Result<CompilerImport> {
    ensure!(
        path.symlink_metadata()?.is_file(),
        "compiler object must be a regular file"
    );
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() <= MAX_BYTES + 1024 * 1024,
        "compiler object exceeds budget"
    );
    let value: CompilerImport = serde_json::from_reader(file.take(MAX_BYTES + 1024 * 1024 + 1))?;
    let id = digest("compiler-import", &value);
    ensure!(
        path.file_stem().and_then(|s| s.to_str()) == id.split_once(':').map(|(_, id)| id),
        "compiler object checksum mismatch"
    );
    Ok(value)
}
fn paths(root: &Path, snapshot: &SnapshotId) -> Result<Vec<PathBuf>> {
    let folder = folder(root, snapshot);
    if !folder.exists() {
        return Ok(vec![]);
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
        ensure!(paths.len() <= 16, "compiler import count exceeds budget");
    }
    paths.sort();
    Ok(paths)
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
    paths(root, snapshot)?
        .into_iter()
        .map(|path| {
            let imported = read(&path)?;
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
    let imported = read(&folder(root, snapshot).join(format!("{suffix}.json")))?;
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
mod tests;
