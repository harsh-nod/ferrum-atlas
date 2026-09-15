use std::{
    os::unix::fs::{PermissionsExt, symlink},
    time::{Duration, Instant},
};

fn bounded_error(operation: impl FnOnce() -> Result<()> + Send + 'static) {
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = send.send(operation().is_err());
    });
    assert!(
        receive
            .recv_timeout(Duration::from_secs(1))
            .expect("compiler evidence I/O blocked")
    );
    worker.join().unwrap();
}

fn fifo(path: &Path) {
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        path,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
}

#[test]
fn compiler_object_symlinks_fifos_and_directories_are_rejected_without_blocking() {
    for kind in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("compiler");
        let (bundle, snapshot, files, definitions) = fixture();
        let imported = import(&root, &bundle, &snapshot, &files, &definitions).unwrap();
        let object = folder(&root, &snapshot.id)
            .join(format!("{}.json", imported.id.split_once(':').unwrap().1));
        let saved = temp.path().join("saved.json");
        fs::rename(&object, &saved).unwrap();
        match kind {
            0 => symlink(&saved, &object).unwrap(),
            1 => fifo(&object),
            2 => fs::create_dir(&object).unwrap(),
            _ => unreachable!(),
        }
        let read_root = root.clone();
        let read_snapshot = snapshot.clone();
        bounded_error(move || {
            list(&read_root, &read_snapshot.id, &read_snapshot.context.id).map(|_| ())
        });
        let read_root = root.clone();
        let read_snapshot = snapshot.clone();
        let definition = definitions[0].id.clone();
        bounded_error(move || {
            flow(
                &read_root,
                &read_snapshot.id,
                &read_snapshot.context.id,
                &definition,
                &imported.id,
                (0, 10),
            )
            .map(|_| ())
        });
        bounded_error(move || import(&root, &bundle, &snapshot, &files, &definitions).map(|_| ()));
    }
}

#[test]
fn compiler_root_scope_and_ancestor_symlinks_are_rejected() {
    for depth in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("parent/compiler");
        let (bundle, snapshot, files, definitions) = fixture();
        let imported = import(&root, &bundle, &snapshot, &files, &definitions).unwrap();
        let replace = match depth {
            0 => root.clone(),
            1 => folder(&root, &snapshot.id),
            _ => root.parent().unwrap().to_owned(),
        };
        let saved = temp.path().join("saved");
        fs::rename(&replace, &saved).unwrap();
        symlink(&saved, &replace).unwrap();
        assert!(list(&root, &snapshot.id, &snapshot.context.id).is_err());
        assert!(
            flow(
                &root,
                &snapshot.id,
                &snapshot.context.id,
                &definitions[0].id,
                &imported.id,
                (0, 10)
            )
            .is_err()
        );
        assert!(import(&root, &bundle, &snapshot, &files, &definitions).is_err());
    }
}

#[test]
fn compiler_non_directory_roots_scopes_and_special_locks_fail_promptly() {
    for component in 0..3 {
        for pipe in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("compiler");
            let (bundle, snapshot, files, definitions) = fixture();
            let path = match component {
                0 => root.clone(),
                1 => {
                    fs::create_dir(&root).unwrap();
                    folder(&root, &snapshot.id)
                }
                _ => {
                    let scope = folder(&root, &snapshot.id);
                    fs::create_dir_all(&scope).unwrap();
                    scope.join(".import.lock")
                }
            };
            if pipe {
                fifo(&path);
            } else {
                fs::write(&path, b"not a directory or empty lock").unwrap();
            }
            bounded_error(move || {
                import(&root, &bundle, &snapshot, &files, &definitions).map(|_| ())
            });
        }
    }
}

#[test]
fn compiler_counts_non_json_entries_and_rejects_oversize_objects() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("compiler");
    let (bundle, snapshot, files, definitions) = fixture();
    let imported = import(&root, &bundle, &snapshot, &files, &definitions).unwrap();
    let scope = folder(&root, &snapshot.id);
    let object = scope.join(format!("{}.json", imported.id.split_once(':').unwrap().1));
    File::create(&object)
        .unwrap()
        .set_len(MAX_BYTES + 1024 * 1024 + 1)
        .unwrap();
    assert!(
        list(&root, &snapshot.id, &snapshot.context.id)
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    fs::remove_file(&object).unwrap();
    for index in 0..128 {
        fs::write(scope.join(format!("ignored-{index}")), b"").unwrap();
    }
    assert!(
        list(&root, &snapshot.id, &snapshot.context.id)
            .unwrap_err()
            .to_string()
            .contains("entry budget")
    );
    assert!(
        import(&root, &bundle, &snapshot, &files, &definitions)
            .unwrap_err()
            .to_string()
            .contains("entry budget")
    );
}

#[test]
fn compiler_import_lock_and_object_are_private_and_contention_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("compiler");
    let (bundle, snapshot, files, definitions) = fixture();
    let imported = import(&root, &bundle, &snapshot, &files, &definitions).unwrap();
    let scope = folder(&root, &snapshot.id);
    let lock_path = scope.join(".import.lock");
    let object = scope.join(format!("{}.json", imported.id.split_once(':').unwrap().1));
    for path in [&lock_path, &object] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let lock = File::open(&lock_path).unwrap();
    lock.lock().unwrap();
    let start = Instant::now();
    assert!(
        import(&root, &bundle, &snapshot, &files, &definitions)
            .unwrap_err()
            .to_string()
            .contains("lock deadline")
    );
    assert!(start.elapsed() >= Duration::from_secs(2) && start.elapsed() < Duration::from_secs(4));
    drop(lock);
    fs::remove_file(&lock_path).unwrap();
    symlink(&object, &lock_path).unwrap();
    assert!(import(&root, &bundle, &snapshot, &files, &definitions).is_err());
}

#[test]
fn concurrent_compiler_imports_cannot_overfill_the_snapshot_limit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("compiler");
    let (bundle, snapshot, files, definitions) = fixture();
    for index in 0..15 {
        let mut candidate = bundle.clone();
        candidate.limitations = vec![format!("run-{index}")];
        import(&root, &candidate, &snapshot, &files, &definitions).unwrap();
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|index| {
            let root = root.clone();
            let snapshot = snapshot.clone();
            let files = files.clone();
            let definitions = definitions.clone();
            let barrier = barrier.clone();
            let mut candidate = bundle.clone();
            candidate.limitations = vec![format!("concurrent-{index}")];
            std::thread::spawn(move || {
                barrier.wait();
                import(&root, &candidate, &snapshot, &files, &definitions).is_ok()
            })
        })
        .collect();
    barrier.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|success| *success)
            .count(),
        1
    );
    assert_eq!(
        list(&root, &snapshot.id, &snapshot.context.id)
            .unwrap()
            .len(),
        16
    );
}
