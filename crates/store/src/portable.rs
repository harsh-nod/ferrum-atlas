use crate::*;
use rustix::fs::{Mode, OFlags, RenameFlags, openat};
use serde::{Deserialize, Serialize};

const FACT_BYTES: u64 = 256 * 1024 * 1024;
const METADATA_BYTES: u64 = 2 * 1024 * 1024;
const OBSERVATION_BYTES: u64 = 2 * 1024 * 1024;
const MAX_OBSERVATIONS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortableFile {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PortableManifest {
    pub version: u32,
    pub snapshot_id: SnapshotId,
    pub files: Vec<PortableFile>,
}

pub struct ImportedSnapshot {
    pub snapshot: Snapshot,
    pub observations: Vec<ObservationBundle>,
    pub reader: SnapshotReader,
}

impl Store {
    pub fn export_directory(
        &self,
        id: &SnapshotId,
        destination: impl AsRef<Path>,
    ) -> Result<PortableManifest> {
        let destination = destination.as_ref();
        if fs::symlink_metadata(destination).is_ok() {
            return Err(Error::Invalid("export destination already exists".into()));
        }
        let reader = self.reader(id)?;
        let batch = reader.complete_batch(FACT_BYTES as usize)?;
        verify_snapshot(&reader.snapshot, &batch)?;
        let observations = self.portable_observations(&reader.snapshot)?;
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent)?;
        let name = destination
            .file_name()
            .ok_or_else(|| Error::Invalid("export destination must name a new directory".into()))?;
        let destination = parent.join(name);
        let staging = tempfile::Builder::new()
            .prefix(".atlas-export-")
            .tempdir_in(&parent)?;
        let mut manifest = PortableManifest {
            version: 1,
            snapshot_id: id.clone(),
            files: Vec::new(),
        };
        write_artifact(
            staging.path(),
            "snapshot.json",
            &reader.snapshot,
            METADATA_BYTES,
            &mut manifest,
        )?;
        write_artifact(
            staging.path(),
            "facts.json",
            &batch,
            FACT_BYTES,
            &mut manifest,
        )?;
        for bundle in observations {
            let name = format!(
                "observation-{}.json",
                digest("observation", &bundle)
                    .split_once(':')
                    .expect("digest prefix")
                    .1
            );
            write_artifact(
                staging.path(),
                &name,
                &bundle,
                OBSERVATION_BYTES,
                &mut manifest,
            )?;
        }
        manifest.files.sort_by(|a, b| a.name.cmp(&b.name));
        let mut marker = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(staging.path().join("manifest.json"))?;
        serde_json::to_writer(&mut marker, &manifest)?;
        marker.sync_all()?;
        File::open(staging.path())?.sync_all()?;
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            staging.path(),
            rustix::fs::CWD,
            &destination,
            RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        File::open(parent)?.sync_all()?;
        Ok(manifest)
    }

    pub fn import_directory(
        &self,
        source: impl AsRef<Path>,
        head: &str,
        expected: Option<&SnapshotId>,
        validate_observations: impl FnOnce(
            &Snapshot,
            &BTreeSet<DefinitionId>,
            &[ObservationBundle],
        ) -> Result<()>,
    ) -> Result<ImportedSnapshot> {
        let directory = open_directory(source.as_ref())?;
        let manifest: PortableManifest =
            serde_json::from_slice(&read_artifact(&directory, "manifest.json", 1024 * 1024)?)?;
        if manifest.version != 1 {
            return Err(Error::UnsupportedVersion(manifest.version));
        }
        if manifest.files.len() < 2 || manifest.files.len() > MAX_OBSERVATIONS + 2 {
            return Err(Error::Invalid(
                "portable manifest file count exceeds bounds".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        let mut batch = None;
        let mut snapshot = None;
        let mut observations = Vec::new();
        for file in &manifest.files {
            if !seen.insert(file.name.clone()) || !hex_digest(&file.sha256) {
                return Err(Error::Invalid(
                    "duplicate archive name or invalid digest".into(),
                ));
            }
            let limit = match file.name.as_str() {
                "facts.json" => FACT_BYTES,
                "snapshot.json" => METADATA_BYTES,
                name if observation_name(name).is_some() => OBSERVATION_BYTES,
                _ => return Err(Error::Invalid("unsupported portable artifact name".into())),
            };
            if file.bytes > limit {
                return Err(Error::BudgetExhausted);
            }
            let bytes = read_artifact(&directory, &file.name, limit)?;
            if bytes.len() as u64 != file.bytes || raw_digest(&bytes) != file.sha256 {
                return Err(Error::Invalid("portable artifact checksum mismatch".into()));
            }
            match file.name.as_str() {
                "facts.json" => batch = Some(serde_json::from_slice::<FactBatch>(&bytes)?),
                "snapshot.json" => snapshot = Some(serde_json::from_slice::<Snapshot>(&bytes)?),
                _ => {
                    let bundle: ObservationBundle = serde_json::from_slice(&bytes)?;
                    if observation_name(&file.name)
                        != digest("observation", &bundle)
                            .split_once(':')
                            .map(|(_, hash)| hash)
                    {
                        return Err(Error::Invalid("observation identity mismatch".into()));
                    }
                    observations.push(bundle);
                }
            }
        }
        let batch =
            batch.ok_or_else(|| Error::Invalid("portable archive is missing facts".into()))?;
        let snapshot = snapshot
            .ok_or_else(|| Error::Invalid("portable archive is missing metadata".into()))?;
        validate(&batch)?;
        verify_snapshot(&snapshot, &batch)?;
        if snapshot.id != manifest.snapshot_id {
            return Err(Error::Invalid("portable manifest snapshot mismatch".into()));
        }
        for bundle in &observations {
            verify_observation_identity(bundle, &snapshot)?;
        }
        let definitions = batch
            .definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect();
        validate_observations(&snapshot, &definitions, &observations)?;
        // The returned reader keeps this lease alive across the caller's evidence restore.
        let lease = self.lease(false, &|| false)?;
        let published =
            self.publish_version(&batch, head, expected, None, Some(&snapshot.created_at))?;
        let reader = self.reader_leased(&published.id, &|| false, Some(lease))?;
        Ok(ImportedSnapshot {
            snapshot: published,
            observations,
            reader,
        })
    }

    fn portable_observations(&self, snapshot: &Snapshot) -> Result<Vec<ObservationBundle>> {
        let scope = digest("observations", &snapshot.id);
        let directory = match (|| {
            let root = open_directory(&self.root)?;
            let observations = open_child_directory(&root, "observations")?;
            open_child_directory(
                &observations,
                scope.split_once(':').expect("digest prefix").1,
            )
        })() {
            Ok(directory) => directory,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let lock: File = openat(
            &directory,
            ".import.lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(std::io::Error::from)?
        .into();
        let started = std::time::Instant::now();
        loop {
            match lock.try_lock_shared() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if started.elapsed() < Duration::from_secs(5) =>
                {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(Error::Unavailable("observation import is busy".into()));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let mut bundles = Vec::new();
        for (index, entry) in rustix::fs::Dir::read_from(&directory)
            .map_err(std::io::Error::from)?
            .enumerate()
        {
            if index > 1024 {
                return Err(Error::BudgetExhausted);
            }
            let entry = entry.map_err(std::io::Error::from)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(hash) = name.strip_suffix(".json").filter(|hash| hex_digest(hash)) else {
                continue;
            };
            if bundles.len() >= MAX_OBSERVATIONS {
                return Err(Error::BudgetExhausted);
            }
            let bundle: ObservationBundle =
                serde_json::from_slice(&read_artifact(&directory, &name, OBSERVATION_BYTES)?)?;
            if digest("observation", &bundle)
                .split_once(':')
                .map(|(_, digest)| digest)
                != Some(hash)
            {
                return Err(Error::Unavailable(
                    "observation object checksum mismatch".into(),
                ));
            }
            verify_observation_identity(&bundle, snapshot)?;
            bundles.push(bundle);
        }
        Ok(bundles)
    }
}

fn verify_snapshot(snapshot: &Snapshot, batch: &FactBatch) -> Result<()> {
    let hash = digest("facts", &validate::normalize(batch));
    if snapshot.fact_digest != hash
        || snapshot.id.0 != digest("analysis", &hash)
        || snapshot.source_id != batch.source.id
        || snapshot.repository_id != batch.source.repository_id
        || snapshot.context != batch.context
        || snapshot.revision != batch.source.revision
        || snapshot.producer != batch.producer
        || snapshot.coverage != batch.coverage
        || snapshot.file_count as usize != batch.source.files.len()
        || snapshot.definition_count as usize != batch.definitions.len()
        || snapshot.relation_count as usize != batch.relations.len()
        || snapshot.created_at.parse::<u64>().is_err()
    {
        return Err(Error::Invalid(
            "portable snapshot metadata or fact identity mismatch".into(),
        ));
    }
    Ok(())
}

fn verify_observation_identity(bundle: &ObservationBundle, snapshot: &Snapshot) -> Result<()> {
    if bundle.schema_version != SCHEMA_VERSION
        || bundle.snapshot_id != snapshot.id
        || bundle.artifact.source_id != snapshot.source_id
        || bundle.artifact.context_id != snapshot.context.id
        || !hex_digest(&bundle.artifact.sha256)
    {
        return Err(Error::Invalid(
            "portable observation artifact/snapshot mismatch".into(),
        ));
    }
    Ok(())
}

fn open_directory(path: &Path) -> Result<File> {
    Ok(rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?
    .into())
}

fn open_child_directory(parent: &File, name: &str) -> Result<File> {
    Ok(openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?
    .into())
}

fn read_artifact(directory: &File, name: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let file: File = openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?
    .into();
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::Invalid(
            "portable artifact must be a regular file".into(),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(Error::BudgetExhausted);
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::BudgetExhausted);
    }
    Ok(bytes)
}

fn write_artifact(
    root: &Path,
    name: &str,
    value: &impl Serialize,
    max_bytes: u64,
    manifest: &mut PortableManifest,
) -> Result<()> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join(name))?;
    let mut writer = BoundedWriter {
        file,
        max_bytes,
        written: 0,
        exhausted: false,
    };
    if let Err(error) = serde_json::to_writer(&mut writer, value) {
        return Err(if writer.exhausted {
            Error::BudgetExhausted
        } else {
            error.into()
        });
    }
    writer.file.sync_all()?;
    manifest.files.push(PortableFile {
        name: name.into(),
        sha256: checksum(&root.join(name))?,
        bytes: writer.written,
    });
    Ok(())
}

struct BoundedWriter {
    file: File,
    max_bytes: u64,
    written: u64,
    exhausted: bool,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() as u64 > self.max_bytes.saturating_sub(self.written) {
            self.exhausted = true;
            return Err(std::io::Error::other(
                "portable artifact exceeds byte budget",
            ));
        }
        let count = self.file.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn observation_name(name: &str) -> Option<&str> {
    name.strip_prefix("observation-")
        .and_then(|name| name.strip_suffix(".json"))
        .filter(|hash| hex_digest(hash))
}

fn hex_digest(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn raw_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests;
