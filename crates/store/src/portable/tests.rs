use super::*;
use crate::tests::batch;

fn complete_fixture() -> FactBatch {
    let mut facts = batch("portable");
    facts.source.manifests.insert(
        "Cargo.toml".into(),
        "[package]\nname = \"fixture\"\n".into(),
    );
    facts
        .source
        .warnings
        .push("Source capture limitation".into());
    facts.diagnostics.push(Diagnostic {
        path: Some("src/lib.rs".into()),
        severity: "warning".into(),
        message: "Fixture diagnostic".into(),
    });
    facts.flows.push(FunctionFlow {
        definition_id: facts.definitions[0].id.clone(),
        phase: "source".into(),
        points: vec![FlowPoint {
            kind: "call".into(),
            label: "leaf()".into(),
            span: facts.relations[0].span.clone(),
        }],
        coverage: Coverage::complete(),
    });
    facts
}

fn export(temp: &tempfile::TempDir) -> (Store, Snapshot, PathBuf) {
    let store = Store::open(temp.path().join("source-store")).unwrap();
    let snapshot = store.publish(&complete_fixture(), "main", None).unwrap();
    let directory = temp.path().join("portable");
    store.export_directory(&snapshot.id, &directory).unwrap();
    (store, snapshot, directory)
}

#[test]
fn complete_directory_round_trip_preserves_all_facts_source_and_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let (_, snapshot, directory) = export(&temp);
    let restored = Store::open(temp.path().join("restored")).unwrap();
    let imported = restored
        .import_directory(&directory, "main", None, |_, _, _| Ok(()))
        .unwrap();
    assert_eq!(imported.snapshot, snapshot);
    assert_eq!(
        imported.reader.complete_batch(FACT_BYTES as usize).unwrap(),
        validate::normalize(&complete_fixture())
    );
    assert!(restored.gc_plan(&RetentionPolicy::default()).is_err());
    drop(imported);
    let duplicate = restored
        .import_directory(&directory, "main", None, |_, _, _| Ok(()))
        .unwrap();
    assert_eq!(duplicate.snapshot, snapshot);
    assert_eq!(restored.snapshots().unwrap().len(), 1);
    assert!(restored.integrity_check().unwrap().valid);
    let names: BTreeSet<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "manifest.json".into(),
            "snapshot.json".into(),
            "facts.json".into()
        ])
    );
}

#[test]
fn archive_includes_artifact_matched_observations_and_requires_validation_before_publish() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("source-store")).unwrap();
    let snapshot = store.publish(&complete_fixture(), "main", None).unwrap();
    let bundle = ObservationBundle {
        schema_version: SCHEMA_VERSION,
        snapshot_id: snapshot.id.clone(),
        artifact: ArtifactIdentity {
            sha256: "a".repeat(64),
            source_id: snapshot.source_id.clone(),
            context_id: snapshot.context.id.clone(),
            producer: "test-fixture".into(),
        },
        tests: vec![TestObservation {
            name: "wide counter".into(),
            outcome: TestOutcome::Pass,
            elapsed_ns: Some("9007199254740993".into()),
            timeout_ns: None,
            reason: None,
            definition_ids: vec![complete_fixture().definitions[0].id.clone()],
        }],
        streams: vec![],
        limitations: vec!["Imported observations are not exhaustive".into()],
    };
    let scope = digest("observations", &snapshot.id);
    let folder = store
        .root
        .join("observations")
        .join(scope.split_once(':').unwrap().1);
    fs::create_dir_all(&folder).unwrap();
    let hash = digest("observation", &bundle);
    fs::write(
        folder.join(format!("{}.json", hash.split_once(':').unwrap().1)),
        serde_json::to_vec(&bundle).unwrap(),
    )
    .unwrap();
    let directory = temp.path().join("portable");
    let manifest = store.export_directory(&snapshot.id, &directory).unwrap();
    assert_eq!(manifest.files.len(), 3);
    let restored = Store::open(temp.path().join("restored")).unwrap();
    assert!(
        restored
            .import_directory(&directory, "main", None, |_, _, _| Err(Error::Invalid(
                "rejected observation".into()
            )))
            .is_err()
    );
    assert!(restored.snapshots().unwrap().is_empty());
    let imported = restored
        .import_directory(
            &directory,
            "main",
            None,
            |metadata, definitions, bundles| {
                assert_eq!(metadata.id, snapshot.id);
                assert_eq!(bundles.len(), 1);
                assert!(definitions.contains(&bundles[0].tests[0].definition_ids[0]));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        imported.observations[0].tests[0].elapsed_ns.as_deref(),
        Some("9007199254740993")
    );
}

fn change_artifact(directory: &Path, name: &str, bytes: &[u8]) {
    fs::write(directory.join(name), bytes).unwrap();
    let mut manifest: PortableManifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let file = manifest
        .files
        .iter_mut()
        .find(|file| file.name == name)
        .unwrap();
    file.sha256 = raw_digest(bytes);
    file.bytes = bytes.len() as u64;
    fs::write(
        directory.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

#[test]
fn corrupt_sources_or_forged_fact_identity_never_publish() {
    for forge_manifest in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let (_, _, directory) = export(&temp);
        let mut facts = complete_fixture();
        facts.source.files[0].text.push_str("// tampered\n");
        facts.source.files[0].content_hash = digest("content", &facts.source.files[0].text);
        let bytes = serde_json::to_vec(&facts).unwrap();
        if forge_manifest {
            change_artifact(&directory, "facts.json", &bytes);
        } else {
            fs::write(directory.join("facts.json"), bytes).unwrap();
        }
        let restored = Store::open(temp.path().join("restored")).unwrap();
        assert!(
            restored
                .import_directory(&directory, "main", None, |_, _, _| Ok(()))
                .is_err()
        );
        assert!(restored.snapshots().unwrap().is_empty());
        assert!(restored.head("main").unwrap().is_none());
    }
}

#[test]
fn archive_paths_symlinks_versions_and_duplicate_entries_are_rejected() {
    for mutation in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let (_, _, directory) = export(&temp);
        let mut manifest: PortableManifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        match mutation {
            0 => manifest.files[0].name = "../outside.json".into(),
            1 => {
                let path = directory.join("facts.json");
                let target = temp.path().join("outside.json");
                fs::rename(&path, &target).unwrap();
                std::os::unix::fs::symlink(&target, &path).unwrap();
            }
            2 => manifest.version = 99,
            _ => manifest.files.push(manifest.files[0].clone()),
        }
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let restored = Store::open(temp.path().join("restored")).unwrap();
        assert!(
            restored
                .import_directory(&directory, "main", None, |_, _, _| Ok(()))
                .is_err()
        );
        assert!(restored.snapshots().unwrap().is_empty());
    }
}

#[test]
fn export_refuses_existing_destinations_and_import_obeys_head_cas() {
    let temp = tempfile::tempdir().unwrap();
    let (store, snapshot, directory) = export(&temp);
    let marker = directory.join("keep");
    fs::write(&marker, b"user file").unwrap();
    assert!(store.export_directory(&snapshot.id, &directory).is_err());
    assert_eq!(fs::read(marker).unwrap(), b"user file");
    let restored = Store::open(temp.path().join("restored")).unwrap();
    let existing = restored.publish(&batch("existing"), "main", None).unwrap();
    assert!(matches!(
        restored.import_directory(&directory, "main", None, |_, _, _| Ok(())),
        Err(Error::Conflict)
    ));
    assert_eq!(restored.head("main").unwrap(), Some(existing.id));
    assert_eq!(restored.snapshots().unwrap().len(), 1);
}

#[test]
fn legacy_shards_remain_readable_but_cannot_claim_complete_export() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("source-store")).unwrap();
    let snapshot = store.publish(&complete_fixture(), "main", None).unwrap();
    let connection = store.connect().unwrap();
    let original: String = connection
        .query_row(
            "SELECT shard_hash FROM snapshots WHERE id=?1",
            [&snapshot.id.0],
            |row| row.get(0),
        )
        .unwrap();
    let legacy = tempfile::NamedTempFile::new_in(store.root.join("staging")).unwrap();
    fs::copy(
        store.object_path("shards", &original).unwrap(),
        legacy.path(),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(legacy.path(), fs::Permissions::from_mode(0o600)).unwrap();
    let shard = Connection::open(legacy.path()).unwrap();
    shard
        .execute("DELETE FROM metadata WHERE key='fact_envelope'", [])
        .unwrap();
    drop(shard);
    let hash = checksum(legacy.path()).unwrap();
    persist_immutable(legacy, &store.object_path("shards", &hash).unwrap(), &hash).unwrap();
    store.register_object("shards", &hash).unwrap();
    connection
        .execute(
            "UPDATE snapshots SET shard_hash=?1 WHERE id=?2",
            params![hash, snapshot.id.0],
        )
        .unwrap();
    assert!(store.reader(&snapshot.id).unwrap().files().is_ok());
    let destination = temp.path().join("portable");
    let error = store
        .export_directory(&snapshot.id, &destination)
        .unwrap_err();
    assert!(error.to_string().contains("fresh store"));
    assert!(!destination.exists());
}

#[test]
fn portable_output_and_envelope_reads_enforce_byte_budgets() {
    let temp = tempfile::tempdir().unwrap();
    let (store, snapshot, _) = export(&temp);
    assert!(matches!(
        store.reader(&snapshot.id).unwrap().complete_batch(1),
        Err(Error::BudgetExhausted)
    ));
    let mut manifest = PortableManifest {
        version: 1,
        snapshot_id: snapshot.id,
        files: vec![],
    };
    assert!(matches!(
        write_artifact(
            temp.path(),
            "bounded.json",
            &"x".repeat(1024),
            16,
            &mut manifest
        ),
        Err(Error::BudgetExhausted)
    ));
    assert!(manifest.files.is_empty());
    assert!(
        fs::metadata(temp.path().join("bounded.json"))
            .unwrap()
            .len()
            <= 16
    );
}

#[test]
fn concurrent_exports_never_replace_a_completed_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("source-store")).unwrap();
    let snapshot = store.publish(&complete_fixture(), "main", None).unwrap();
    let destination = temp.path().join("portable");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let id = snapshot.id.clone();
            let destination = destination.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.export_directory(&id, destination)
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let restored = Store::open(temp.path().join("restored")).unwrap();
    assert!(
        restored
            .import_directory(&destination, "main", None, |_, _, _| Ok(()))
            .is_ok()
    );
}

#[test]
fn export_does_not_follow_an_observation_directory_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path().join("source-store")).unwrap();
    let snapshot = store.publish(&complete_fixture(), "main", None).unwrap();
    std::os::unix::fs::symlink(outside.path(), store.root.join("observations")).unwrap();
    let error = store
        .export_directory(&snapshot.id, temp.path().join("portable"))
        .unwrap_err();
    assert!(matches!(error, Error::Io(_)));
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}
