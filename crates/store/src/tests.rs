use super::*;
use std::collections::BTreeMap;

pub(super) fn process_test_guard() -> std::sync::MutexGuard<'static, ()> {
    // Forked crash-test children inherit unrelated flock descriptors until exec,
    // even with CLOEXEC. Keep top-level fixtures separate; inner races stay concurrent.
    static PROCESS_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    PROCESS_TESTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn batch(revision: &str) -> FactBatch {
    let text = "fn entry() { leaf(); }\r\nfn leaf() {}\r\n// \u{03bb}\n".to_string();
    let file_id = FileId("file:test".into());
    let context = BuildContext {
        id: ContextId("context:test".into()),
        name: "test".into(),
        target: "host".into(),
        features: vec![],
        default_features: true,
        cfg: BTreeMap::new(),
        crates: vec![],
        manifest_digest: "manifest:test".into(),
        trust: "read_only".into(),
        coverage: Coverage::complete(),
    };
    let definition = |name: &str, start, end| Definition {
        id: DefinitionId(format!("definition:{name}")),
        context_id: context.id.clone(),
        file_id: file_id.clone(),
        name: name.into(),
        qualified_name: format!("test::{name}"),
        kind: "function".into(),
        parent_id: None,
        signature: format!("fn {name}()"),
        signature_hash: digest("signature", name),
        body_hash: digest("body", name),
        span: Span {
            file_id: file_id.clone(),
            start,
            end,
        },
        body_span: None,
        cfg: vec![],
        cfg_status: CfgStatus::Active,
        visibility: "private".into(),
        metrics: Metrics::default(),
    };
    let definitions = vec![definition("entry", 0, 22), definition("leaf", 24, 36)];
    FactBatch {
        schema_version: SCHEMA_VERSION,
        source: SourceSnapshot {
            id: SourceId(format!("source:{revision}")),
            repository_id: RepositoryId("repository:test".into()),
            revision: revision.into(),
            files: vec![SourceFile {
                id: file_id.clone(),
                path: "src/lib.rs".into(),
                content_hash: digest("content", &text),
                text,
            }],
            manifests: BTreeMap::new(),
            warnings: vec![],
        },
        context,
        producer: "fixture/1".into(),
        definitions,
        relations: vec![Relation {
            id: RelationId("relation:call".into()),
            source: DefinitionId("definition:entry".into()),
            target: Target::Resolved {
                id: DefinitionId("definition:leaf".into()),
            },
            kind: "calls".into(),
            span: Span {
                file_id,
                start: 13,
                end: 17,
            },
            evidence_id: EvidenceId("evidence:test".into()),
        }],
        evidence: vec![Evidence {
            id: EvidenceId("evidence:test".into()),
            basis: "resolved".into(),
            producer: "fixture/1".into(),
            inputs: vec![],
            assumptions: vec![],
            limitations: vec![],
        }],
        flows: vec![],
        coverage: Coverage::complete(),
        diagnostics: vec![],
    }
}

#[test]
fn publication_is_deterministic_and_sources_are_exact() {
    let _process_guard = crate::tests::process_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let first = store.publish(&batch("one"), "main", None).unwrap();
    let mut reordered = batch("one");
    reordered.definitions.reverse();
    let second = store.publish(&reordered, "main", Some(&first.id)).unwrap();
    assert_eq!(first, second);
    assert_eq!(store.publish(&reordered, "main", None).unwrap(), first);
    assert_eq!(store.snapshots().unwrap().len(), 1);
    let reader = store.reader(&first.id).unwrap();
    assert_eq!(
        reader
            .source(&FileId("file:test".into()))
            .unwrap()
            .unwrap()
            .text,
        batch("one").source.files[0].text
    );
    assert!(store.integrity_check().unwrap().valid);
    let forward = reader
        .adjacency(
            &DefinitionId("definition:entry".into()),
            &Direction::Outgoing,
            10,
        )
        .unwrap();
    let reverse = reader
        .adjacency(
            &DefinitionId("definition:leaf".into()),
            &Direction::Incoming,
            10,
        )
        .unwrap();
    assert_eq!(forward, reverse);
}

#[test]
fn private_store_creation_and_bounded_snapshot_enumeration() {
    let _process_guard = crate::tests::process_test_guard();
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private-store");
    let store = Store::open(&root).unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(root.join("catalog.sqlite"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let first = store.publish(&batch("one"), "main", None).unwrap();
    let _second = store
        .publish(&batch("two"), "main", Some(&first.id))
        .unwrap();
    assert!(matches!(
        store.snapshots_scoped_bounded(None, 1, 1024 * 1024),
        Err(Error::BudgetExhausted)
    ));
    assert!(matches!(
        store.snapshots_scoped_bounded(None, 1000, 10),
        Err(Error::BudgetExhausted)
    ));
    assert_eq!(store.snapshots().unwrap().len(), 2);
    assert!(
        store
            .snapshots_scoped_bounded(Some(&BTreeSet::new()), 0, 0)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store.reader_with_stop(&first.id, &|| true),
        Err(Error::BudgetExhausted)
    ));
    assert!(store.reader(&first.id).is_ok());
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
    Store::open(&existing).unwrap();
    assert_eq!(
        fs::metadata(existing).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[test]
fn every_failed_publication_boundary_keeps_old_head_visible() {
    let _process_guard = crate::tests::process_test_guard();
    for stage in 1..=4 {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let first = store.publish(&batch("one"), "main", None).unwrap();
        assert!(
            store
                .publish_inner(&batch("two"), "main", Some(&first.id), Some(stage))
                .is_err()
        );
        drop(store);
        let recovered = Store::open(temp.path()).unwrap();
        assert_eq!(recovered.head("main").unwrap(), Some(first.id));
        assert_eq!(recovered.snapshots().unwrap().len(), 1);
        assert!(recovered.integrity_check().unwrap().valid);
        assert!(recovered.gc_dry_run().unwrap().dry_run);
    }
}

#[test]
fn publication_crash_child() {
    let _process_guard = crate::tests::process_test_guard();
    let Ok(root) = std::env::var("ATLAS_TEST_CRASH_ROOT") else {
        return;
    };
    let stage = std::env::var("ATLAS_TEST_CRASH_STAGE")
        .unwrap()
        .parse::<u8>()
        .unwrap();
    let store = Store::open(root).unwrap();
    let expected = store.head("main").unwrap();
    store
        .publish_inner(&batch("two"), "main", expected.as_ref(), Some(stage))
        .unwrap();
    panic!("publication did not reach injected crash boundary");
}

#[test]
fn abrupt_process_death_exposes_only_whole_generations() {
    let _process_guard = crate::tests::process_test_guard();
    for stage in 1..=5 {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let before = store.publish(&batch("one"), "main", None).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::publication_crash_child"])
            .env("ATLAS_TEST_CRASH_ROOT", temp.path())
            .env("ATLAS_TEST_CRASH_STAGE", stage.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77), "stage {stage}");
        let recovered = Store::open(temp.path()).unwrap();
        assert!(recovered.integrity_check().unwrap().valid, "stage {stage}");
        let current = recovered.head("main").unwrap().unwrap();
        if stage < 5 {
            assert_eq!(current, before.id);
            assert_eq!(recovered.snapshots().unwrap().len(), 1);
        } else {
            assert_ne!(current, before.id);
            assert_eq!(recovered.snapshot(&current).unwrap().revision, "two");
            assert_eq!(recovered.snapshots().unwrap().len(), 2);
        }
        assert!(recovered.reader(&before.id).is_ok());
    }
}

#[test]
fn concurrent_writers_use_compare_and_swap() {
    let _process_guard = crate::tests::process_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let first = store.publish(&batch("one"), "main", None).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let writers: Vec<_> = (0..2)
        .map(|n| {
            let store = store.clone();
            let parent = first.id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.publish(&batch(&format!("next{n}")), "main", Some(&parent))
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(Error::Conflict)))
            .count(),
        1
    );
    assert_eq!(store.snapshots().unwrap().len(), 2);
    assert!(store.reader(&first.id).is_ok());
}

#[test]
fn validation_rejects_dangling_ids_invalid_spans_and_checksums() {
    let _process_guard = crate::tests::process_test_guard();
    let mut invalid = batch("one");
    invalid.relations[0].target = Target::Resolved {
        id: DefinitionId("missing".into()),
    };
    assert!(validate(&invalid).is_err());
    let mut invalid = batch("one");
    invalid.source.files[0].content_hash = "content:wrong".into();
    assert!(validate(&invalid).is_err());
    let mut invalid = batch("one");
    invalid.definitions[0].span.end = 1000;
    assert!(validate(&invalid).is_err());
    let mut invalid = batch("one");
    let offset = invalid.source.files[0].text.find('\u{03bb}').unwrap() as u32;
    invalid.definitions[0].span.start = offset + 1;
    invalid.definitions[0].span.end = offset + 2;
    assert!(validate(&invalid).is_err());
    let mut invalid = batch("one");
    invalid.definitions[0].parent_id = Some(invalid.definitions[0].id.clone());
    assert!(validate(&invalid).is_err());
    let mut invalid = batch("one");
    invalid.source.files[0].path = "../secret.rs".into();
    assert!(validate(&invalid).is_err());
}

#[test]
fn corrupt_and_missing_objects_are_never_replaced_by_older_snapshots() {
    let _process_guard = crate::tests::process_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let first = store.publish(&batch("one"), "main", None).unwrap();
    let source = &batch("one").source.files[0];
    let source_path = store
        .object_path(
            "sources",
            source.content_hash.strip_prefix("content:").unwrap(),
        )
        .unwrap();
    fs::remove_file(&source_path).unwrap();
    fs::write(&source_path, "wrong source").unwrap();
    assert!(store.reader(&first.id).unwrap().source(&source.id).is_err());
    assert!(!store.integrity_check().unwrap().valid);
    let hash: String = store
        .connect()
        .unwrap()
        .query_row(
            "SELECT shard_hash FROM snapshots WHERE id=?1",
            [first.id.0.clone()],
            |r| r.get(0),
        )
        .unwrap();
    fs::remove_file(store.object_path("shards", &hash).unwrap()).unwrap();
    assert!(store.reader(&first.id).is_err());
    assert_eq!(store.head("main").unwrap(), Some(first.id));
}

#[test]
fn unknown_schema_is_rejected_without_migration() {
    let _process_guard = crate::tests::process_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    store
        .connect()
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    assert!(matches!(
        Store::open(temp.path()),
        Err(Error::UnsupportedVersion(99))
    ));
}

#[test]
fn gc_reports_orphans_without_deleting_live_or_unpublished_objects() {
    let _process_guard = crate::tests::process_test_guard();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let first = store.publish(&batch("one"), "main", None).unwrap();
    let orphan = store.root.join("staging/orphan");
    fs::write(&orphan, "unfinished").unwrap();
    let report = store.gc_dry_run().unwrap();
    assert!(report.unreferenced_objects.is_empty());
    assert_eq!(report.unreferenced_bytes, 0);
    assert!(orphan.exists());
    assert!(store.reader(&first.id).is_ok());
}
