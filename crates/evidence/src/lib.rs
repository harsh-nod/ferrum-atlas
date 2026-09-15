//! Imported observations remain separate from extracted static evidence.
pub mod compiler;
use anyhow::{Context, Result, ensure};
use atlas_model::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::File,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_BUNDLE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EVENTS: usize = 10_000;
const MAX_BUNDLES: usize = 50;
const MAX_DIRECTORY_ENTRIES: usize = 128;
const IMPORT_LOCK_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
use std::fs;

struct ScopedDirectory(File);

impl ScopedDirectory {
    fn open(path: &Path, create: bool) -> Result<Option<Self>> {
        use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
        ensure!(
            path.as_os_str().len() <= 4096 && path.components().count() <= 256,
            "evidence directory path exceeds budget"
        );
        let flags = OFlags::RDONLY
            | OFlags::DIRECTORY
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::CLOEXEC;
        let mut directory = File::from(open(
            if path.is_absolute() { "/" } else { "." },
            flags,
            Mode::empty(),
        )?);
        for component in path.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => name,
                Component::ParentDir => OsStr::new(".."),
                Component::Prefix(_) => anyhow::bail!("unsupported evidence directory prefix"),
            };
            let next = match openat(&directory, name, flags, Mode::empty()) {
                Ok(next) => next,
                Err(rustix::io::Errno::NOENT) if !create => return Ok(None),
                Err(rustix::io::Errno::NOENT) => {
                    match mkdirat(&directory, name, Mode::RWXU) {
                        Ok(()) => directory.sync_all()?,
                        Err(rustix::io::Errno::EXIST) => (),
                        Err(error) => return Err(error.into()),
                    }
                    openat(&directory, name, flags, Mode::empty())?
                }
                Err(error) => return Err(error.into()),
            };
            directory = File::from(next);
        }
        Ok(Some(Self(directory)))
    }

    fn child(name: &Path) -> Result<()> {
        let mut components = name.components();
        ensure!(
            matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
            "invalid evidence object name"
        );
        Ok(())
    }

    fn exists(&self, name: &Path) -> Result<bool> {
        Self::child(name)?;
        match rustix::fs::statat(&self.0, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn read(&self, name: &Path, max_bytes: u64) -> Result<Vec<u8>> {
        self.read_with_stop(name, max_bytes, &|| false)
    }

    fn read_with_stop(
        &self,
        name: &Path,
        max_bytes: u64,
        stopped: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>> {
        use rustix::fs::{Mode, OFlags, openat};
        evidence_checkpoint(stopped)?;
        Self::child(name)?;
        let mut file = File::from(openat(
            &self.0,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        evidence_checkpoint(stopped)?;
        let metadata = file.metadata()?;
        evidence_checkpoint(stopped)?;
        ensure!(metadata.is_file(), "evidence object must be a regular file");
        ensure!(
            metadata.len() <= max_bytes,
            "evidence object exceeds byte budget"
        );
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            evidence_checkpoint(stopped)?;
            let count = match file.read(&mut buffer) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            evidence_checkpoint(stopped)?;
            ensure!(
                (bytes.len() as u64).saturating_add(count as u64) <= max_bytes,
                "evidence object grew beyond byte budget"
            );
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        Ok(bytes)
    }

    fn json_names(&self, max_objects: usize) -> Result<Vec<PathBuf>> {
        self.json_names_with_stop(max_objects, &|| false)
    }

    fn json_names_with_stop(
        &self,
        max_objects: usize,
        stopped: &dyn Fn() -> bool,
    ) -> Result<Vec<PathBuf>> {
        evidence_checkpoint(stopped)?;
        let mut names = Vec::new();
        let mut count = 0;
        let mut directory = rustix::fs::Dir::read_from(&self.0)?;
        loop {
            evidence_checkpoint(stopped)?;
            let entry = directory.next();
            evidence_checkpoint(stopped)?;
            let Some(entry) = entry else {
                break;
            };
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            count += 1;
            ensure!(
                count <= MAX_DIRECTORY_ENTRIES,
                "evidence directory entry budget exceeded"
            );
            let name = Path::new(OsStr::from_bytes(name));
            if name
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                names.push(name.to_owned());
                ensure!(
                    names.len() <= max_objects,
                    "snapshot evidence object limit exceeded"
                );
            }
        }
        names.sort();
        evidence_checkpoint(stopped)?;
        Ok(names)
    }

    fn import_lock(&self) -> Result<File> {
        self.import_lock_with_stop(&|| false)
    }

    fn import_lock_with_stop(&self, stopped: &dyn Fn() -> bool) -> Result<File> {
        use rustix::fs::{FlockOperation, Mode, OFlags, fchmod, flock, openat};
        evidence_checkpoint(stopped)?;
        let lock = File::from(openat(
            &self.0,
            ".import.lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        evidence_checkpoint(stopped)?;
        let metadata = lock.metadata()?;
        evidence_checkpoint(stopped)?;
        ensure!(
            metadata.is_file() && metadata.len() == 0 && metadata.nlink() == 1,
            "import lock must be a regular, empty, singly-linked private file"
        );
        ensure!(
            metadata.uid() == self.0.metadata()?.uid(),
            "import lock owner differs from its scope"
        );
        evidence_checkpoint(stopped)?;
        fchmod(&lock, Mode::RUSR | Mode::WUSR)?;
        evidence_checkpoint(stopped)?;
        let deadline = Instant::now() + IMPORT_LOCK_TIMEOUT;
        loop {
            evidence_checkpoint(stopped)?;
            match flock(&lock, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => {
                    evidence_checkpoint(stopped)?;
                    return Ok(lock);
                }
                Err(rustix::io::Errno::WOULDBLOCK) | Err(rustix::io::Errno::INTR) => {
                    evidence_checkpoint(stopped)?;
                    ensure!(
                        Instant::now() < deadline,
                        "evidence import lock deadline exceeded"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn publish(&self, name: &Path, bytes: &[u8]) -> Result<bool> {
        self.publish_with_stop(name, bytes, &|| false)
    }

    fn publish_with_stop(
        &self,
        name: &Path,
        bytes: &[u8],
        stopped: &dyn Fn() -> bool,
    ) -> Result<bool> {
        evidence_checkpoint(stopped)?;
        Self::child(name)?;
        // This kernel-owned path names the held descriptor, never the mutable user path.
        let pinned = PathBuf::from(format!("/proc/self/fd/{}", self.0.as_raw_fd()));
        let mut temporary = tempfile::Builder::new()
            .prefix(".import-")
            .tempfile_in(pinned)?;
        evidence_checkpoint(stopped)?;
        for chunk in bytes.chunks(64 * 1024) {
            evidence_checkpoint(stopped)?;
            temporary.write_all(chunk)?;
            evidence_checkpoint(stopped)?;
        }
        temporary.as_file().sync_all()?;
        evidence_checkpoint(stopped)?;
        match rustix::fs::renameat_with(
            &self.0,
            temporary
                .path()
                .file_name()
                .context("temporary object has no name")?,
            &self.0,
            name,
            rustix::fs::RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                // Publication already happened. Finish durability even if cancellation
                // raced rename; callers may retry, but must never roll this object back.
                self.0.sync_all()?;
                evidence_checkpoint(stopped)?;
                Ok(true)
            }
            Err(rustix::io::Errno::EXIST) => {
                evidence_checkpoint(stopped)?;
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn evidence_checkpoint(stopped: &dyn Fn() -> bool) -> Result<()> {
    ensure!(!stopped(), "evidence operation cancelled");
    Ok(())
}

fn scope(root: &Path, snapshot: &SnapshotId) -> PathBuf {
    root.join(digest("observations", snapshot).split_once(':').unwrap().1)
}

pub fn artifact_sha256(path: &Path) -> Result<String> {
    let fd = rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    let mut input = File::from(fd);
    ensure!(
        input.metadata()?.is_file(),
        "artifact must be a regular file"
    );
    ensure!(
        input.metadata()?.len() <= 2 * 1024 * 1024 * 1024,
        "artifact exceeds 2 GiB import budget"
    );
    let mut hash = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = input.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        ensure!(
            total <= 2 * 1024 * 1024 * 1024,
            "artifact grew beyond its import budget"
        );
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn validate(
    bundle: &ObservationBundle,
    snapshot: &Snapshot,
    definitions: &BTreeSet<DefinitionId>,
    artifact_hash: &str,
) -> Result<()> {
    ensure!(
        bundle.schema_version == SCHEMA_VERSION,
        "unsupported observation schema"
    );
    ensure!(
        bundle.snapshot_id == snapshot.id,
        "observation snapshot mismatch"
    );
    ensure!(
        bundle.artifact.source_id == snapshot.source_id
            && bundle.artifact.context_id == snapshot.context.id,
        "artifact source/context mismatch"
    );
    ensure!(
        artifact_hash.len() == 64 && artifact_hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid artifact digest"
    );
    ensure!(
        bundle.artifact.sha256 == artifact_hash,
        "executable artifact digest mismatch"
    );
    ensure!(
        !bundle.artifact.producer.trim().is_empty(),
        "artifact producer is required"
    );
    ensure!(
        bundle.artifact.producer.len() <= 512,
        "artifact producer exceeds text budget"
    );
    ensure!(
        bundle.limitations.len() <= 32 && bundle.limitations.iter().all(|s| s.len() <= 1024),
        "limitation text exceeds budget"
    );
    ensure!(
        bundle.tests.len() <= 1000 && bundle.streams.len() <= 64,
        "observation record budget exceeded"
    );
    ensure!(
        bundle.streams.iter().map(|s| s.events.len()).sum::<usize>() <= MAX_EVENTS,
        "event budget exceeded"
    );
    for test in &bundle.tests {
        ensure!(!test.name.is_empty(), "test identity is required");
        ensure!(
            test.name.len() <= 256
                && test.reason.as_ref().is_none_or(|s| s.len() <= 1024)
                && test.definition_ids.len() <= 100,
            "test record exceeds budget"
        );
        for number in [&test.elapsed_ns, &test.timeout_ns].into_iter().flatten() {
            decimal(number)?;
        }
        ensure!(
            test.outcome != TestOutcome::Timeout || test.timeout_ns.is_some(),
            "timeout observations require the timeout cap"
        );
        ensure!(
            test.definition_ids
                .iter()
                .all(|id| definitions.contains(id)),
            "test mapping references a different snapshot or missing definition"
        );
    }
    let mut streams = BTreeSet::new();
    for stream in &bundle.streams {
        ensure!(
            [
                &stream.id,
                &stream.process_or_device,
                &stream.thread_or_hart,
                &stream.clock_domain,
                &stream.timestamp_unit
            ]
            .iter()
            .all(|s| s.len() <= 128),
            "stream identity exceeds text budget"
        );
        ensure!(
            !stream.id.is_empty() && streams.insert(&stream.id),
            "trace stream identity missing or duplicated"
        );
        ensure!(
            !stream.clock_domain.is_empty() && !stream.timestamp_unit.is_empty(),
            "clock domain and timestamp unit are required"
        );
        let mut previous = None;
        for event in &stream.events {
            let sequence = decimal(&event.sequence)?;
            decimal(&event.timestamp)?;
            ensure!(
                previous.is_none_or(|p| sequence > p),
                "trace sequence must increase within each stream"
            );
            previous = Some(sequence);
            if let Some(loss) = &event.loss_count {
                decimal(loss)?;
            }
            ensure!(
                event
                    .definition_id
                    .as_ref()
                    .is_none_or(|id| definitions.contains(id)),
                "trace mapping references a different snapshot or missing definition"
            );
            ensure!(!event.kind.is_empty(), "trace event kind is required");
            ensure!(
                event.kind.len() <= 128
                    && event.correlation_id.as_ref().is_none_or(|s| s.len() <= 128),
                "trace event exceeds text budget"
            );
        }
    }
    ensure!(
        serde_json::to_vec(bundle)?.len() as u64 <= MAX_BUNDLE_BYTES,
        "observation bundle exceeds 2 MiB"
    );
    Ok(())
}

fn decimal(value: &str) -> Result<u64> {
    ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "timestamps and sequence numbers must be unsigned decimal strings"
    );
    value.parse().context("decimal value exceeds u64")
}

pub fn import(
    root: &Path,
    bundle: &ObservationBundle,
    snapshot: &Snapshot,
    definitions: &BTreeSet<DefinitionId>,
    artifact: &Path,
) -> Result<ObservationSummary> {
    validate(bundle, snapshot, definitions, &artifact_sha256(artifact)?)?;
    persist_bundle(root, bundle, snapshot)
}

/// Restore an already checksummed portable record. This validates identity
/// consistency, not execution or possession of the original executable.
pub fn restore(
    root: &Path,
    bundle: &ObservationBundle,
    snapshot: &Snapshot,
    definitions: &BTreeSet<DefinitionId>,
) -> Result<ObservationSummary> {
    validate(bundle, snapshot, definitions, &bundle.artifact.sha256)?;
    persist_bundle(root, bundle, snapshot)
}

fn persist_bundle(
    root: &Path,
    bundle: &ObservationBundle,
    snapshot: &Snapshot,
) -> Result<ObservationSummary> {
    let folder = ScopedDirectory::open(&scope(root, &snapshot.id), true)?
        .context("evidence scope unavailable")?;
    let _lock = folder.import_lock()?;
    let id = digest("observation", bundle);
    let destination = PathBuf::from(format!("{}.json", id.split_once(':').unwrap().1));
    let names = folder.json_names(MAX_BUNDLES)?;
    if folder.exists(&destination)? {
        ensure!(
            read_bundle(&folder, &destination)?.0 == id,
            "stored observation is corrupt"
        );
        return Ok(summary(&id, bundle));
    }
    ensure!(
        names.len() < MAX_BUNDLES,
        "snapshot observation limit reached"
    );
    if !folder.publish(&destination, &serde_json::to_vec(bundle)?)? {
        ensure!(
            read_bundle(&folder, &destination)?.0 == id,
            "stored observation is corrupt"
        );
    }
    Ok(summary(&id, bundle))
}

/// Load one checksummed, bounded observation object after caller authorization.
pub fn load(root: &Path, snapshot: &SnapshotId, id: &str) -> Result<ObservationBundle> {
    let suffix = id
        .strip_prefix("observation:")
        .context("invalid observation ID")?;
    ensure!(
        suffix.len() == 64 && suffix.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid observation ID"
    );
    let folder = ScopedDirectory::open(&scope(root, snapshot), false)?
        .context("observation scope unavailable")?;
    let (_, bundle) = read_bundle(&folder, Path::new(&format!("{suffix}.json")))?;
    ensure!(
        &bundle.snapshot_id == snapshot,
        "observation snapshot mismatch"
    );
    Ok(bundle)
}

fn read_bundle(folder: &ScopedDirectory, name: &Path) -> Result<(String, ObservationBundle)> {
    let bundle: ObservationBundle = serde_json::from_slice(&folder.read(name, MAX_BUNDLE_BYTES)?)?;
    ensure!(
        bundle.schema_version == SCHEMA_VERSION,
        "unsupported observation schema"
    );
    let id = digest("observation", &bundle);
    ensure!(
        name.file_stem().and_then(|n| n.to_str()) == id.split_once(':').map(|(_, v)| v),
        "observation checksum mismatch"
    );
    Ok((id, bundle))
}

pub fn list(root: &Path, snapshot: &SnapshotId) -> Result<Vec<ObservationSummary>> {
    let Some(directory) = ScopedDirectory::open(&scope(root, snapshot), false)? else {
        return Ok(vec![]);
    };
    directory
        .json_names(MAX_BUNDLES)?
        .into_iter()
        .map(|path| {
            let (id, bundle) = read_bundle(&directory, &path)?;
            ensure!(
                &bundle.snapshot_id == snapshot,
                "observation snapshot mismatch"
            );
            Ok(summary(&id, &bundle))
        })
        .collect()
}

pub fn window(
    root: &Path,
    snapshot: &SnapshotId,
    id: &str,
    offset: u32,
    limit: u32,
) -> Result<ObservationWindow> {
    ensure!(
        (1..=200).contains(&limit),
        "observation window limit must be 1-200"
    );
    let bundle = load(root, snapshot, id)?;
    let total = bundle.tests.len() + bundle.streams.iter().map(|s| s.events.len()).sum::<usize>();
    let start = offset as usize;
    ensure!(start <= total, "observation offset out of range");
    let end = start.saturating_add(limit as usize).min(total);
    let tests = bundle
        .tests
        .iter()
        .enumerate()
        .filter(|(i, _)| *i >= start && *i < end)
        .map(|(_, t)| t.clone())
        .collect();
    let mut index = bundle.tests.len();
    let streams = bundle
        .streams
        .iter()
        .filter_map(|s| {
            let mut result = s.clone();
            result.events = s
                .events
                .iter()
                .enumerate()
                .filter(|(i, _)| index + *i >= start && index + *i < end)
                .map(|(_, e)| e.clone())
                .collect();
            index += s.events.len();
            (!result.events.is_empty()).then_some(result)
        })
        .collect();
    Ok(ObservationWindow {
        summary: summary(id, &bundle),
        tests,
        streams,
        offset,
        next_offset: (end < total).then_some(end as u32),
        truncated: end < total,
    })
}

fn summary(id: &str, bundle: &ObservationBundle) -> ObservationSummary {
    let mut limitations = bundle.limitations.clone();
    limitations.push(
        "Imported run observations are not exhaustive behavior or static call-resolution evidence."
            .into(),
    );
    limitations.push("Cross-stream clock order and causal links are not inferred.".into());
    ObservationSummary {
        id: id.into(),
        snapshot_id: bundle.snapshot_id.clone(),
        artifact: bundle.artifact.clone(),
        test_count: bundle.tests.len() as u32,
        event_count: bundle.streams.iter().map(|s| s.events.len() as u32).sum(),
        clock_domains: bundle
            .streams
            .iter()
            .map(|s| s.clock_domain.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        limitations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Snapshot, ObservationBundle, BTreeSet<DefinitionId>) {
        let source = SourceId("source:fixture".into());
        let context = BuildContext {
            id: ContextId("context:fixture".into()),
            name: "fixture".into(),
            target: "unknown".into(),
            features: vec![],
            default_features: false,
            cfg: Default::default(),
            crates: vec![],
            manifest_digest: "fixture".into(),
            trust: "read_only".into(),
            coverage: Coverage::complete(),
        };
        let snapshot = Snapshot {
            id: SnapshotId("snapshot:fixture".into()),
            source_id: source.clone(),
            repository_id: RepositoryId("repo:fixture".into()),
            context: context.clone(),
            revision: "fixture".into(),
            created_at: "0".into(),
            file_count: 1,
            definition_count: 1,
            relation_count: 0,
            coverage: Coverage::complete(),
            producer: "fixture".into(),
            fact_digest: "fixture".into(),
        };
        let definition = DefinitionId("definition:fixture".into());
        let bundle = ObservationBundle {
            schema_version: 1,
            snapshot_id: snapshot.id.clone(),
            artifact: ArtifactIdentity {
                sha256: format!("{:x}", Sha256::digest(b"artifact")),
                source_id: source,
                context_id: context.id,
                producer: "fixture-test-runner".into(),
            },
            tests: vec![TestObservation {
                name: "dispatch".into(),
                outcome: TestOutcome::Pass,
                elapsed_ns: Some("9007199254740993".into()),
                timeout_ns: None,
                reason: None,
                definition_ids: vec![definition.clone()],
            }],
            streams: vec![TraceStream {
                id: "cpu0".into(),
                process_or_device: "test".into(),
                thread_or_hart: "0".into(),
                clock_domain: "cpu-clock".into(),
                timestamp_unit: "ns".into(),
                events: vec![TraceEvent {
                    sequence: "1".into(),
                    timestamp: "9007199254740993".into(),
                    kind: "enter".into(),
                    definition_id: Some(definition.clone()),
                    correlation_id: None,
                    loss_count: Some("0".into()),
                }],
            }],
            limitations: vec![],
        };
        (snapshot, bundle, BTreeSet::from([definition]))
    }
    #[test]
    fn rejects_wrong_artifact_context_and_source() {
        let (snapshot, mut bundle, definitions) = fixture();
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_ok());
        assert!(validate(&bundle, &snapshot, &definitions, &"0".repeat(64)).is_err());
        bundle.artifact.context_id = ContextId("other".into());
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_err());
        bundle.artifact.context_id = snapshot.context.id.clone();
        bundle.artifact.source_id = SourceId("other".into());
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_err());
    }
    #[test]
    fn preserves_decimal_precision_and_independent_clocks() {
        let (snapshot, mut bundle, definitions) = fixture();
        let mut other = bundle.streams[0].clone();
        other.id = "device0".into();
        other.clock_domain = "device-cycles".into();
        other.timestamp_unit = "cycles".into();
        other.events[0].timestamp = "2".into();
        bundle.streams.push(other);
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_ok());
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(json.contains("\"9007199254740993\""));
        assert_eq!(summary("id", &bundle).clock_domains.len(), 2);
        let duplicate = bundle.streams[0].events[0].clone();
        bundle.streams[0].events.push(duplicate);
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_err());
    }
    #[test]
    fn outcomes_and_missing_mapping_are_not_coerced() {
        let (snapshot, mut bundle, definitions) = fixture();
        bundle.tests[0].outcome = TestOutcome::Timeout;
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_err());
        bundle.tests[0].timeout_ns = Some("1000".into());
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_ok());
        bundle.streams[0].events[0].definition_id = Some(DefinitionId("missing".into()));
        assert!(validate(&bundle, &snapshot, &definitions, &bundle.artifact.sha256).is_err());
    }
    #[test]
    fn persistence_is_deduplicated_bounded_and_checksum_verified() {
        let temp = tempfile::tempdir().unwrap();
        let artifact = temp.path().join("program");
        fs::write(&artifact, b"artifact").unwrap();
        let (snapshot, bundle, definitions) = fixture();
        let root = temp.path().join("observations");
        let first = import(&root, &bundle, &snapshot, &definitions, &artifact).unwrap();
        let second = import(&root, &bundle, &snapshot, &definitions, &artifact).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(list(&root, &snapshot.id).unwrap().len(), 1);
        let page = window(&root, &snapshot.id, &first.id, 0, 1).unwrap();
        assert_eq!(page.tests.len(), 1);
        assert!(page.truncated);
        assert_eq!(page.next_offset, Some(1));
        let next = window(&root, &snapshot.id, &first.id, 1, 1).unwrap();
        assert_eq!(next.streams[0].events[0].sequence, "1");
        assert!(!next.truncated);
        assert!(window(&root, &snapshot.id, "../invalid", 0, 1).is_err());
        let path = scope(&root, &snapshot.id)
            .join(format!("{}.json", first.id.split_once(':').unwrap().1));
        fs::write(path, b"{}").unwrap();
        assert!(list(&root, &snapshot.id).is_err());
    }

    #[test]
    fn concurrent_imports_cannot_overfill_the_snapshot_limit() {
        let temp = tempfile::tempdir().unwrap();
        let artifact = temp.path().join("program");
        fs::write(&artifact, b"artifact").unwrap();
        let root = temp.path().join("observations");
        let (snapshot, bundle, definitions) = fixture();
        for index in 0..49 {
            let mut candidate = bundle.clone();
            candidate.tests[0].name = format!("test-{index}");
            import(&root, &candidate, &snapshot, &definitions, &artifact).unwrap();
        }
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let root = root.clone();
                let artifact = artifact.clone();
                let snapshot = snapshot.clone();
                let definitions = definitions.clone();
                let barrier = barrier.clone();
                let mut candidate = bundle.clone();
                candidate.tests[0].name = format!("concurrent-{index}");
                std::thread::spawn(move || {
                    barrier.wait();
                    import(&root, &candidate, &snapshot, &definitions, &artifact).is_ok()
                })
            })
            .collect();
        barrier.wait();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|ok| *ok)
                .count(),
            1
        );
        assert_eq!(list(&root, &snapshot.id).unwrap().len(), 50);
    }

    mod io {
        use super::*;
        include!("io_tests.rs");
    }
}
