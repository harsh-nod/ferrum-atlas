use super::*;

fn fixture() -> (CompilerBundle, Snapshot, Vec<SourceFile>, Vec<Definition>) {
    let file = SourceFile {
        id: FileId("file:one".into()),
        path: "lib.rs".into(),
        content_hash: "unused".into(),
        text: "pub fn entry() {}\r\n".into(),
    };
    let context = BuildContext {
        id: ContextId("context:one".into()),
        name: "host".into(),
        target: "x86_64-unknown-linux-gnu".into(),
        features: vec![],
        default_features: false,
        cfg: Default::default(),
        crates: vec![CrateInput {
            name: "fixture".into(),
            root_file: "lib.rs".into(),
            edition: "2021".into(),
            dependencies: Default::default(),
        }],
        manifest_digest: "manifest".into(),
        trust: "read_only".into(),
        coverage: Coverage::complete(),
    };
    let snapshot = Snapshot {
        id: SnapshotId("analysis:one".into()),
        source_id: SourceId("source:one".into()),
        repository_id: RepositoryId("repo:one".into()),
        context: context.clone(),
        revision: "fixture".into(),
        created_at: "1".into(),
        file_count: 1,
        definition_count: 1,
        relation_count: 0,
        coverage: Coverage::complete(),
        producer: "fixture".into(),
        fact_digest: "facts".into(),
    };
    let definition = Definition {
        id: DefinitionId("definition:entry".into()),
        context_id: context.id,
        file_id: file.id.clone(),
        name: "entry".into(),
        qualified_name: "fixture::entry".into(),
        kind: "function".into(),
        parent_id: None,
        signature: "pub fn entry()".into(),
        signature_hash: "signature".into(),
        body_hash: "body".into(),
        span: Span {
            file_id: file.id.clone(),
            start: 0,
            end: 17,
        },
        body_span: None,
        cfg: vec![],
        cfg_status: CfgStatus::Active,
        visibility: "pub".into(),
        metrics: Default::default(),
    };
    let mapping = CompilerSourceMapping::Exact {
        path: file.path.clone(),
        start_byte: 0,
        end_byte: 17,
    };
    let mut bundle = CompilerBundle {
        schema_version: 1,
        compiler: CompilerIdentity {
            adapter: "atlas-rustc".into(),
            adapter_version: "0.1.0".into(),
            release: "1.96.0-nightly".into(),
            commit_hash: COMMIT.into(),
            commit_date: "2026-04-02".into(),
            host: context.target.clone(),
            llvm_version: "22.1.2".into(),
        },
        inputs: CompilerInputs {
            manifest_hash: String::new(),
            files: vec![CompilerSourceFile {
                path: file.path.clone(),
                sha256: sha(file.text.as_bytes()),
                byte_length: file.text.len() as u32,
            }],
            crate_name: "fixture".into(),
            crate_root: "lib.rs".into(),
            edition: "2021".into(),
            target: context.target,
            panic_strategy: "unwind".into(),
            mir_opt_level: 0,
            rustc_args: vec![],
            environment_policy: "empty".into(),
            trust: "trusted_local".into(),
            compiled_artifact: None,
        },
        phase: "runtime_optimized".into(),
        bodies: vec![CompilerBody {
            body_id: String::new(),
            def_path: "fixture::entry".into(),
            kind: "function".into(),
            span: mapping.clone(),
            argument_count: 0,
            locals: vec![CompilerLocal {
                index: 0,
                role: "return".into(),
                names: vec![],
                type_display: "()".into(),
                source_scope: 0,
                span: mapping.clone(),
            }],
            source_scopes: vec![CompilerSourceScope {
                index: 0,
                parent: None,
                span: mapping.clone(),
                inlined_def_path: None,
            }],
            blocks: vec![CompilerBlock {
                index: 0,
                is_cleanup: false,
                statements: vec![],
                terminator: CompilerTerminator {
                    kind: "return".into(),
                    source_scope: 0,
                    span: mapping,
                    locals: Default::default(),
                    normal_return_defs: vec![],
                    successors: vec![],
                    unwind: None,
                    call_target: None,
                    assert_expected: None,
                },
            }],
        }],
        limitations: vec![
            "Hand-authored import validator fixture, not compiler execution evidence.".into(),
        ],
    };
    seal(&mut bundle);
    (bundle, snapshot, vec![file], vec![definition])
}
fn seal(bundle: &mut CompilerBundle) {
    bundle.inputs.manifest_hash.clear();
    bundle.inputs.manifest_hash =
        sha(&serde_json::to_vec(&(&bundle.compiler, &bundle.phase, &bundle.inputs)).unwrap());
    for body in &mut bundle.bodies {
        body.body_id = format!(
            "mir:{}",
            sha(&serde_json::to_vec(&(&bundle.inputs.manifest_hash, &body.def_path)).unwrap())
        );
    }
}

#[test]
fn matching_compiler_import_is_deduplicated_pinned_and_windowed() {
    let (bundle, snapshot, files, definitions) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let imported = import(temp.path(), &bundle, &snapshot, &files, &definitions).unwrap();
    assert_eq!(imported.mapped_count, 1);
    assert_eq!(
        import(temp.path(), &bundle, &snapshot, &files, &definitions)
            .unwrap()
            .id,
        imported.id
    );
    assert_eq!(
        list(temp.path(), &snapshot.id, &snapshot.context.id)
            .unwrap()
            .len(),
        1
    );
    let page = flow(
        temp.path(),
        &snapshot.id,
        &snapshot.context.id,
        &definitions[0].id,
        &imported.id,
        (0, 1),
    )
    .unwrap();
    assert_eq!(page.phase, "runtime_optimized");
    assert_eq!(page.body.blocks[0].terminator.kind, "return");
    assert_eq!(page.next_offset, None);
    assert!(
        flow(
            temp.path(),
            &snapshot.id,
            &snapshot.context.id,
            &definitions[0].id,
            &imported.id,
            (1, 1)
        )
        .is_err()
    );
    assert!(
        flow(
            temp.path(),
            &snapshot.id,
            &ContextId("wrong".into()),
            &definitions[0].id,
            &imported.id,
            (0, 1)
        )
        .is_err()
    );
    assert!(
        flow(
            temp.path(),
            &snapshot.id,
            &snapshot.context.id,
            &definitions[0].id,
            "compiler-import:../../etc/passwd",
            (0, 1)
        )
        .is_err()
    );
}

#[test]
fn source_context_compiler_and_cfg_mismatches_are_rejected() {
    let (bundle, snapshot, files, definitions) = fixture();
    let mut changed = files.clone();
    changed[0].text.push(' ');
    assert!(validate(&bundle, &snapshot, &changed, &definitions).is_err());
    let mut changed = snapshot.clone();
    changed.context.target = "unknown".into();
    assert!(validate(&bundle, &changed, &files, &definitions).is_err());
    changed = snapshot.clone();
    changed.context.default_features = true;
    assert!(validate(&bundle, &changed, &files, &definitions).is_err());
    changed = snapshot.clone();
    changed.context.cfg.insert("test".into(), None);
    assert!(validate(&bundle, &changed, &files, &definitions).is_err());
    let mut compiler = bundle.clone();
    compiler.compiler.commit_hash = "unsupported".into();
    seal(&mut compiler);
    assert!(validate(&compiler, &snapshot, &files, &definitions).is_err());
    compiler = bundle.clone();
    compiler.inputs.manifest_hash = "wrong".into();
    assert!(validate(&compiler, &snapshot, &files, &definitions).is_err());
}

#[test]
fn malformed_cfg_local_scope_successor_and_span_references_are_rejected() {
    let (bundle, snapshot, files, definitions) = fixture();
    let mut bad = bundle.clone();
    bad.inputs.rustc_args.push("--cfg".into());
    seal(&mut bad);
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle.clone();
    bad.bodies[0].blocks[0].terminator.locals.defs.push(1);
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle.clone();
    bad.bodies[0].source_scopes[0].parent = Some(0);
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle.clone();
    bad.bodies[0].blocks[0]
        .terminator
        .successors
        .push(CompilerSuccessor {
            target: 5,
            kind: CompilerEdgeKind::Normal,
            switch_value: None,
        });
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle.clone();
    bad.bodies[0].span = CompilerSourceMapping::Exact {
        path: "lib.rs".into(),
        start_byte: 0,
        end_byte: 999,
    };
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
}

#[test]
fn ambiguous_mappings_and_unavailable_origins_are_not_guessed() {
    let (mut bundle, snapshot, files, mut definitions) = fixture();
    let mut other = definitions[0].clone();
    other.id = DefinitionId("definition:other".into());
    definitions.push(other);
    let imported = validate(&bundle, &snapshot, &files, &definitions).unwrap();
    assert!(matches!(
        imported.mappings[0].mapping,
        CompilerDefinitionMapping::Unmapped { .. }
    ));
    bundle.bodies[0].span = CompilerSourceMapping::Unavailable {
        reason: "macro origin".into(),
    };
    let imported = validate(&bundle, &snapshot, &files, &definitions[..1]).unwrap();
    assert!(matches!(
        imported.mappings[0].mapping,
        CompilerDefinitionMapping::Unmapped { .. }
    ));
}

#[test]
fn compiler_object_corruption_and_invalid_cleanup_are_visible() {
    let (mut bundle, snapshot, files, definitions) = fixture();
    let temp = tempfile::tempdir().unwrap();
    let imported = import(temp.path(), &bundle, &snapshot, &files, &definitions).unwrap();
    let path = folder(temp.path(), &snapshot.id)
        .join(format!("{}.json", imported.id.split_once(':').unwrap().1));
    let mut object = read(&path).unwrap();
    object.bundle.phase = "wrong".into();
    serde_json::to_writer(File::create(path).unwrap(), &object).unwrap();
    assert!(list(temp.path(), &snapshot.id, &snapshot.context.id).is_err());
    bundle.bodies[0].blocks[0].terminator.unwind = Some(CompilerUnwind::Cleanup { target: 0 });
    assert!(validate(&bundle, &snapshot, &files, &definitions).is_err());
}
