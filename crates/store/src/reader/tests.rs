use super::*;
use crate::{
    FileIdentity, MAX_SHARD_BYTES, VERIFIED_SHARDS, VerificationCache, verification_cache,
};
use std::cell::Cell;
use std::fs::{self, FileTimes, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{PermissionsExt, symlink};

fn fixture() -> (tempfile::TempDir, Store, Snapshot, String, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let snapshot = store
        .publish(&crate::tests::batch("cache"), "main", None)
        .unwrap();
    let hash: String = store
        .connect()
        .unwrap()
        .query_row(
            "SELECT shard_hash FROM snapshots WHERE id=?1",
            [&snapshot.id.0],
            |row| row.get(0),
        )
        .unwrap();
    let path = store.object_path("shards", &hash).unwrap();
    (temp, store, snapshot, hash, path)
}

fn clear_cache() {
    verification_cache().lock().unwrap().0.clear();
}

fn alter_same_size(path: &Path) {
    let metadata = fs::metadata(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::End(-1)).unwrap();
    file.write_all(&[0x8b]).unwrap();
    file.set_times(FileTimes::new().set_modified(metadata.modified().unwrap()))
        .unwrap();
    file.sync_all().unwrap();
    fs::set_permissions(path, metadata.permissions()).unwrap();
}

#[test]
fn publication_seeds_verified_cache_and_warm_reads_still_honor_cancellation() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, snapshot, hash, _path) = fixture();
    let identity = FileIdentity::of(&store.open_shard(&hash).unwrap()).unwrap();
    assert!(
        verification_cache()
            .lock()
            .unwrap()
            .contains(&hash, &identity)
    );
    let calls = Cell::new(0);
    assert!(
        store
            .verified_shard(&hash, &|| {
                calls.set(calls.get() + 1);
                calls.get() > 2
            })
            .is_ok()
    );
    assert!(matches!(
        store.reader_with_stop(&snapshot.id, &|| true),
        Err(Error::BudgetExhausted)
    ));
    assert!(
        store
            .reader(&snapshot.id)
            .unwrap()
            .definition(&DefinitionId("definition:entry".into()))
            .unwrap()
            .is_some()
    );
}

#[test]
fn cancelled_cold_verification_never_populates_cache() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, _snapshot, hash, _path) = fixture();
    clear_cache();
    let calls = Cell::new(0);
    assert!(matches!(
        store.verified_shard(&hash, &|| {
            calls.set(calls.get() + 1);
            calls.get() > 2
        }),
        Err(Error::BudgetExhausted)
    ));
    assert!(verification_cache().lock().unwrap().0.is_empty());
    store.verified_shard(&hash, &|| false).unwrap();
    assert_eq!(verification_cache().lock().unwrap().0.len(), 1);
}

#[test]
fn same_length_corruption_with_restored_mtime_invalidates_cache() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, snapshot, hash, path) = fixture();
    let original = FileIdentity::of(&store.open_shard(&hash).unwrap()).unwrap();
    alter_same_size(&path);
    let changed = FileIdentity::of(&store.open_shard(&hash).unwrap()).unwrap();
    assert_eq!(original.bytes, changed.bytes);
    assert_eq!(original.modified, changed.modified);
    assert_ne!(original.changed, changed.changed);
    assert!(matches!(
        store.reader(&snapshot.id),
        Err(Error::Unavailable(_))
    ));
    assert!(!store.integrity_check().unwrap().valid);
}

#[test]
fn replacement_inode_must_be_reverified_even_with_matching_bytes_and_mtime() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, snapshot, hash, path) = fixture();
    let old = FileIdentity::of(&store.open_shard(&hash).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let replacement = path.with_extension("replacement");
    fs::write(&replacement, bytes).unwrap();
    fs::rename(replacement, &path).unwrap();
    let new = FileIdentity::of(&store.open_shard(&hash).unwrap()).unwrap();
    assert_ne!(old.inode, new.inode);
    assert!(!verification_cache().lock().unwrap().contains(&hash, &new));
    store.reader(&snapshot.id).unwrap();
    assert!(verification_cache().lock().unwrap().contains(&hash, &new));
    alter_same_size(&path);
    assert!(store.reader(&snapshot.id).is_err());
}

#[test]
fn warmed_readers_reject_leaf_and_directory_symlinks() {
    let _process_guard = crate::tests::process_test_guard();
    let (temp, store, snapshot, _hash, path) = fixture();
    let held = temp.path().join("held-shard");
    fs::rename(&path, &held).unwrap();
    symlink(&held, &path).unwrap();
    assert!(store.reader(&snapshot.id).is_err());
    fs::remove_file(&path).unwrap();
    fs::rename(&held, &path).unwrap();
    let directory = temp.path().join("shards");
    let moved = temp.path().join("held-shards");
    fs::rename(&directory, &moved).unwrap();
    symlink(&moved, &directory).unwrap();
    assert!(store.reader(&snapshot.id).is_err());
}

#[test]
fn a_changed_open_reader_fails_before_serving_cached_sqlite_pages() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, snapshot, _hash, path) = fixture();
    let reader = store.reader(&snapshot.id).unwrap();
    assert!(!reader.search("", None, 50).unwrap().is_empty());
    alter_same_size(&path);
    assert!(reader.search("", None, 50).is_err());
    assert!(reader.source(&FileId("file:test".into())).is_err());
}

#[test]
fn mutation_during_hash_never_trusts_a_changed_identity() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, snapshot, hash, path) = fixture();
    clear_cache();
    let calls = Cell::new(0);
    let result = store.verified_shard(&hash, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 3 {
            alter_same_size(&path);
        }
        false
    });
    assert!(result.is_err());
    assert!(verification_cache().lock().unwrap().0.is_empty());
    assert!(store.reader(&snapshot.id).is_err());
}

#[test]
fn verification_cache_and_untrusted_shard_size_are_bounded() {
    let _process_guard = crate::tests::process_test_guard();
    let (_temp, store, snapshot, hash, path) = fixture();
    let identity = FileIdentity::of(&store.open_shard(&hash).unwrap()).unwrap();
    let mut cache = VerificationCache::default();
    for index in 0..VERIFIED_SHARDS + 20 {
        cache.insert(&format!("{index:064x}"), identity.clone());
    }
    assert_eq!(cache.0.len(), VERIFIED_SHARDS);
    assert!(!cache.contains(&format!("{:064x}", 0), &identity));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(MAX_SHARD_BYTES + 1)
        .unwrap();
    assert!(matches!(
        store.reader(&snapshot.id),
        Err(Error::BudgetExhausted)
    ));
}
