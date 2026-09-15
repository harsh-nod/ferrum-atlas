//! Validated immutable snapshot storage and transactional publication.
mod error;
mod reader;
mod validate;

pub use error::{Error, Result};
pub use reader::SnapshotReader;
pub use validate::validate;

use atlas_model::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct IntegrityReport {
    pub valid: bool,
    pub snapshots_checked: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub live_objects: usize,
    pub unreferenced_objects: Vec<String>,
    pub unreferenced_bytes: u64,
    pub limitations: Vec<String>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if rusqlite::version_number() < 3_051_003 {
            return Err(Error::Unavailable(
                "SQLite 3.51.3 or newer is required".into(),
            ));
        }
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path.as_ref())?;
        let root = fs::canonicalize(path)?;
        for name in ["shards", "sources", "staging"] {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(root.join(name))?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("catalog.sqlite"))
        {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let store = Self { root };
        let connection = store.connect()?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != 0 && version != SCHEMA_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS snapshots(id TEXT PRIMARY KEY, repository_id TEXT NOT NULL, metadata TEXT NOT NULL, shard_hash TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS snapshots_repository ON snapshots(repository_id,id);
            CREATE TABLE IF NOT EXISTS heads(name TEXT PRIMARY KEY, snapshot_id TEXT NOT NULL REFERENCES snapshots(id));
            PRAGMA user_version=1;")?;
        Ok(store)
    }

    fn connect(&self) -> Result<Connection> {
        let connection = Connection::open(self.root.join("catalog.sqlite"))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL;",
        )?;
        Ok(connection)
    }

    pub fn head(&self, name: &str) -> Result<Option<SnapshotId>> {
        Ok(self
            .connect()?
            .query_row("SELECT snapshot_id FROM heads WHERE name=?1", [name], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(SnapshotId))
    }

    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        self.snapshots_scoped(None)
    }

    pub fn snapshots_scoped(
        &self,
        repositories: Option<&BTreeSet<RepositoryId>>,
    ) -> Result<Vec<Snapshot>> {
        let connection = self.connect()?;
        let mut output = Vec::new();
        if let Some(repositories) = repositories {
            let mut statement = connection
                .prepare("SELECT metadata FROM snapshots WHERE repository_id=?1 ORDER BY id")?;
            for repository in repositories {
                for row in statement.query_map([&repository.0], |r| r.get::<_, String>(0))? {
                    output.push(serde_json::from_str(&row?)?);
                }
            }
        } else {
            let mut statement = connection.prepare("SELECT metadata FROM snapshots ORDER BY id")?;
            for row in statement.query_map([], |r| r.get::<_, String>(0))? {
                output.push(serde_json::from_str(&row?)?);
            }
        }
        output.sort_by(|a: &Snapshot, b| a.id.cmp(&b.id));
        Ok(output)
    }

    pub fn snapshots_scoped_bounded(
        &self,
        repositories: Option<&BTreeSet<RepositoryId>>,
        max_count: usize,
        max_bytes: usize,
    ) -> Result<Vec<Snapshot>> {
        let connection = self.connect()?;
        let mut output = Vec::new();
        let mut used_bytes = 2usize;
        let mut append = |json: Option<String>| -> Result<()> {
            let json = json.ok_or(Error::BudgetExhausted)?;
            used_bytes = used_bytes.saturating_add(json.len() + 1);
            if output.len() >= max_count || used_bytes > max_bytes {
                return Err(Error::BudgetExhausted);
            }
            output.push(serde_json::from_str::<Snapshot>(&json)?);
            Ok(())
        };
        if let Some(repositories) = repositories {
            let mut statement = connection.prepare("SELECT CASE WHEN length(CAST(metadata AS BLOB))<=?2 THEN metadata ELSE NULL END FROM snapshots WHERE repository_id=?1 ORDER BY id LIMIT ?3")?;
            for repository in repositories {
                for row in statement.query_map(
                    params![
                        repository.0,
                        max_bytes as i64,
                        max_count.saturating_add(1) as i64
                    ],
                    |r| r.get::<_, Option<String>>(0),
                )? {
                    append(row?)?;
                }
            }
        } else {
            let mut statement = connection.prepare("SELECT CASE WHEN length(CAST(metadata AS BLOB))<=?1 THEN metadata ELSE NULL END FROM snapshots ORDER BY id LIMIT ?2")?;
            for row in statement.query_map(
                params![max_bytes as i64, max_count.saturating_add(1) as i64],
                |r| r.get::<_, Option<String>>(0),
            )? {
                append(row?)?;
            }
        }
        output.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(output)
    }

    pub fn snapshot(&self, id: &SnapshotId) -> Result<Snapshot> {
        let json = self
            .connect()?
            .query_row("SELECT metadata FROM snapshots WHERE id=?1", [&id.0], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .ok_or(Error::UnknownSnapshot)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn repository(&self, id: &SnapshotId) -> Result<RepositoryId> {
        self.connect()?
            .query_row(
                "SELECT repository_id FROM snapshots WHERE id=?1",
                [&id.0],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(RepositoryId)
            .ok_or(Error::UnknownSnapshot)
    }

    pub fn publish(
        &self,
        batch: &FactBatch,
        head: &str,
        expected: Option<&SnapshotId>,
    ) -> Result<Snapshot> {
        self.publish_inner(batch, head, expected, None)
    }

    fn publish_inner(
        &self,
        batch: &FactBatch,
        head: &str,
        expected: Option<&SnapshotId>,
        fail_after: Option<u8>,
    ) -> Result<Snapshot> {
        validate(batch)?;
        if head.is_empty() || head.len() > 256 {
            return Err(Error::Invalid(
                "head name must contain 1 to 256 bytes".into(),
            ));
        }
        let batch = validate::normalize(batch);
        let fact_digest = digest("facts", &batch);
        let id = SnapshotId(digest("analysis", &fact_digest));
        let mut snapshot = Snapshot {
            id: id.clone(),
            source_id: batch.source.id.clone(),
            repository_id: batch.source.repository_id.clone(),
            context: batch.context.clone(),
            revision: batch.source.revision.clone(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string(),
            file_count: batch
                .source
                .files
                .len()
                .try_into()
                .map_err(|_| Error::Invalid("too many files".into()))?,
            definition_count: batch
                .definitions
                .len()
                .try_into()
                .map_err(|_| Error::Invalid("too many definitions".into()))?,
            relation_count: batch
                .relations
                .len()
                .try_into()
                .map_err(|_| Error::Invalid("too many relations".into()))?,
            coverage: batch.coverage.clone(),
            producer: batch.producer.clone(),
            fact_digest,
        };
        for source in &batch.source.files {
            self.write_source(source)?;
        }
        injected_failure(fail_after, 1)?;
        let staged = tempfile::NamedTempFile::new_in(self.root.join("staging"))?;
        build_shard(staged.path(), &batch, &snapshot)?;
        File::open(staged.path())?.sync_all()?;
        let shard_hash = checksum(staged.path())?;
        let final_path = self.object_path("shards", &shard_hash)?;
        persist_immutable(staged, &final_path, &shard_hash)?;
        injected_failure(fail_after, 2)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<String> = transaction
            .query_row("SELECT snapshot_id FROM heads WHERE name=?1", [head], |r| {
                r.get(0)
            })
            .optional()?;
        if current.as_deref() != expected.map(|v| v.0.as_str())
            && current.as_deref() != Some(id.0.as_str())
        {
            return Err(Error::Conflict);
        }
        let existing: Option<String> = transaction
            .query_row("SELECT metadata FROM snapshots WHERE id=?1", [&id.0], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(existing) = existing {
            snapshot = serde_json::from_str(&existing)?;
        } else {
            transaction.execute(
                "INSERT INTO snapshots VALUES(?1,?2,?3,?4)",
                params![
                    id.0,
                    snapshot.repository_id.0,
                    serde_json::to_string(&snapshot)?,
                    shard_hash
                ],
            )?;
        }
        injected_failure(fail_after, 3)?;
        transaction.execute("INSERT INTO heads VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET snapshot_id=excluded.snapshot_id", params![head,id.0])?;
        injected_failure(fail_after, 4)?;
        transaction.commit()?;
        Ok(snapshot)
    }

    fn write_source(&self, source: &SourceFile) -> Result<()> {
        let hash = source
            .content_hash
            .strip_prefix("content:")
            .ok_or_else(|| Error::Invalid("invalid source digest".into()))?;
        let path = self.object_path("sources", hash)?;
        if path.exists() {
            let text = fs::read_to_string(&path)?;
            if digest("content", &text) != source.content_hash {
                return Err(Error::Unavailable(
                    "existing source object failed checksum".into(),
                ));
            }
            return Ok(());
        }
        let mut temp = tempfile::NamedTempFile::new_in(self.root.join("staging"))?;
        temp.write_all(source.text.as_bytes())?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(file) => {
                let mut permissions = file.metadata()?.permissions();
                permissions.set_readonly(true);
                file.set_permissions(permissions)?;
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                if fs::read(&path)? != source.text.as_bytes() {
                    return Err(Error::Unavailable("source object collision".into()));
                }
            }
            Err(error) => return Err(Error::Io(error.error)),
        }
        File::open(self.root.join("sources"))?.sync_all()?;
        Ok(())
    }

    pub(crate) fn object_path(&self, category: &str, hash: &str) -> Result<PathBuf> {
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::Unavailable("invalid object digest".into()));
        }
        Ok(self.root.join(category).join(hash))
    }

    pub fn reader(&self, id: &SnapshotId) -> Result<SnapshotReader> {
        self.reader_with_stop(id, &|| false)
    }

    pub fn reader_with_stop(
        &self,
        id: &SnapshotId,
        stopped: &dyn Fn() -> bool,
    ) -> Result<SnapshotReader> {
        let (metadata, hash): (String, String) = self
            .connect()?
            .query_row(
                "SELECT metadata,shard_hash FROM snapshots WHERE id=?1",
                [&id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::UnknownSnapshot)?;
        let snapshot: Snapshot = serde_json::from_str(&metadata)?;
        if snapshot.id != *id || snapshot.id.0 != digest("analysis", &snapshot.fact_digest) {
            return Err(Error::Unavailable("snapshot identity mismatch".into()));
        }
        let path = self.object_path("shards", &hash)?;
        if checksum_with_stop(&path, stopped)? != hash {
            return Err(Error::Unavailable("shard checksum mismatch".into()));
        }
        SnapshotReader::open(self.clone(), snapshot, &path)
    }

    pub fn integrity_check(&self) -> Result<IntegrityReport> {
        let snapshots = self.snapshots()?;
        let mut errors = Vec::new();
        let check: String = self
            .connect()?
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            errors.push("catalog integrity check failed".into());
        }
        for snapshot in &snapshots {
            let result = (|| -> Result<()> {
                let reader = self.reader(&snapshot.id)?;
                reader.verify()?;
                for file in reader.files()? {
                    reader.source(&file.id)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(format!("{}: {error}", snapshot.id));
            }
        }
        Ok(IntegrityReport {
            valid: errors.is_empty(),
            snapshots_checked: snapshots.len(),
            errors,
        })
    }

    pub fn gc_dry_run(&self) -> Result<GcReport> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT id,shard_hash FROM snapshots")?;
        let mut live = BTreeSet::new();
        for row in
            statement.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        {
            let (id, hash) = row?;
            live.insert(format!("shards/{hash}"));
            for source in self.reader(&SnapshotId(id))?.files()? {
                live.insert(format!(
                    "sources/{}",
                    source.content_hash.trim_start_matches("content:")
                ));
            }
        }
        let mut unreferenced_objects = Vec::new();
        let mut unreferenced_bytes = 0;
        for category in ["sources", "shards", "staging"] {
            for entry in fs::read_dir(self.root.join(category))? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let key = format!("{category}/{}", entry.file_name().to_string_lossy());
                if !live.contains(&key) {
                    unreferenced_bytes += entry.metadata()?.len();
                    unreferenced_objects.push(key);
                }
            }
        }
        unreferenced_objects.sort();
        Ok(GcReport { dry_run: true, live_objects: live.len(), unreferenced_objects, unreferenced_bytes,
            limitations: vec!["All published snapshots are retained. Deletion is disabled until active-reader leases and retention policy are implemented.".into()] })
    }
}

fn injected_failure(fail_after: Option<u8>, stage: u8) -> Result<()> {
    if fail_after == Some(stage) {
        Err(Error::Unavailable("injected publication failure".into()))
    } else {
        Ok(())
    }
}

fn checksum(path: &Path) -> Result<String> {
    checksum_with_stop(path, &|| false)
}

fn checksum_with_stop(path: &Path, stopped: &dyn Fn() -> bool) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if stopped() {
            return Err(Error::BudgetExhausted);
        }
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn persist_immutable(temp: tempfile::NamedTempFile, path: &Path, hash: &str) -> Result<()> {
    match temp.persist_noclobber(path) {
        Ok(file) => {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_readonly(true);
            file.set_permissions(permissions)?;
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if checksum(path)? != hash {
                return Err(Error::Unavailable("shard object collision".into()));
            }
        }
        Err(error) => return Err(Error::Io(error.error)),
    }
    File::open(path.parent().expect("object parent"))?.sync_all()?;
    Ok(())
}

fn build_shard(path: &Path, batch: &FactBatch, snapshot: &Snapshot) -> Result<()> {
    let mut connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
        CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE files(id TEXT PRIMARY KEY,path TEXT NOT NULL,content_hash TEXT NOT NULL);
        CREATE TABLE definitions(id TEXT PRIMARY KEY,name_key TEXT NOT NULL,owner_id TEXT,file_id TEXT NOT NULL REFERENCES files(id),start_byte INTEGER NOT NULL,payload TEXT NOT NULL);
        CREATE INDEX definitions_names ON definitions(name_key,id);
        CREATE INDEX definitions_owner ON definitions(owner_id,name_key,id);
        CREATE INDEX definitions_source ON definitions(file_id,start_byte,id);
        CREATE TABLE evidence(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE relations(id TEXT PRIMARY KEY,source_id TEXT NOT NULL REFERENCES definitions(id),target_id TEXT REFERENCES definitions(id),kind TEXT NOT NULL,evidence_id TEXT NOT NULL REFERENCES evidence(id),payload TEXT NOT NULL);
        CREATE INDEX relations_forward ON relations(source_id,kind,target_id,id);
        CREATE INDEX relations_reverse ON relations(target_id,kind,source_id,id);
        CREATE TABLE flows(definition_id TEXT NOT NULL REFERENCES definitions(id),phase TEXT NOT NULL,payload TEXT NOT NULL,PRIMARY KEY(definition_id,phase));
        PRAGMA user_version=1;")?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO metadata VALUES('fact_digest',?1)",
        [&snapshot.fact_digest],
    )?;
    transaction.execute(
        "INSERT INTO metadata VALUES('context_id',?1)",
        [&snapshot.context.id.0],
    )?;
    for file in &batch.source.files {
        transaction.execute(
            "INSERT INTO files VALUES(?1,?2,?3)",
            params![file.id.0, file.path, file.content_hash],
        )?;
    }
    for definition in &batch.definitions {
        transaction.execute(
            "INSERT INTO definitions VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                definition.id.0,
                definition.name.to_lowercase(),
                definition.parent_id.as_ref().map(|id| &id.0),
                definition.file_id.0,
                definition.span.start,
                serde_json::to_string(definition)?
            ],
        )?;
    }
    for evidence in &batch.evidence {
        transaction.execute(
            "INSERT INTO evidence VALUES(?1,?2)",
            params![evidence.id.0, serde_json::to_string(evidence)?],
        )?;
    }
    for relation in &batch.relations {
        let target = match &relation.target {
            Target::Resolved { id } => Some(&id.0),
            Target::Unknown { .. } => None,
        };
        transaction.execute(
            "INSERT INTO relations VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                relation.id.0,
                relation.source.0,
                target,
                relation.kind,
                relation.evidence_id.0,
                serde_json::to_string(relation)?
            ],
        )?;
    }
    for flow in &batch.flows {
        transaction.execute(
            "INSERT INTO flows VALUES(?1,?2,?3)",
            params![
                flow.definition_id.0,
                flow.phase,
                serde_json::to_string(flow)?
            ],
        )?;
    }
    transaction.commit()?;
    let result: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if result != "ok" {
        return Err(Error::Unavailable("new shard integrity failure".into()));
    }
    connection.close().map_err(|(_, error)| Error::Sql(error))?;
    Ok(())
}

#[cfg(test)]
mod tests;
