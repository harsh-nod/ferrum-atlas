use crate::*;
use rustix::fs::{AtFlags, Mode, OFlags, openat, statat, unlinkat};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotPin {
    pub name: String,
    pub snapshot_id: SnapshotId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub keep_recent: usize,
    pub grace_period: Duration,
    pub additional_roots: BTreeSet<SnapshotId>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_recent: 3,
            grace_period: Duration::from_secs(24 * 60 * 60),
            additional_roots: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetainedSnapshot {
    pub id: SnapshotId,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcObject {
    pub category: String,
    pub hash: String,
    pub bytes: u64,
    pub device: u64,
    pub inode: u64,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcPlan {
    pub fingerprint: String,
    pub policy: RetentionPolicy,
    pub retained_snapshots: Vec<RetainedSnapshot>,
    pub remove_snapshots: Vec<SnapshotId>,
    pub objects: Vec<GcObject>,
    pub category_bytes: BTreeMap<String, u64>,
    pub live_objects: usize,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub live_objects: usize,
    pub unreferenced_objects: Vec<String>,
    pub unreferenced_bytes: u64,
    pub removed_snapshots: Vec<SnapshotId>,
    pub category_bytes: BTreeMap<String, u64>,
    pub plan_fingerprint: String,
    pub limitations: Vec<String>,
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

pub(crate) fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshot_pins(name TEXT PRIMARY KEY,snapshot_id TEXT NOT NULL REFERENCES snapshots(id));
         CREATE TABLE IF NOT EXISTS snapshot_generations(sequence INTEGER PRIMARY KEY AUTOINCREMENT,snapshot_id TEXT NOT NULL UNIQUE REFERENCES snapshots(id) ON DELETE CASCADE,published_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS owned_objects(category TEXT NOT NULL,hash TEXT NOT NULL,device INTEGER NOT NULL,inode INTEGER NOT NULL,bytes INTEGER NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(category,hash));
         CREATE TABLE IF NOT EXISTS retention_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS gc_runs(fingerprint TEXT PRIMARY KEY,completed INTEGER NOT NULL,report TEXT NOT NULL);",
    )?;
    Ok(())
}

impl Store {
    fn root_directory(&self) -> Result<File> {
        let fd = rustix::fs::open(
            &self.root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        Ok(fd.into())
    }

    fn category_directory(&self, category: &str) -> Result<File> {
        if !matches!(category, "sources" | "shards") {
            return Err(Error::Unavailable("invalid owned object category".into()));
        }
        let fd = openat(
            self.root_directory()?,
            category,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?;
        Ok(fd.into())
    }

    pub(crate) fn lease(&self, exclusive: bool, stopped: &dyn Fn() -> bool) -> Result<File> {
        let fd = openat(
            self.root_directory()?,
            ".store.lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(std::io::Error::from)?;
        let file = File::from(fd);
        if !file.metadata()?.is_file() {
            return Err(Error::Unavailable(
                "store lock is not a regular file".into(),
            ));
        }
        let started = std::time::Instant::now();
        loop {
            if stopped() {
                return Err(Error::BudgetExhausted);
            }
            let result = if exclusive {
                file.try_lock()
            } else {
                file.try_lock_shared()
            };
            match result {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock)
                    if !exclusive && started.elapsed() < Duration::from_secs(5) =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(Error::Unavailable(
                        "store busy: active reader, publisher, or garbage collector holds a lease"
                            .into(),
                    ));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    pub(crate) fn register_object(&self, category: &str, hash: &str) -> Result<()> {
        self.object_path(category, hash)?;
        let directory = self.category_directory(category)?;
        let object: File = openat(
            &directory,
            hash,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(std::io::Error::from)?
        .into();
        let metadata = object.metadata()?;
        if !metadata.is_file() {
            return Err(Error::Unavailable(
                "owned object is not a regular file".into(),
            ));
        }
        let connection = self.connect()?;
        connection.execute(
            "INSERT OR IGNORE INTO owned_objects VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                category,
                hash,
                metadata.dev() as i64,
                metadata.ino() as i64,
                metadata.len() as i64,
                now()
            ],
        )?;
        let identity: (i64, i64, i64) = connection.query_row(
            "SELECT device,inode,bytes FROM owned_objects WHERE category=?1 AND hash=?2",
            params![category, hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if identity
            != (
                metadata.dev() as i64,
                metadata.ino() as i64,
                metadata.len() as i64,
            )
        {
            return Err(Error::Unavailable("owned object identity changed".into()));
        }
        Ok(())
    }

    pub(crate) fn initialize_object_ownership(&self) -> Result<()> {
        let connection = self.connect()?;
        let initialized = connection
            .query_row(
                "SELECT value FROM retention_metadata WHERE key='initialized'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if initialized.is_some() {
            return Ok(());
        }
        let mut snapshots = self.snapshots()?;
        snapshots.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        for snapshot in snapshots {
            let hash: String = connection.query_row(
                "SELECT shard_hash FROM snapshots WHERE id=?1",
                [&snapshot.id.0],
                |row| row.get(0),
            )?;
            let reader = self.reader_leased(&snapshot.id, &|| false, None)?;
            reader.verify()?;
            self.register_object("shards", &hash)?;
            for source in reader.files()? {
                self.register_object(
                    "sources",
                    source
                        .content_hash
                        .strip_prefix("content:")
                        .ok_or_else(|| Error::Unavailable("invalid source digest".into()))?,
                )?;
            }
            connection.execute("INSERT OR IGNORE INTO snapshot_generations(snapshot_id,published_at) VALUES(?1,?2)",params![snapshot.id.0,snapshot.created_at.parse::<i64>().unwrap_or_else(|_| now())])?;
        }
        connection.execute(
            "INSERT OR IGNORE INTO retention_metadata VALUES('initialized','1')",
            [],
        )?;
        Ok(())
    }

    pub fn pin(&self, id: &SnapshotId, name: &str) -> Result<()> {
        if name.is_empty() || name.len() > 256 {
            return Err(Error::Invalid(
                "pin name must contain 1 to 256 bytes".into(),
            ));
        }
        let _lease = self.lease(false, &|| false)?;
        self.snapshot(id)?;
        self.connect()?.execute("INSERT INTO snapshot_pins VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET snapshot_id=excluded.snapshot_id", params![name,id.0])?;
        Ok(())
    }

    pub fn unpin(&self, name: &str) -> Result<bool> {
        let _lease = self.lease(false, &|| false)?;
        Ok(self
            .connect()?
            .execute("DELETE FROM snapshot_pins WHERE name=?1", [name])?
            > 0)
    }

    pub fn pins(&self) -> Result<Vec<SnapshotPin>> {
        let connection = self.connect()?;
        let mut statement =
            connection.prepare("SELECT name,snapshot_id FROM snapshot_pins ORDER BY name")?;
        Ok(statement
            .query_map([], |row| {
                Ok(SnapshotPin {
                    name: row.get(0)?,
                    snapshot_id: SnapshotId(row.get(1)?),
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn gc_plan(&self, policy: &RetentionPolicy) -> Result<GcPlan> {
        let _lease = self.lease(true, &|| false)?;
        self.gc_plan_locked(policy)
    }

    fn gc_plan_locked(&self, policy: &RetentionPolicy) -> Result<GcPlan> {
        let connection = self.connect()?;
        let (snapshots, metadata_bytes): (i64, i64) = connection.query_row(
            "SELECT count(*),coalesce(sum(length(CAST(metadata AS BLOB))),0) FROM snapshots",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let objects: i64 =
            connection.query_row("SELECT count(*) FROM owned_objects", [], |row| row.get(0))?;
        if snapshots > 100_000 || metadata_bytes > 128 * 1024 * 1024 || objects > 1_000_000 {
            return Err(Error::BudgetExhausted);
        }
        let mut roots = BTreeMap::<SnapshotId, Vec<String>>::new();
        let mut add = |id: SnapshotId, reason: String| roots.entry(id).or_default().push(reason);
        for table in ["heads", "snapshot_pins"] {
            let mut statement =
                connection.prepare(&format!("SELECT name,snapshot_id FROM {table}"))?;
            for row in statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, SnapshotId(row.get(1)?)))
            })? {
                let (name, id) = row?;
                add(id, format!("{table}:{name}"));
            }
        }
        for id in &policy.additional_roots {
            self.snapshot(id)?;
            add(id.clone(), "additional_root".into());
        }
        let cutoff =
            now().saturating_sub(policy.grace_period.as_secs().min(i64::MAX as u64) as i64);
        let mut snapshots = self.snapshots()?;
        let mut generations = BTreeMap::new();
        let mut statement = connection
            .prepare("SELECT snapshot_id,sequence,published_at FROM snapshot_generations")?;
        for row in statement.query_map([], |row| {
            Ok((
                SnapshotId(row.get(0)?),
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })? {
            let (id, sequence, published_at) = row?;
            generations.insert(id, (sequence, published_at));
        }
        snapshots.sort_by_key(|snapshot| {
            std::cmp::Reverse(
                generations
                    .get(&snapshot.id)
                    .copied()
                    .unwrap_or((i64::MAX, now())),
            )
        });
        let mut recent = BTreeMap::<(RepositoryId, ContextId), usize>::new();
        for snapshot in &snapshots {
            let count = recent
                .entry((snapshot.repository_id.clone(), snapshot.context.id.clone()))
                .or_default();
            if *count < policy.keep_recent {
                add(snapshot.id.clone(), "recent_generation".into());
            }
            *count += 1;
            if generations
                .get(&snapshot.id)
                .map(|(_, time)| *time > cutoff)
                .unwrap_or(true)
            {
                add(snapshot.id.clone(), "grace_period".into());
            }
            let observation_scope = digest("observations", &snapshot.id);
            let observation_metadata = fs::symlink_metadata(
                self.root
                    .join("observations")
                    .join(observation_scope.split_once(':').expect("digest prefix").1),
            );
            match observation_metadata {
                Ok(_) => add(snapshot.id.clone(), "observations".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut live = BTreeSet::new();
        let mut remove_snapshots = Vec::new();
        for snapshot in &snapshots {
            // Corrupt references abort planning instead of declaring their objects dead.
            let reader = self.reader_leased(&snapshot.id, &|| false, None)?;
            let files = reader.files()?;
            if roots.contains_key(&snapshot.id) {
                let hash: String = connection.query_row(
                    "SELECT shard_hash FROM snapshots WHERE id=?1",
                    [&snapshot.id.0],
                    |row| row.get(0),
                )?;
                live.insert(("shards".to_string(), hash));
                for source in files {
                    let hash = source
                        .content_hash
                        .strip_prefix("content:")
                        .ok_or_else(|| Error::Unavailable("invalid source digest".into()))?;
                    self.object_path("sources", hash)?;
                    live.insert(("sources".to_string(), hash.to_string()));
                }
            } else {
                remove_snapshots.push(snapshot.id.clone());
            }
        }
        remove_snapshots.sort();
        let mut objects = Vec::new();
        let mut category_bytes = BTreeMap::<String, u64>::new();
        let mut statement = connection.prepare("SELECT category,hash,device,inode,bytes FROM owned_objects WHERE created_at<=?1 ORDER BY category,hash")?;
        for row in statement.query_map([cutoff], |row| {
            Ok(GcObject {
                category: row.get(0)?,
                hash: row.get(1)?,
                device: row.get::<_, i64>(2)? as u64,
                inode: row.get::<_, i64>(3)? as u64,
                bytes: row.get::<_, i64>(4)? as u64,
                present: true,
            })
        })? {
            let mut object = row?;
            if live.contains(&(object.category.clone(), object.hash.clone())) {
                continue;
            }
            self.object_path(&object.category, &object.hash)?;
            let directory = self.category_directory(&object.category)?;
            match statat(&directory, &object.hash, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat)
                    if stat.st_dev == object.device
                        && stat.st_ino == object.inode
                        && stat.st_size as u64 == object.bytes
                        && rustix::fs::FileType::from_raw_mode(stat.st_mode)
                            == rustix::fs::FileType::RegularFile => {}
                Ok(_) => {
                    return Err(Error::Unavailable(
                        "garbage object identity changed; refusing deletion".into(),
                    ));
                }
                Err(rustix::io::Errno::NOENT) => {
                    object.present = false;
                    object.bytes = 0;
                }
                Err(error) => return Err(std::io::Error::from(error).into()),
            }
            let bytes = category_bytes.entry(object.category.clone()).or_default();
            *bytes = bytes.saturating_add(object.bytes);
            objects.push(object);
        }
        let mut plan = GcPlan {
            fingerprint:String::new(),policy:policy.clone(),
            retained_snapshots:roots.into_iter().map(|(id,mut reasons)| {reasons.sort(); RetainedSnapshot{id,reasons}}).collect(),
            remove_snapshots,objects,category_bytes,live_objects:live.len(),
            limitations:vec!["Exclusive cross-process lease: garbage collection is unavailable while any snapshot reader or publisher is active.".into(),"Unregistered files, staging files, observations, and other store contents are never deleted. Older binaries without leases must not use this store concurrently.".into()],
        };
        plan.fingerprint = digest("gc_plan", &plan);
        Ok(plan)
    }

    pub fn gc_execute(&self, plan: &GcPlan) -> Result<GcReport> {
        self.gc_execute_inner(plan, None)
    }

    fn gc_execute_inner(&self, plan: &GcPlan, fail_after: Option<u8>) -> Result<GcReport> {
        let _lease = self.lease(true, &|| false)?;
        let mut connection = self.connect()?;
        let completed: Option<String> = connection
            .query_row(
                "SELECT report FROM gc_runs WHERE fingerprint=?1 AND completed=1",
                [&plan.fingerprint],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(report) = completed {
            return Ok(serde_json::from_str(&report)?);
        }
        let current = self.gc_plan_locked(&plan.policy)?;
        if current != *plan {
            return Err(Error::Unavailable(
                "garbage collection plan is stale; create a new dry-run plan".into(),
            ));
        }
        let report = report(plan, false);
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT OR REPLACE INTO gc_runs VALUES(?1,0,?2)",
            params![plan.fingerprint, serde_json::to_string(&report)?],
        )?;
        for id in &plan.remove_snapshots {
            transaction.execute("DELETE FROM snapshots WHERE id=?1", [&id.0])?;
        }
        transaction.commit()?;
        gc_failure(fail_after, 1)?;
        for object in &plan.objects {
            let directory = self.category_directory(&object.category)?;
            if object.present {
                let stat = statat(&directory, &object.hash, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(std::io::Error::from)?;
                if stat.st_dev != object.device
                    || stat.st_ino != object.inode
                    || stat.st_size as u64 != object.bytes
                    || rustix::fs::FileType::from_raw_mode(stat.st_mode)
                        != rustix::fs::FileType::RegularFile
                {
                    return Err(Error::Unavailable(
                        "owned object changed before deletion".into(),
                    ));
                }
                unlinkat(&directory, &object.hash, AtFlags::empty())
                    .map_err(std::io::Error::from)?;
                directory.sync_all()?;
            }
            gc_failure(fail_after, 2)?;
            connection.execute(
                "DELETE FROM owned_objects WHERE category=?1 AND hash=?2",
                params![object.category, object.hash],
            )?;
        }
        connection.execute(
            "UPDATE gc_runs SET completed=1 WHERE fingerprint=?1",
            [&plan.fingerprint],
        )?;
        Ok(report)
    }

    pub fn gc_dry_run(&self) -> Result<GcReport> {
        Ok(report(&self.gc_plan(&RetentionPolicy::default())?, true))
    }
}

fn report(plan: &GcPlan, dry_run: bool) -> GcReport {
    GcReport {
        dry_run,
        live_objects: plan.live_objects,
        unreferenced_objects: plan
            .objects
            .iter()
            .map(|object| format!("{}/{}", object.category, object.hash))
            .collect(),
        unreferenced_bytes: plan
            .category_bytes
            .values()
            .fold(0u64, |sum, bytes| sum.saturating_add(*bytes)),
        removed_snapshots: plan.remove_snapshots.clone(),
        category_bytes: plan.category_bytes.clone(),
        plan_fingerprint: plan.fingerprint.clone(),
        limitations: plan.limitations.clone(),
    }
}

fn gc_failure(fail_after: Option<u8>, stage: u8) -> Result<()> {
    #[cfg(test)]
    if fail_after == Some(stage)
        && std::env::var("ATLAS_TEST_GC_CRASH_STAGE").ok().as_deref()
            == Some(stage.to_string().as_str())
    {
        std::process::exit(78);
    }
    if fail_after == Some(stage) {
        return Err(Error::Unavailable(
            "injected garbage collection failure".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
