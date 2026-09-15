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
            rustc_args: vec![
                "atlas-rustc".into(),
                "./lib.rs".into(),
                "--crate-name".into(),
                "fixture".into(),
                "--crate-type=lib".into(),
                "--edition=2021".into(),
                "--target=x86_64-unknown-linux-gnu".into(),
                "-Cpanic=unwind".into(),
                "-Copt-level=0".into(),
                "-Coverflow-checks=yes".into(),
                "-Zmir-opt-level=0".into(),
                "--emit=metadata".into(),
                "--sysroot".into(),
                format!("compiler:{COMMIT}:x86_64-unknown-linux-gnu"),
                "--error-format=json".into(),
            ],
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

#[test]
fn canonical_invocation_rejects_resealed_flag_and_artifact_contradictions() {
    let (bundle, snapshot, files, definitions) = fixture();
    for (index, replacement) in [
        (0, "rustc"),
        (1, "./other.rs"),
        (3, "other_crate"),
        (4, "--crate-type=bin"),
        (5, "--edition=2015"),
        (6, "--target=other"),
        (7, "-Cpanic=abort"),
        (8, "-Copt-level=3"),
        (9, "-Coverflow-checks=no"),
        (10, "-Zmir-opt-level=2"),
        (11, "--emit=link"),
        (13, "/private/sysroot"),
        (14, "--test"),
    ] {
        let mut changed = bundle.clone();
        changed.inputs.rustc_args[index] = replacement.into();
        seal(&mut changed);
        assert!(
            validate(&changed, &snapshot, &files, &definitions).is_err(),
            "accepted contradictory argument {replacement}"
        );
    }
    for extra in [
        "--test",
        "--extern=untrusted",
        "--cfg=test",
        "--target=other",
    ] {
        let mut changed = bundle.clone();
        changed.inputs.rustc_args.push(extra.into());
        seal(&mut changed);
        assert!(validate(&changed, &snapshot, &files, &definitions).is_err());
    }
    let mut changed = bundle.clone();
    changed.inputs.rustc_args.clear();
    seal(&mut changed);
    assert!(validate(&changed, &snapshot, &files, &definitions).is_err());
    changed = bundle.clone();
    changed.inputs.compiled_artifact = Some(CompilerArtifact {
        kind: "executable".into(),
        sha256: "a".repeat(64),
    });
    seal(&mut changed);
    assert!(validate(&changed, &snapshot, &files, &definitions).is_err());
    changed = bundle.clone();
    changed.compiler.adapter_version = "unknown".into();
    seal(&mut changed);
    assert!(validate(&changed, &snapshot, &files, &definitions).is_err());
}

#[test]
fn cfg_arguments_are_exact_sorted_deduplicated_and_context_pinned() {
    let (mut bundle, mut snapshot, files, definitions) = fixture();
    snapshot.context.features = vec!["fast".into()];
    snapshot.context.cfg.insert("firmware".into(), None);
    bundle
        .inputs
        .rustc_args
        .extend(["--cfg", "feature=\"fast\"", "--cfg", "firmware"].map(String::from));
    seal(&mut bundle);
    validate(&bundle, &snapshot, &files, &definitions).unwrap();
    let mut duplicate = bundle.clone();
    duplicate
        .inputs
        .rustc_args
        .extend(["--cfg", "firmware"].map(String::from));
    seal(&mut duplicate);
    assert!(validate(&duplicate, &snapshot, &files, &definitions).is_err());
    let mut reordered = bundle.clone();
    reordered.inputs.rustc_args.swap(16, 18);
    seal(&mut reordered);
    assert!(validate(&reordered, &snapshot, &files, &definitions).is_err());
    let mut missing = bundle;
    missing.inputs.rustc_args.truncate(17);
    seal(&mut missing);
    assert!(validate(&missing, &snapshot, &files, &definitions).is_err());
}

fn successor(kind: CompilerEdgeKind) -> CompilerSuccessor {
    CompilerSuccessor {
        target: 0,
        kind,
        switch_value: None,
    }
}
fn term(kind: &str) -> CompilerTerminator {
    let (bundle, ..) = fixture();
    let mut term = bundle.bodies[0].blocks[0].terminator.clone();
    term.kind = kind.into();
    term
}
fn known_call(kind: &str) -> CompilerTerminator {
    let mut value = term(kind);
    value.call_target = Some(CompilerCallTarget::FunctionDefinition {
        def_path: "fixture::callee".into(),
        is_local: true,
    });
    value.locals.unknown_effects = vec![CompilerUnknownEffect::Call];
    value
}

#[test]
fn supported_terminator_shapes_match_the_pinned_typed_adapter_contract() {
    use CompilerEdgeKind as Edge;
    let mut cases = ["return", "unwind_resume", "unreachable", "coroutine_drop"]
        .map(term)
        .to_vec();
    let mut value = term("goto");
    value.successors.push(successor(Edge::Normal));
    cases.push(value);
    value = term("switch_int");
    value.successors = vec![
        CompilerSuccessor {
            target: 0,
            kind: Edge::SwitchValue,
            switch_value: Some(u128::MAX.to_string()),
        },
        successor(Edge::Otherwise),
    ];
    cases.push(value);
    value = term("unwind_terminate");
    value.unwind = Some(CompilerUnwind::Terminate {
        reason: "abi".into(),
    });
    cases.push(value);
    value = term("drop");
    value.successors = vec![successor(Edge::Normal), successor(Edge::CoroutineDrop)];
    value.unwind = Some(CompilerUnwind::Continue);
    value.locals.unknown_effects = vec![CompilerUnknownEffect::Drop];
    cases.push(value);
    value = known_call("call");
    value.successors = vec![successor(Edge::Normal)];
    value.normal_return_defs = vec![0];
    value.unwind = Some(CompilerUnwind::Continue);
    cases.push(value);
    value = known_call("call");
    value.unwind = Some(CompilerUnwind::Unreachable);
    cases.push(value);
    cases.push(known_call("tail_call"));
    value = term("assert");
    value.successors = vec![successor(Edge::Normal)];
    value.unwind = Some(CompilerUnwind::Continue);
    value.assert_expected = Some(true);
    cases.push(value);
    value = term("yield");
    value.successors = vec![successor(Edge::Resume), successor(Edge::CoroutineDrop)];
    cases.push(value);
    value = term("false_edge");
    value.successors = vec![successor(Edge::Normal), successor(Edge::Imaginary)];
    cases.push(value);
    value = term("false_unwind");
    value.successors = vec![successor(Edge::Normal)];
    value.unwind = Some(CompilerUnwind::Unreachable);
    cases.push(value);
    value = term("inline_asm");
    value.successors = vec![successor(Edge::Normal), successor(Edge::Normal)];
    value.unwind = Some(CompilerUnwind::Continue);
    value.locals.unknown_effects = vec![CompilerUnknownEffect::InlineAssembly];
    cases.push(value);
    for candidate in cases {
        let (mut bundle, snapshot, files, definitions) = fixture();
        bundle.bodies[0].blocks[0].terminator = candidate;
        validate(&bundle, &snapshot, &files, &definitions).unwrap_or_else(|error| {
            panic!(
                "{} rejected: {error}",
                bundle.bodies[0].blocks[0].terminator.kind
            )
        });
    }
}

#[test]
fn malformed_kind_edge_metadata_and_conservative_effects_are_rejected() {
    use CompilerEdgeKind as Edge;
    let mut cases = vec![
        term("unsupported"),
        term("goto"),
        term("assert"),
        term("call"),
        term("switch_int"),
    ];
    let mut value = term("return");
    value.successors.push(successor(Edge::Normal));
    cases.push(value);
    value = term("return");
    value.assert_expected = Some(false);
    cases.push(value);
    value = term("return");
    value.normal_return_defs = vec![0];
    cases.push(value);
    value = known_call("return");
    cases.push(value);
    value = term("goto");
    value.successors = vec![CompilerSuccessor {
        target: 0,
        kind: Edge::Normal,
        switch_value: Some("1".into()),
    }];
    cases.push(value);
    value = term("switch_int");
    value.successors = vec![successor(Edge::SwitchValue), successor(Edge::Otherwise)];
    cases.push(value);
    value = term("switch_int");
    value.successors = vec![
        CompilerSuccessor {
            target: 0,
            kind: Edge::SwitchValue,
            switch_value: Some("01".into()),
        },
        successor(Edge::Otherwise),
    ];
    cases.push(value);
    value = term("switch_int");
    value.successors = vec![
        CompilerSuccessor {
            target: 0,
            kind: Edge::SwitchValue,
            switch_value: Some("1".into()),
        },
        CompilerSuccessor {
            target: 0,
            kind: Edge::SwitchValue,
            switch_value: Some("1".into()),
        },
        successor(Edge::Otherwise),
    ];
    cases.push(value);
    value = known_call("call");
    value.unwind = Some(CompilerUnwind::Continue);
    value.normal_return_defs = vec![0];
    cases.push(value);
    value = known_call("call");
    value.unwind = Some(CompilerUnwind::Continue);
    value.locals.unknown_effects.clear();
    cases.push(value);
    value = known_call("call");
    value.unwind = Some(CompilerUnwind::Continue);
    value.call_target = Some(CompilerCallTarget::Indirect {
        reason: "pointer".into(),
    });
    cases.push(value);
    value = term("goto");
    value.successors = vec![successor(Edge::Unwind)];
    cases.push(value);
    value = term("unwind_terminate");
    value.unwind = Some(CompilerUnwind::Terminate {
        reason: "unknown".into(),
    });
    cases.push(value);
    for candidate in cases {
        let (mut bundle, snapshot, files, definitions) = fixture();
        bundle.bodies[0].blocks[0].terminator = candidate;
        assert!(
            validate(&bundle, &snapshot, &files, &definitions).is_err(),
            "accepted {:?}",
            bundle.bodies[0].blocks[0].terminator
        );
    }
}

#[test]
fn cleanup_edges_are_bijective_with_unwind_metadata_and_cleanup_blocks() {
    let (mut bundle, snapshot, files, definitions) = fixture();
    let mut cleanup = bundle.bodies[0].blocks[0].clone();
    cleanup.index = 1;
    cleanup.is_cleanup = true;
    cleanup.terminator = term("unwind_resume");
    bundle.bodies[0].blocks.push(cleanup);
    let mut call = known_call("call");
    call.unwind = Some(CompilerUnwind::Cleanup { target: 1 });
    call.successors = vec![CompilerSuccessor {
        target: 1,
        kind: CompilerEdgeKind::Unwind,
        switch_value: None,
    }];
    bundle.bodies[0].blocks[0].terminator = call;
    validate(&bundle, &snapshot, &files, &definitions).unwrap();
    let mut duplicate = bundle.clone();
    let duplicate_edge = duplicate.bodies[0].blocks[0].terminator.successors[0].clone();
    duplicate.bodies[0].blocks[0]
        .terminator
        .successors
        .push(duplicate_edge);
    assert!(validate(&duplicate, &snapshot, &files, &definitions).is_err());
    let mut absent = bundle.clone();
    absent.bodies[0].blocks[0].terminator.unwind = None;
    assert!(validate(&absent, &snapshot, &files, &definitions).is_err());
    bundle.bodies[0].blocks[1].is_cleanup = false;
    assert!(validate(&bundle, &snapshot, &files, &definitions).is_err());
}

#[test]
fn local_roles_ordering_and_statement_variants_are_validated() {
    let (bundle, snapshot, files, definitions) = fixture();
    let mut bad = bundle.clone();
    bad.bodies[0].locals[0].role = "argument".into();
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle.clone();
    bad.bodies[0].blocks[0].terminator.locals.uses = vec![0, 0];
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle.clone();
    bad.bodies[0].kind = "unknown".into();
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
    bad = bundle;
    let span = bad.bodies[0].span.clone();
    bad.bodies[0].blocks[0].statements.push(CompilerStatement {
        index: 0,
        kind: "invented".into(),
        source_scope: 0,
        span,
        locals: Default::default(),
    });
    assert!(validate(&bad, &snapshot, &files, &definitions).is_err());
}

#[test]
#[ignore = "requires ATLAS_TEST_COMPILER pointing to the separately built pinned adapter"]
fn actual_pinned_adapter_fixture_passes_strict_import_contract() {
    let adapter = std::env::var_os("ATLAS_TEST_COMPILER").expect("set ATLAS_TEST_COMPILER");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/rustc/fixtures");
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("compiler.json");
    let extraction = std::process::Command::new(adapter)
        .args(["--trusted-local", "--root"])
        .arg(&root)
        .args([
            "--source",
            "control_flow.rs",
            "--crate-name",
            "fixture",
            "--output",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        extraction.status.success(),
        "{}",
        String::from_utf8_lossy(&extraction.stderr)
    );
    let bundle: CompilerBundle = serde_json::from_reader(File::open(output).unwrap()).unwrap();
    let (_, mut snapshot, _, _) = fixture();
    snapshot.context.crates[0].root_file = "control_flow.rs".into();
    let files = vec![SourceFile {
        id: FileId("file:control-flow".into()),
        path: "control_flow.rs".into(),
        content_hash: "independent-fixture".into(),
        text: fs::read_to_string(root.join("control_flow.rs")).unwrap(),
    }];
    let imported = validate(&bundle, &snapshot, &files, &[]).unwrap();
    assert!(imported.bundle.bodies.len() >= 7);
    assert!(
        imported
            .mappings
            .iter()
            .all(|mapping| matches!(mapping.mapping, CompilerDefinitionMapping::Unmapped { .. }))
    );
}
