use std::os::unix::fs::{PermissionsExt, symlink};

fn bounded_error(operation: impl FnOnce() -> Result<()> + Send + 'static) {
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = send.send(operation().is_err());
    });
    assert!(
        receive
            .recv_timeout(Duration::from_secs(1))
            .expect("evidence I/O blocked")
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
fn observation_object_symlinks_fifos_and_directories_are_rejected_without_blocking() {
    for kind in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("observations");
        let (snapshot, bundle, definitions) = fixture();
        let imported = restore(&root, &bundle, &snapshot, &definitions).unwrap();
        let path = scope(&root, &snapshot.id)
            .join(format!("{}.json", imported.id.split_once(':').unwrap().1));
        let saved = temp.path().join("saved.json");
        fs::rename(&path, &saved).unwrap();
        match kind {
            0 => symlink(&saved, &path).unwrap(),
            1 => fifo(&path),
            2 => fs::create_dir(&path).unwrap(),
            _ => unreachable!(),
        }
        let read_root = root.clone();
        let read_snapshot = snapshot.id.clone();
        let id = imported.id.clone();
        bounded_error(move || load(&read_root, &read_snapshot, &id).map(|_| ()));
        let read_root = root.clone();
        let read_snapshot = snapshot.id.clone();
        bounded_error(move || list(&read_root, &read_snapshot).map(|_| ()));
        bounded_error(move || restore(&root, &bundle, &snapshot, &definitions).map(|_| ()));
    }
}

#[test]
fn observation_root_scope_and_ancestor_symlinks_are_rejected() {
    for depth in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("parent/observations");
        let (snapshot, bundle, definitions) = fixture();
        let imported = restore(&root, &bundle, &snapshot, &definitions).unwrap();
        let replace = match depth {
            0 => root.clone(),
            1 => scope(&root, &snapshot.id),
            _ => root.parent().unwrap().to_owned(),
        };
        let saved = temp.path().join("saved");
        fs::rename(&replace, &saved).unwrap();
        symlink(&saved, &replace).unwrap();
        assert!(list(&root, &snapshot.id).is_err());
        assert!(load(&root, &snapshot.id, &imported.id).is_err());
        assert!(restore(&root, &bundle, &snapshot, &definitions).is_err());
    }
}

#[test]
fn observation_root_and_scope_non_directories_are_rejected_without_blocking() {
    for scope_only in [false, true] {
        for pipe in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("observations");
            let (snapshot, bundle, definitions) = fixture();
            let path = if scope_only {
                fs::create_dir(&root).unwrap();
                scope(&root, &snapshot.id)
            } else {
                root.clone()
            };
            if pipe {
                fifo(&path);
            } else {
                fs::write(&path, b"not a directory").unwrap();
            }
            let read_root = root.clone();
            let read_snapshot = snapshot.id.clone();
            bounded_error(move || list(&read_root, &read_snapshot).map(|_| ()));
            bounded_error(move || restore(&root, &bundle, &snapshot, &definitions).map(|_| ()));
        }
    }
}

#[test]
fn non_json_entries_count_toward_every_scan_budget() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("observations");
    let (snapshot, bundle, definitions) = fixture();
    let folder = scope(&root, &snapshot.id);
    fs::create_dir_all(&folder).unwrap();
    for index in 0..MAX_DIRECTORY_ENTRIES {
        fs::write(folder.join(format!("ignored-{index}")), b"").unwrap();
    }
    assert!(list(&root, &snapshot.id).unwrap().is_empty());
    fs::write(folder.join("one-too-many"), b"").unwrap();
    assert!(
        list(&root, &snapshot.id)
            .unwrap_err()
            .to_string()
            .contains("entry budget")
    );
    assert!(
        restore(&root, &bundle, &snapshot, &definitions)
            .unwrap_err()
            .to_string()
            .contains("entry budget")
    );
}

#[test]
fn import_locks_reject_special_nonempty_and_hardlinked_files() {
    for kind in 0..5 {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("observations");
        let (snapshot, bundle, definitions) = fixture();
        let folder = scope(&root, &snapshot.id);
        fs::create_dir_all(&folder).unwrap();
        let lock = folder.join(".import.lock");
        let saved = temp.path().join("saved");
        fs::write(&saved, b"").unwrap();
        match kind {
            0 => symlink(&saved, &lock).unwrap(),
            1 => fifo(&lock),
            2 => fs::create_dir(&lock).unwrap(),
            3 => fs::write(&lock, b"occupied").unwrap(),
            4 => fs::hard_link(&saved, &lock).unwrap(),
            _ => unreachable!(),
        }
        bounded_error(move || restore(&root, &bundle, &snapshot, &definitions).map(|_| ()));
    }
}

#[test]
fn contended_import_lock_expires_and_legacy_lock_permissions_become_private() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("observations");
    let (snapshot, bundle, definitions) = fixture();
    let summary = restore(&root, &bundle, &snapshot, &definitions).unwrap();
    let folder = scope(&root, &snapshot.id);
    let lock_path = folder.join(".import.lock");
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o666)).unwrap();
    restore(&root, &bundle, &snapshot, &definitions).unwrap();
    assert_eq!(
        fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let object = folder.join(format!("{}.json", summary.id.split_once(':').unwrap().1));
    assert_eq!(
        fs::metadata(object).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let lock = File::open(lock_path).unwrap();
    lock.lock().unwrap();
    let start = Instant::now();
    let error = restore(&root, &bundle, &snapshot, &definitions).unwrap_err();
    assert!(error.to_string().contains("lock deadline"));
    assert!(start.elapsed() >= IMPORT_LOCK_TIMEOUT && start.elapsed() < Duration::from_secs(4));
    drop(lock);
    assert_eq!(
        restore(&root, &bundle, &snapshot, &definitions).unwrap().id,
        summary.id
    );
}

#[test]
fn held_directory_descriptor_survives_path_replacement_for_reads_and_publication() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scope");
    let directory = ScopedDirectory::open(&path, true).unwrap().unwrap();
    fs::write(path.join("original.json"), b"original").unwrap();
    let moved = temp.path().join("moved");
    let attacker = temp.path().join("replacement");
    fs::create_dir(&attacker).unwrap();
    fs::write(attacker.join("original.json"), b"replacement").unwrap();
    fs::rename(&path, &moved).unwrap();
    symlink(&attacker, &path).unwrap();
    assert_eq!(
        directory.read(Path::new("original.json"), 100).unwrap(),
        b"original"
    );
    assert_eq!(
        directory.json_names(10).unwrap(),
        [PathBuf::from("original.json")]
    );
    assert!(directory.publish(Path::new("new.json"), b"new").unwrap());
    assert!(
        !directory
            .publish(Path::new("new.json"), b"overwrite")
            .unwrap()
    );
    assert_eq!(fs::read(moved.join("new.json")).unwrap(), b"new");
    assert!(!attacker.join("new.json").exists());
    assert!(
        directory
            .read(Path::new("../replacement/original.json"), 100)
            .is_err()
    );
}

#[test]
fn listing_missing_evidence_does_not_create_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("missing/evidence");
    let (snapshot, _, _) = fixture();
    assert!(list(&root, &snapshot.id).unwrap().is_empty());
    assert!(
        compiler::list(&root, &snapshot.id, &snapshot.context.id)
            .unwrap()
            .is_empty()
    );
    assert!(!temp.path().join("missing").exists());
}

#[test]
fn observation_sparse_oversize_and_trailing_bytes_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("observations");
    let (snapshot, bundle, definitions) = fixture();
    let imported = restore(&root, &bundle, &snapshot, &definitions).unwrap();
    let path =
        scope(&root, &snapshot.id).join(format!("{}.json", imported.id.split_once(':').unwrap().1));
    File::create(&path)
        .unwrap()
        .set_len(MAX_BUNDLE_BYTES + 1)
        .unwrap();
    assert!(
        load(&root, &snapshot.id, &imported.id)
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    let mut bytes = serde_json::to_vec(&bundle).unwrap();
    bytes.extend_from_slice(b"{}");
    fs::write(&path, bytes).unwrap();
    assert!(load(&root, &snapshot.id, &imported.id).is_err());
}
