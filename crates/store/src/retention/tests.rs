use super::*;
use crate::tests::batch;

fn policy() -> RetentionPolicy {
    RetentionPolicy {
        keep_recent: 0,
        grace_period: Duration::ZERO,
        additional_roots: BTreeSet::new(),
    }
}

fn revisions(store: &Store) -> (Snapshot, Snapshot, Snapshot) {
    let first = store.publish(&batch("one"), "main", None).unwrap();
    let second = store
        .publish(&batch("two"), "main", Some(&first.id))
        .unwrap();
    let third = store
        .publish(&batch("three"), "main", Some(&second.id))
        .unwrap();
    (first, second, third)
}

#[test]
fn pins_heads_recent_grace_and_observations_are_retention_roots() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (first, second, third) = revisions(&store);
    store.pin(&first.id, "review").unwrap();
    assert_eq!(store.pins().unwrap()[0].snapshot_id, first.id);
    let retained = RetentionPolicy {
        keep_recent: 2,
        ..policy()
    };
    assert!(
        store
            .gc_plan(&retained)
            .unwrap()
            .remove_snapshots
            .is_empty()
    );
    let plan = store.gc_plan(&policy()).unwrap();
    assert_eq!(plan.remove_snapshots, vec![second.id.clone()]);
    assert!(store.unpin("review").unwrap());
    assert!(!store.unpin("review").unwrap());
    assert!(
        store
            .gc_plan(&RetentionPolicy::default())
            .unwrap()
            .remove_snapshots
            .is_empty()
    );
    let grace_only = RetentionPolicy {
        grace_period: Duration::from_secs(86400),
        ..policy()
    };
    assert!(
        store
            .gc_plan(&grace_only)
            .unwrap()
            .remove_snapshots
            .is_empty()
    );
    let additional_root = RetentionPolicy {
        additional_roots: BTreeSet::from([first.id.clone()]),
        ..policy()
    };
    assert_eq!(
        store.gc_plan(&additional_root).unwrap().remove_snapshots,
        vec![second.id.clone()]
    );
    let scope = digest("observations", &first.id);
    fs::create_dir_all(
        store
            .root
            .join("observations")
            .join(scope.split_once(':').unwrap().1),
    )
    .unwrap();
    let plan = store.gc_plan(&policy()).unwrap();
    assert_eq!(plan.remove_snapshots, vec![second.id]);
    assert!(
        plan.retained_snapshots
            .iter()
            .any(|entry| entry.id == first.id && entry.reasons.contains(&"observations".into()))
    );
    assert!(
        plan.retained_snapshots
            .iter()
            .any(|entry| entry.id == third.id)
    );
}

#[test]
fn reader_lifetime_blocks_collection_across_independent_store_handles() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (first, _, _) = revisions(&store);
    let other = Store::open(temp.path()).unwrap();
    let plan = other.gc_plan(&policy()).unwrap();
    let reader = store.reader(&first.id).unwrap();
    assert!(
        other
            .gc_execute(&plan)
            .unwrap_err()
            .to_string()
            .contains("store busy")
    );
    assert!(reader.files().is_ok());
    drop(reader);
    other.gc_execute(&plan).unwrap();
    assert!(matches!(
        store.reader(&first.id),
        Err(Error::UnknownSnapshot)
    ));
}

#[test]
fn gc_keeps_shared_sources_and_unregistered_files_and_retries_idempotently() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (first, second, third) = revisions(&store);
    let unknown = store.root.join("sources").join("f".repeat(64));
    fs::write(&unknown, b"not owned").unwrap();
    let staging = store.root.join("staging/user-file");
    fs::write(&staging, b"not owned").unwrap();
    let plan = store.gc_plan(&policy()).unwrap();
    assert_eq!(
        plan.objects
            .iter()
            .filter(|object| object.category == "sources")
            .count(),
        0
    );
    assert_eq!(
        plan.objects
            .iter()
            .filter(|object| object.category == "shards")
            .count(),
        2
    );
    let report = store.gc_execute(&plan).unwrap();
    assert!(!report.dry_run);
    assert_eq!(
        report.unreferenced_bytes,
        plan.category_bytes.values().sum::<u64>()
    );
    assert_eq!(
        store.gc_execute(&plan).unwrap().plan_fingerprint,
        report.plan_fingerprint
    );
    assert!(unknown.exists() && staging.exists());
    assert!(matches!(
        store.snapshot(&first.id),
        Err(Error::UnknownSnapshot)
    ));
    assert!(matches!(
        store.snapshot(&second.id),
        Err(Error::UnknownSnapshot)
    ));
    assert!(
        store
            .reader(&third.id)
            .unwrap()
            .source(&batch("three").source.files[0].id)
            .unwrap()
            .is_some()
    );
    assert!(store.integrity_check().unwrap().valid);
}

#[test]
fn changed_heads_or_pins_invalidate_an_exact_gc_plan() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (first, _, third) = revisions(&store);
    let plan = store.gc_plan(&policy()).unwrap();
    store.pin(&first.id, "new-pin").unwrap();
    assert!(
        store
            .gc_execute(&plan)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    assert!(store.reader(&first.id).is_ok());
    store.unpin("new-pin").unwrap();
    let plan = store.gc_plan(&policy()).unwrap();
    store
        .publish(&batch("four"), "main", Some(&third.id))
        .unwrap();
    assert!(
        store
            .gc_execute(&plan)
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
}

#[test]
fn garbage_collection_refuses_symlink_directories_and_replaced_owned_objects() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    revisions(&store);
    let plan = store.gc_plan(&policy()).unwrap();
    let object = &plan.objects[0];
    let path = store.object_path(&object.category, &object.hash).unwrap();
    fs::remove_file(&path).unwrap();
    let target = outside.path().join("keep");
    fs::write(&target, b"private").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(store.gc_execute(&plan).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"private");
    let sources = store.root.join("sources");
    fs::rename(&sources, store.root.join("saved-sources")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &sources).unwrap();
    assert!(Store::open(temp.path()).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"private");
}

#[test]
fn collection_recovers_after_catalog_commit_or_unlink_interruption() {
    for stage in [1, 2] {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let (_, _, third) = revisions(&store);
        let plan = store.gc_plan(&policy()).unwrap();
        assert!(store.gc_execute_inner(&plan, Some(stage)).is_err());
        drop(store);
        let recovered = Store::open(temp.path()).unwrap();
        let retry = recovered.gc_plan(&policy()).unwrap();
        recovered.gc_execute(&retry).unwrap();
        assert_eq!(recovered.snapshots().unwrap().len(), 1);
        assert_eq!(recovered.head("main").unwrap(), Some(third.id));
        assert!(recovered.integrity_check().unwrap().valid);
        assert!(recovered.gc_plan(&policy()).unwrap().objects.is_empty());
    }
}

#[test]
fn lease_child_process() {
    let Ok(root) = std::env::var("ATLAS_TEST_LEASE_ROOT") else {
        return;
    };
    let store = Store::open(&root).unwrap();
    let id = store.snapshots().unwrap()[0].id.clone();
    let _reader = store.reader(&id).unwrap();
    fs::write(Path::new(&root).join("lease-ready"), b"ready").unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn process_death_releases_reader_lease_without_expiring_live_readers() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    revisions(&store);
    let plan = store.gc_plan(&policy()).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "retention::tests::lease_child_process",
            "--nocapture",
        ])
        .env("ATLAS_TEST_LEASE_ROOT", temp.path())
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    while !temp.path().join("lease-ready").exists() {
        if started.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("child did not acquire lease");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        store
            .gc_execute(&plan)
            .unwrap_err()
            .to_string()
            .contains("store busy")
    );
    child.kill().unwrap();
    child.wait().unwrap();
    store.gc_execute(&plan).unwrap();
    assert!(store.integrity_check().unwrap().valid);
}

#[test]
fn gc_crash_child_process() {
    let Ok(root) = std::env::var("ATLAS_TEST_GC_ROOT") else {
        return;
    };
    let stage = std::env::var("ATLAS_TEST_GC_CRASH_STAGE")
        .unwrap()
        .parse()
        .unwrap();
    let store = Store::open(root).unwrap();
    let plan = store.gc_plan(&policy()).unwrap();
    store.gc_execute_inner(&plan, Some(stage)).unwrap();
    panic!("expected process termination");
}

#[test]
fn abrupt_gc_process_death_recovers_after_metadata_commit_and_unlink() {
    for stage in [1, 2] {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let (_, _, third) = revisions(&store);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "retention::tests::gc_crash_child_process",
                "--nocapture",
            ])
            .env("ATLAS_TEST_GC_ROOT", temp.path())
            .env("ATLAS_TEST_GC_CRASH_STAGE", stage.to_string())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(78));
        let recovered = Store::open(temp.path()).unwrap();
        recovered
            .gc_execute(&recovered.gc_plan(&policy()).unwrap())
            .unwrap();
        assert_eq!(recovered.head("main").unwrap(), Some(third.id));
        assert!(recovered.integrity_check().unwrap().valid);
        assert!(recovered.gc_plan(&policy()).unwrap().objects.is_empty());
    }
}

#[test]
fn duplicate_collectors_and_publishers_cannot_delete_live_objects() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (_, _, third) = revisions(&store);
    let plan = store.gc_plan(&policy()).unwrap();
    let lease = store.lease(false, &|| false).unwrap();
    assert!(
        store
            .gc_execute(&plan)
            .unwrap_err()
            .to_string()
            .contains("store busy")
    );
    drop(lease);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let plan = plan.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.gc_execute(&plan)
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(results.iter().any(Result::is_ok));
    for error in results.into_iter().filter_map(Result::err) {
        assert!(error.to_string().contains("store busy"));
    }
    store
        .publish(&batch("four"), "main", Some(&third.id))
        .unwrap();
    assert!(store.integrity_check().unwrap().valid);
}

#[test]
fn concurrent_publish_and_gc_preserve_the_new_head() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (_, _, third) = revisions(&store);
    let plan = store.gc_plan(&policy()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let publisher = {
        let store = store.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            store.publish(&batch("four"), "main", Some(&third.id))
        })
    };
    barrier.wait();
    if let Err(error) = store.gc_execute(&plan) {
        assert!(error.to_string().contains("store busy") || error.to_string().contains("stale"));
    }
    let published = publisher.join().unwrap().unwrap();
    assert_eq!(store.head("main").unwrap(), Some(published.id));
    assert!(store.integrity_check().unwrap().valid);
}

#[test]
fn recovery_rechecks_objects_republished_after_interrupted_collection() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let (first, _, third) = revisions(&store);
    let plan = store.gc_plan(&policy()).unwrap();
    assert!(store.gc_execute_inner(&plan, Some(1)).is_err());
    let republished = store
        .publish(&batch("one"), "main", Some(&third.id))
        .unwrap();
    assert_eq!(republished.id, first.id);
    store
        .gc_execute(&store.gc_plan(&policy()).unwrap())
        .unwrap();
    assert!(store.reader(&first.id).is_ok());
    assert!(store.integrity_check().unwrap().valid);
}
