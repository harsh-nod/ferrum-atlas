use crate::{Error, Result, Store, VerifiedShard};
use atlas_model::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

pub struct SnapshotReader {
    store: Store,
    pub snapshot: Snapshot,
    connection: Connection,
    shard_path: PathBuf,
    verified: VerifiedShard,
    _lease: Option<std::fs::File>,
}

impl SnapshotReader {
    pub(crate) fn complete_batch(&self, max_bytes: usize) -> Result<FactBatch> {
        self.verify()?;
        let envelope: Option<Option<String>> = self.connection.query_row(
            "SELECT CASE WHEN length(CAST(value AS BLOB))<=?1 THEN value ELSE NULL END FROM metadata WHERE key='fact_envelope'",
            [max_bytes as i64], |row| row.get(0)).optional()?;
        let envelope = envelope.ok_or_else(|| Error::Unavailable("complete export unavailable: legacy shard lacks a fact envelope; reindex into a fresh store (immutable existing snapshots are not upgraded in place)".into()))?.ok_or(Error::BudgetExhausted)?;
        let mut remaining = max_bytes
            .checked_sub(envelope.len())
            .ok_or(Error::BudgetExhausted)?;
        let mut batch: FactBatch = serde_json::from_str(&envelope)?;
        batch.definitions = self.export_rows("definitions", &mut remaining)?;
        batch.relations = self.export_rows("relations", &mut remaining)?;
        batch.evidence = self.export_rows("evidence", &mut remaining)?;
        batch.flows = self.export_rows("flows", &mut remaining)?;
        batch.source.files = Vec::new();
        if self.snapshot.file_count > 100_000 {
            return Err(Error::BudgetExhausted);
        }
        for file in self.files()? {
            let source = self
                .source(&file.id)?
                .ok_or_else(|| Error::Unavailable("missing archived source".into()))?;
            remaining = remaining
                .checked_sub(source.text.len())
                .ok_or(Error::BudgetExhausted)?;
            batch.source.files.push(source);
        }
        crate::validate(&batch)?;
        let batch = crate::validate::normalize(&batch);
        if digest("facts", &batch) != self.snapshot.fact_digest {
            return Err(Error::Unavailable(
                "complete snapshot fact digest mismatch".into(),
            ));
        }
        Ok(batch)
    }

    fn export_rows<T: DeserializeOwned>(
        &self,
        table: &str,
        remaining: &mut usize,
    ) -> Result<Vec<T>> {
        let (rows, bytes): (i64, i64) = self.connection.query_row(
            &format!("SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM {table}"),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if rows > 1_000_000 || bytes < 0 || bytes as u64 > *remaining as u64 {
            return Err(Error::BudgetExhausted);
        }
        *remaining -= bytes as usize;
        let mut statement = self
            .connection
            .prepare(&format!("SELECT payload FROM {table}"))?;
        let mut output = Vec::new();
        for row in statement.query_map([], |row| row.get::<_, String>(0))? {
            output.push(serde_json::from_str(&row?)?);
        }
        Ok(output)
    }

    pub(crate) fn open(
        store: Store,
        snapshot: Snapshot,
        path: &Path,
        lease: Option<std::fs::File>,
        verified: VerifiedShard,
    ) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let fact_digest: String = connection.query_row(
            "SELECT value FROM metadata WHERE key='fact_digest'",
            [],
            |r| r.get(0),
        )?;
        let context: String = connection.query_row(
            "SELECT value FROM metadata WHERE key='context_id'",
            [],
            |r| r.get(0),
        )?;
        if fact_digest != snapshot.fact_digest || context != snapshot.context.id.0 {
            return Err(Error::Unavailable("shard manifest mismatch".into()));
        }
        let reader = Self {
            store,
            snapshot,
            connection,
            shard_path: path.to_path_buf(),
            verified,
            _lease: lease,
        };
        reader.check_identity()?;
        Ok(reader)
    }

    pub(crate) fn check_identity(&self) -> Result<()> {
        self.verified.unchanged()?;
        let hash = self
            .shard_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::Unavailable("invalid shard path".into()))?;
        // statat avoids opening/closing another database descriptor, which can
        // release process-associated SQLite POSIX locks on the same inode.
        if self.store.shard_identity(hash)? != self.verified.identity {
            return Err(Error::Unavailable(
                "shard replaced after verification".into(),
            ));
        }
        Ok(())
    }

    pub fn progress_handler(&self, callback: impl FnMut() -> bool + Send + 'static) -> Result<()> {
        self.connection.progress_handler(1000, Some(callback))?;
        Ok(())
    }

    pub fn verify(&self) -> Result<()> {
        self.check_identity()?;
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if result != "ok" {
            return Err(Error::Unavailable("shard integrity check failed".into()));
        }
        let foreign: Option<String> = self
            .connection
            .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
            .optional()?;
        if foreign.is_some() {
            return Err(Error::Unavailable("shard reference check failed".into()));
        }
        for (table, count) in [
            ("files", self.snapshot.file_count),
            ("definitions", self.snapshot.definition_count),
            ("relations", self.snapshot.relation_count),
        ] {
            let actual: u32 =
                self.connection
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
            if actual != count {
                return Err(Error::Unavailable("shard count mismatch".into()));
            }
        }
        Ok(())
    }

    pub fn definition(&self, id: &DefinitionId) -> Result<Option<Definition>> {
        self.one("SELECT payload FROM definitions WHERE id=?1", &id.0)
    }
    pub fn evidence(&self, id: &EvidenceId) -> Result<Option<Evidence>> {
        self.one("SELECT payload FROM evidence WHERE id=?1", &id.0)
    }

    fn one<T: DeserializeOwned>(&self, sql: &str, id: &str) -> Result<Option<T>> {
        self.check_identity()?;
        self.connection
            .query_row(sql, [id], |r| r.get::<_, String>(0))
            .optional()?
            .map(|json| serde_json::from_str(&json).map_err(Error::from))
            .transpose()
    }

    pub fn search(
        &self,
        prefix: &str,
        after: Option<(&str, &str)>,
        limit: usize,
    ) -> Result<Vec<Definition>> {
        self.check_identity()?;
        let upper = format!("{prefix}\u{10ffff}");
        let (after_name, after_id) = after.unwrap_or(("", ""));
        let mut statement = self.connection.prepare("SELECT payload FROM definitions WHERE name_key>=?1 AND name_key<?2 AND (name_key,id)>(?3,?4) ORDER BY name_key,id LIMIT ?5")?;
        collect(statement.query_map(
            params![
                prefix,
                upper,
                after_name,
                after_id,
                limit.min(10_001) as u32
            ],
            |r| r.get::<_, String>(0),
        )?)
    }

    pub fn definitions(&self, limit: usize) -> Result<Vec<Definition>> {
        self.definitions_bounded(limit, 256 * 1024 * 1024)
    }

    pub fn definitions_bounded(&self, limit: usize, max_bytes: usize) -> Result<Vec<Definition>> {
        self.check_identity()?;
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM definitions ORDER BY id LIMIT ?1")?;
        collect_bounded(
            statement.query_map([limit.min(100_001) as u32], |r| r.get::<_, String>(0))?,
            max_bytes,
        )
    }

    pub fn adjacency(
        &self,
        id: &DefinitionId,
        direction: &Direction,
        limit: usize,
    ) -> Result<Vec<Relation>> {
        self.check_identity()?;
        let sql = match direction {
            Direction::Incoming => {
                "SELECT payload FROM relations WHERE target_id=?1 AND kind='calls' ORDER BY kind,source_id,id LIMIT ?2"
            }
            Direction::Outgoing => {
                "SELECT payload FROM relations WHERE source_id=?1 AND kind='calls' ORDER BY kind,target_id,id LIMIT ?2"
            }
            Direction::Both => {
                "SELECT payload FROM (SELECT id,payload FROM relations WHERE source_id=?1 AND kind='calls' UNION SELECT id,payload FROM relations WHERE target_id=?1 AND kind='calls') ORDER BY id LIMIT ?2"
            }
        };
        let mut statement = self.connection.prepare(sql)?;
        collect(
            statement.query_map(params![id.0, limit.min(10_001) as u32], |r| {
                r.get::<_, String>(0)
            })?,
        )
    }

    pub fn definition_evidence(&self, id: &DefinitionId, limit: usize) -> Result<Vec<Evidence>> {
        self.check_identity()?;
        let mut statement = self.connection.prepare("SELECT e.payload FROM evidence e WHERE e.id IN (SELECT DISTINCT evidence_id FROM relations WHERE source_id=?1 ORDER BY evidence_id LIMIT ?2) ORDER BY e.id LIMIT ?2")?;
        collect(
            statement.query_map(params![id.0, limit.min(1001) as u32], |r| {
                r.get::<_, String>(0)
            })?,
        )
    }

    pub fn flow(&self, id: &DefinitionId, phase: &str) -> Result<Option<FunctionFlow>> {
        self.check_identity()?;
        self.connection
            .query_row(
                "SELECT payload FROM flows WHERE definition_id=?1 AND phase=?2",
                params![id.0, phase],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|json| serde_json::from_str(&json).map_err(Error::from))
            .transpose()
    }

    pub fn files(&self) -> Result<Vec<SourceFile>> {
        self.check_identity()?;
        let mut statement = self
            .connection
            .prepare("SELECT id,path,content_hash FROM files ORDER BY id")?;
        Ok(statement
            .query_map([], |r| {
                Ok(SourceFile {
                    id: FileId(r.get(0)?),
                    path: r.get(1)?,
                    content_hash: r.get(2)?,
                    text: String::new(),
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn source(&self, id: &FileId) -> Result<Option<SourceFile>> {
        self.check_identity()?;
        let Some(mut file) = self
            .connection
            .query_row(
                "SELECT path,content_hash FROM files WHERE id=?1",
                [&id.0],
                |r| {
                    Ok(SourceFile {
                        id: id.clone(),
                        path: r.get(0)?,
                        content_hash: r.get(1)?,
                        text: String::new(),
                    })
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let hash = file
            .content_hash
            .strip_prefix("content:")
            .ok_or_else(|| Error::Unavailable("invalid content digest".into()))?;
        let path = self.store.object_path("sources", hash)?;
        if std::fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
            return Err(Error::Unavailable(
                "source object exceeds local 64 MiB limit".into(),
            ));
        }
        file.text = std::fs::read_to_string(path)?;
        if digest("content", &file.text) != file.content_hash {
            return Err(Error::Unavailable("source checksum mismatch".into()));
        }
        Ok(Some(file))
    }
}

fn collect<T: DeserializeOwned>(
    rows: impl Iterator<Item = std::result::Result<String, rusqlite::Error>>,
) -> Result<Vec<T>> {
    collect_bounded(rows, 2 * 1024 * 1024)
}

fn collect_bounded<T: DeserializeOwned>(
    rows: impl Iterator<Item = std::result::Result<String, rusqlite::Error>>,
    max_bytes: usize,
) -> Result<Vec<T>> {
    let mut bytes = 0usize;
    rows.map(|row| {
        let json = row?;
        bytes = bytes.saturating_add(json.len());
        if bytes > max_bytes {
            return Err(Error::BudgetExhausted);
        }
        Ok(serde_json::from_str(&json)?)
    })
    .collect()
}

#[cfg(test)]
mod tests;
