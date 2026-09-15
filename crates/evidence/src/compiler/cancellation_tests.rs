use std::cell::Cell;

fn assert_cancelled<T: std::fmt::Debug>(result: Result<T>) {
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("evidence operation cancelled")
    );
}

#[test]
fn cancelled_compiler_queries_do_not_read_or_return_an_empty_missing_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("does-not-exist");
    let (_, snapshot, _, definitions) = fixture();
    let id = format!("compiler-import:{}", "0".repeat(64));
    assert_cancelled(flow_with_stop(
        &root,
        &snapshot.id,
        &snapshot.context.id,
        &definitions[0].id,
        &id,
        (0, 1),
        &|| true,
    ));
    assert_cancelled(list_with_stop(
        &root,
        &snapshot.id,
        &snapshot.context.id,
        &|| true,
    ));
    assert!(!root.exists());
    assert!(
        list(&root, &snapshot.id, &snapshot.context.id)
            .unwrap()
            .is_empty()
    );
    let checkpoints = Cell::new(0);
    assert_cancelled(list_with_stop(
        &root,
        &snapshot.id,
        &snapshot.context.id,
        &|| {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() == 2
        },
    ));
    assert_eq!(checkpoints.get(), 2);
}

#[test]
fn compiler_read_stops_during_chunks_and_after_parse_before_identity() {
    let temp = tempfile::tempdir().unwrap();
    let (mut bundle, snapshot, files, definitions) = fixture();
    bundle.limitations.push("bounded payload ".repeat(20_000));
    let imported = import(temp.path(), &bundle, &snapshot, &files, &definitions).unwrap();
    let directory = ScopedDirectory::open(&folder(temp.path(), &snapshot.id), false)
        .unwrap()
        .unwrap();
    let name = PathBuf::from(format!("{}.json", imported.id.split_once(':').unwrap().1));
    let checkpoints = Cell::new(0);
    let bytes = directory
        .read_with_stop(&name, MAX_BYTES + 1024 * 1024, &|| {
            checkpoints.set(checkpoints.get() + 1);
            false
        })
        .unwrap();
    let read_checks = checkpoints.get();
    assert!(bytes.len() > 128 * 1024);
    assert!(read_checks > 9);
    // The reader surrounds the chunked read and bounded serde parse with checks.
    // These stops cover a later read chunk, before parsing, and just after parsing.
    for stop_at in [8, read_checks + 2, read_checks + 3] {
        let checkpoints = Cell::new(0);
        assert_cancelled(read_from_with_stop(&directory, &name, &|| {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() >= stop_at
        }));
        assert_eq!(checkpoints.get(), stop_at);
    }
    assert_eq!(
        read_from(&directory, &name).unwrap().snapshot_id,
        snapshot.id
    );
    fs::write(
        folder(temp.path(), &snapshot.id).join(&name),
        b"malformed object",
    )
    .unwrap();
    assert_cancelled(read_from_with_stop(&directory, &name, &|| true));
    assert!(read_from_with_stop(&directory, &name, &|| false).is_err());
}

#[test]
fn streaming_compiler_identity_matches_canonical_escaped_and_large_objects() {
    let (bundle, snapshot, files, definitions) = fixture();
    let mut value = validate(&bundle, &snapshot, &files, &definitions).unwrap();
    value
        .coverage
        .limitations
        .push("Unicode \u{96ea}\u{1f600}\r\n\t\0\"\\".repeat(10_000));
    assert_eq!(
        import_identity_with_stop(&value, &|| false).unwrap(),
        digest("compiler-import", &value)
    );
    assert_cancelled(import_identity_with_stop(&value, &|| true));
    let checkpoints = Cell::new(0);
    import_identity_with_stop(&value, &|| {
        checkpoints.set(checkpoints.get() + 1);
        false
    })
    .unwrap();
    let total = checkpoints.get();
    assert!(total > 20);
    for stop_at in [2, total / 2, total - 1, total] {
        let checkpoints = Cell::new(0);
        assert_cancelled(import_identity_with_stop(&value, &|| {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() >= stop_at
        }));
    }
}

#[test]
fn every_compiler_flow_checkpoint_rejects_partial_pages_and_retry_preserves_window() {
    let temp = tempfile::tempdir().unwrap();
    let (mut bundle, snapshot, files, definitions) = fixture();
    let original = bundle.bodies[0].blocks[0].clone();
    bundle.bodies[0].blocks = (0..4)
        .map(|index| CompilerBlock {
            index,
            ..original.clone()
        })
        .collect();
    let imported = import(temp.path(), &bundle, &snapshot, &files, &definitions).unwrap();
    let load = |stopped: &dyn Fn() -> bool| {
        flow_with_stop(
            temp.path(),
            &snapshot.id,
            &snapshot.context.id,
            &definitions[0].id,
            &imported.id,
            (1, 2),
            stopped,
        )
    };
    let checkpoints = Cell::new(0);
    let page = load(&|| {
        checkpoints.set(checkpoints.get() + 1);
        false
    })
    .unwrap();
    let total = checkpoints.get();
    assert!(total > 20);
    for stop_at in 1..=total {
        let checkpoints = Cell::new(0);
        assert_cancelled(load(&|| {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() >= stop_at
        }));
    }
    assert_eq!(
        page.body
            .blocks
            .iter()
            .map(|block| block.index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(page.next_offset, Some(3));
    assert_eq!(page.total_blocks, 4);
    let legacy = flow(
        temp.path(),
        &snapshot.id,
        &snapshot.context.id,
        &definitions[0].id,
        &imported.id,
        (1, 2),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(page).unwrap(),
        serde_json::to_value(legacy).unwrap()
    );
}

#[test]
fn every_compiler_list_checkpoint_rejects_partial_summaries_and_retry_is_compatible() {
    let temp = tempfile::tempdir().unwrap();
    let (mut bundle, snapshot, files, definitions) = fixture();
    import(temp.path(), &bundle, &snapshot, &files, &definitions).unwrap();
    bundle.limitations.push("second recorded import".into());
    import(temp.path(), &bundle, &snapshot, &files, &definitions).unwrap();
    let load = |stopped: &dyn Fn() -> bool| {
        list_with_stop(temp.path(), &snapshot.id, &snapshot.context.id, stopped)
    };
    let checkpoints = Cell::new(0);
    let summaries = load(&|| {
        checkpoints.set(checkpoints.get() + 1);
        false
    })
    .unwrap();
    let total = checkpoints.get();
    assert_eq!(summaries.len(), 2);
    for stop_at in 1..=total {
        let checkpoints = Cell::new(0);
        assert_cancelled(load(&|| {
            checkpoints.set(checkpoints.get() + 1);
            checkpoints.get() >= stop_at
        }));
    }
    assert_eq!(
        serde_json::to_value(summaries).unwrap(),
        serde_json::to_value(list(temp.path(), &snapshot.id, &snapshot.context.id).unwrap())
            .unwrap()
    );
}
