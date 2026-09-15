//! Bounded queries over a single immutable snapshot.
mod cursor;
mod diff;
mod error;
pub use error::{Error, Result};

use atlas_model::*;
use atlas_store::{SnapshotReader, Store};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const SOURCE_BYTES: usize = 256 * 1024;
const INTERACTIVE: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct QueryControl {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
}
impl QueryControl {
    pub fn new(duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }
}

#[derive(Clone)]
pub struct QueryEngine {
    store: Store,
    repositories: Option<BTreeSet<RepositoryId>>,
    secret: Arc<[u8; 32]>,
}

impl QueryEngine {
    pub fn new(store: Store) -> Result<Self> {
        let mut secret = [0; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut secret)?;
        Ok(Self {
            store,
            repositories: None,
            secret: Arc::new(secret),
        })
    }

    pub fn with_repositories(
        mut self,
        repositories: impl IntoIterator<Item = RepositoryId>,
    ) -> Self {
        self.repositories = Some(repositories.into_iter().collect());
        self
    }

    fn authorize(&self, id: &SnapshotId) -> Result<()> {
        if let Some(repositories) = &self.repositories {
            let repository = self
                .store
                .repository(id)
                .map_err(|_| Error::NotAuthorized)?;
            if !repositories.contains(&repository) {
                return Err(Error::NotAuthorized);
            }
        }
        Ok(())
    }

    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        Ok(self
            .store
            .snapshots_scoped_bounded(self.repositories.as_ref(), 1000, RESPONSE_BYTES)?)
    }

    pub fn snapshot(&self, id: &SnapshotId) -> Result<Snapshot> {
        self.authorize(id)?;
        Ok(self.store.snapshot(id)?)
    }

    pub fn set_snapshot_pin(
        &self,
        id: &SnapshotId,
        request: &SnapshotPinRequest,
        retained: bool,
    ) -> Result<SnapshotPinState> {
        let snapshot = self.snapshot(id)?;
        if snapshot.context.id != request.context_id {
            return Err(Error::ContextMismatch);
        }
        if request.name.is_empty()
            || request.name.len() > 128
            || !request
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
        {
            return Err(Error::InvalidQuery(
                "pin name requires 1..128 ASCII letters, digits, hyphens or underscores".into(),
            ));
        }
        // Scope the storage key before mutation; a bookmark label is never a global pin capability.
        let key = digest("bookmark-pin", &(id, &request.context_id, &request.name));
        if retained {
            self.store.pin(id, &key)?;
        } else {
            self.store.unpin(&key)?;
        }
        Ok(SnapshotPinState {
            snapshot_id: id.clone(),
            context_id: request.context_id.clone(),
            name: request.name.clone(),
            retained,
        })
    }

    fn reader(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        control: &QueryControl,
    ) -> Result<SnapshotReader> {
        self.authorize(snapshot)?;
        if self.store.snapshot(snapshot)?.context.id != *context {
            return Err(Error::ContextMismatch);
        }
        let reader = self
            .store
            .reader_with_stop(snapshot, &|| control.stopped())?;
        let control = control.clone();
        reader.progress_handler(move || control.stopped())?;
        Ok(reader)
    }

    pub fn search(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        query: &str,
        limit: usize,
        next: Option<&str>,
    ) -> Result<QueryResponse<Definition>> {
        let started = Instant::now();
        let control = QueryControl::new(INTERACTIVE);
        if !(1..=200).contains(&limit) || query.len() > 256 {
            return Err(Error::InvalidQuery(
                "search requires limit 1..200 and at most 256 query bytes".into(),
            ));
        }
        let reader = match self.reader(snapshot, context, &control) {
            Ok(reader) => reader,
            Err(error) if error.code() == "budget_exhausted" => {
                let mut coverage = self.snapshot(snapshot)?.coverage;
                budget_coverage(
                    &mut coverage,
                    "Search deadline reached during snapshot validation",
                );
                return bounded(QueryResponse {
                    api_version: API_VERSION.into(),
                    snapshot_id: snapshot.clone(),
                    context_id: context.clone(),
                    items: vec![],
                    coverage,
                    page: Page {
                        truncated: true,
                        next_cursor: None,
                    },
                    work: Work {
                        deadline_reached: true,
                        elapsed_ms: elapsed(started),
                    },
                });
            }
            Err(error) => return Err(error),
        };
        let query = query.trim().to_lowercase();
        let query_hash = digest("search", &query);
        let scope = digest("authorization", &self.repositories);
        let cursor = next
            .map(|value| cursor::decode(value, &self.secret))
            .transpose()?;
        if let Some(cursor) = &cursor
            && (cursor.snapshot != *snapshot
                || cursor.context != *context
                || cursor.query != query_hash
                || cursor.scope != scope)
        {
            return Err(Error::InvalidCursor);
        }
        let mut coverage = reader.snapshot.coverage.clone();
        let mut deadline_reached = control.stopped();
        let mut read_exhausted = deadline_reached;
        let mut items = if deadline_reached {
            Vec::new()
        } else {
            match reader.search(
                &query,
                cursor
                    .as_ref()
                    .map(|c| (c.last_name.as_str(), c.last_id.as_str())),
                limit + 1,
            ) {
                Ok(items) => items,
                Err(error) if error.code() == "budget_exhausted" => {
                    read_exhausted = true;
                    deadline_reached = control.stopped();
                    Vec::new()
                }
                Err(error) => return Err(error.into()),
            }
        };
        let mut truncated = items.len() > limit || read_exhausted;
        items.truncate(limit);
        let mut bytes = 4096;
        let mut retained = 0;
        for item in &items {
            let size = size(item)?;
            if bytes + size > RESPONSE_BYTES {
                truncated = true;
                break;
            }
            bytes += size;
            retained += 1;
        }
        items.truncate(retained);
        let next_cursor = if truncated && !read_exhausted {
            items
                .last()
                .map(|last| {
                    cursor::encode(
                        &cursor::Cursor {
                            version: 1,
                            snapshot: snapshot.clone(),
                            context: context.clone(),
                            scope,
                            query: query_hash,
                            last_name: last.name.to_lowercase(),
                            last_id: last.id.0.clone(),
                            expires: cursor::now() + 900,
                        },
                        &self.secret,
                    )
                })
                .transpose()?
        } else {
            None
        };
        if read_exhausted {
            budget_coverage(
                &mut coverage,
                "Search reached a time or payload read budget; result is incomplete",
            );
        }
        if truncated && items.is_empty() && !read_exhausted {
            budget_coverage(
                &mut coverage,
                "A definition exceeds the response byte budget",
            );
        }
        let result = QueryResponse {
            api_version: API_VERSION.into(),
            snapshot_id: snapshot.clone(),
            context_id: context.clone(),
            items,
            coverage,
            page: Page {
                truncated,
                next_cursor,
            },
            work: Work {
                deadline_reached,
                elapsed_ms: elapsed(started),
            },
        };
        bounded(result)
    }

    pub fn definition(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        id: &DefinitionId,
    ) -> Result<DefinitionDetail> {
        let control = QueryControl::new(INTERACTIVE);
        let reader = self.reader(snapshot, context, &control)?;
        let definition = reader.definition(id)?.ok_or(Error::NotFound)?;
        let mut evidence = reader.definition_evidence(id, 201)?;
        let mut coverage = reader.snapshot.coverage.clone();
        if evidence.len() > 200 {
            evidence.truncate(200);
            budget_coverage(&mut coverage, "Evidence list is limited to 200 records");
        }
        bounded(DefinitionDetail {
            snapshot_id: snapshot.clone(),
            definition,
            evidence,
            coverage,
        })
    }

    pub fn evidence(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        id: &EvidenceId,
    ) -> Result<Evidence> {
        let reader = self.reader(snapshot, context, &QueryControl::new(INTERACTIVE))?;
        bounded(reader.evidence(id)?.ok_or(Error::NotFound)?)
    }

    pub fn flow(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        id: &DefinitionId,
        phase: &str,
    ) -> Result<FunctionFlow> {
        let reader = self.reader(snapshot, context, &QueryControl::new(INTERACTIVE))?;
        if phase != "source" {
            return Err(Error::UnsupportedCapability);
        }
        if reader.definition(id)?.is_none() {
            return Err(Error::NotFound);
        }
        bounded(reader.flow(id, phase)?.unwrap_or_else(|| FunctionFlow {
            definition_id: id.clone(),
            phase: phase.into(),
            points: vec![],
            coverage: Coverage {
                status: Status::Unavailable,
                reasons: vec![ReasonCount {
                    reason: UnknownReason::UnsupportedConstruct,
                    count: 1,
                }],
                limitations: vec!["Source flow is unavailable for this definition".into()],
            },
        }))
    }

    pub fn source(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        file: &FileId,
        start_line: u32,
        line_count: u32,
    ) -> Result<SourceWindow> {
        let reader = self.reader(snapshot, context, &QueryControl::new(INTERACTIVE))?;
        let source = reader.source(file)?.ok_or(Error::NotFound)?;
        source_window(snapshot, source, start_line, line_count)
    }

    pub fn source_at_byte(
        &self,
        snapshot: &SnapshotId,
        context: &ContextId,
        file: &FileId,
        offset: u32,
        line_count: u32,
    ) -> Result<SourceWindow> {
        let reader = self.reader(snapshot, context, &QueryControl::new(INTERACTIVE))?;
        let source = reader.source(file)?.ok_or(Error::NotFound)?;
        if offset as usize > source.text.len() || !source.text.is_char_boundary(offset as usize) {
            return Err(Error::InvalidQuery(
                "source byte offset is out of bounds or splits UTF-8".into(),
            ));
        }
        let line = source.text.as_bytes()[..offset as usize]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count() as u32
            + 1;
        source_window(snapshot, source, line, line_count)
    }

    pub fn neighborhood(&self, request: &GraphRequest) -> Result<GraphResponse> {
        self.neighborhood_with_control(request, &QueryControl::new(INTERACTIVE))
    }

    pub fn neighborhood_with_control(
        &self,
        request: &GraphRequest,
        control: &QueryControl,
    ) -> Result<GraphResponse> {
        let started = Instant::now();
        if request.max_nodes == 0
            || request.max_nodes > 200
            || request.max_edges == 0
            || request.max_edges > 500
            || request.depth > 4
        {
            return Err(Error::InvalidQuery(
                "graph limits: 1..200 nodes, 1..500 edges, depth 0..4".into(),
            ));
        }
        let reader = match self.reader(&request.snapshot_id, &request.context_id, control) {
            Ok(reader) => reader,
            Err(error) if error.code() == "budget_exhausted" => {
                let mut coverage = self.snapshot(&request.snapshot_id)?.coverage;
                budget_coverage(
                    &mut coverage,
                    "Graph deadline or cancellation reached during snapshot validation",
                );
                return bounded(GraphResponse {
                    api_version: API_VERSION.into(),
                    snapshot_id: request.snapshot_id.clone(),
                    context_id: request.context_id.clone(),
                    nodes: vec![],
                    edges: vec![],
                    coverage,
                    page: Page {
                        truncated: true,
                        next_cursor: None,
                    },
                    work: Work {
                        deadline_reached: true,
                        elapsed_ms: elapsed(started),
                    },
                });
            }
            Err(error) => return Err(error),
        };
        let root = reader
            .definition(&request.definition_id)?
            .ok_or(Error::NotFound)?;
        let mut coverage = reader.snapshot.coverage.clone();
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();
        let mut queue = VecDeque::new();
        let mut visited = BTreeSet::new();
        let mut bytes = size(&root)? + 4096;
        let mut truncated = bytes > RESPONSE_BYTES;
        if !truncated {
            nodes.insert(root.id.clone(), root);
            queue.push_back((request.definition_id.clone(), 0));
        }
        'walk: while let Some((id, depth)) = queue.pop_front() {
            if control.stopped() {
                truncated = true;
                break;
            }
            if depth >= request.depth || !visited.insert(id.clone()) {
                continue;
            }
            let adjacent =
                match reader.adjacency(&id, &request.direction, request.max_edges as usize + 1) {
                    Ok(items) => items,
                    Err(error) if error.code() == "budget_exhausted" => {
                        truncated = true;
                        break;
                    }
                    Err(error) => return Err(error.into()),
                };
            for relation in adjacent {
                if control.stopped() {
                    truncated = true;
                    break 'walk;
                }
                if edges.contains_key(&relation.id) {
                    continue;
                }
                if edges.len() >= request.max_edges as usize {
                    truncated = true;
                    break 'walk;
                }
                let mut required = Vec::new();
                if !nodes.contains_key(&relation.source) {
                    required.push(relation.source.clone());
                }
                if let Target::Resolved { id } = &relation.target
                    && !nodes.contains_key(id)
                    && !required.contains(id)
                {
                    required.push(id.clone());
                }
                if nodes.len() + required.len() > request.max_nodes as usize {
                    truncated = true;
                    continue;
                }
                let mut additions = Vec::new();
                let mut extra_bytes = size(&relation)?;
                for id in required {
                    let definition = reader.definition(&id)?.ok_or_else(|| {
                        Error::Store(atlas_store::Error::Unavailable(
                            "relation references missing definition".into(),
                        ))
                    })?;
                    extra_bytes += size(&definition)?;
                    additions.push(definition);
                }
                if bytes + extra_bytes > RESPONSE_BYTES {
                    truncated = true;
                    break 'walk;
                }
                bytes += extra_bytes;
                for definition in additions {
                    nodes.insert(definition.id.clone(), definition);
                }
                match &request.direction {
                    Direction::Incoming => queue.push_back((relation.source.clone(), depth + 1)),
                    Direction::Outgoing => {
                        if let Target::Resolved { id } = &relation.target {
                            queue.push_back((id.clone(), depth + 1));
                        }
                    }
                    Direction::Both => {
                        queue.push_back((relation.source.clone(), depth + 1));
                        if let Target::Resolved { id } = &relation.target {
                            queue.push_back((id.clone(), depth + 1));
                        }
                    }
                }
                if let Target::Unknown { reason, .. } = &relation.target {
                    add_reason(&mut coverage, reason.clone());
                }
                edges.insert(relation.id.clone(), relation);
            }
        }
        let deadline_reached = control.stopped();
        if truncated || deadline_reached {
            budget_coverage(
                &mut coverage,
                "Graph expansion reached a node, edge, time, cancellation, or response byte budget",
            );
        }
        bounded(GraphResponse {
            api_version: API_VERSION.into(),
            snapshot_id: request.snapshot_id.clone(),
            context_id: request.context_id.clone(),
            nodes: nodes.into_values().collect(),
            edges: edges.into_values().collect(),
            coverage,
            page: Page {
                truncated: truncated || deadline_reached,
                next_cursor: None,
            },
            work: Work {
                deadline_reached,
                elapsed_ms: elapsed(started),
            },
        })
    }
}

fn source_window(
    snapshot: &SnapshotId,
    source: SourceFile,
    start_line: u32,
    line_count: u32,
) -> Result<SourceWindow> {
    if start_line == 0 || line_count == 0 || line_count > 1000 {
        return Err(Error::InvalidQuery(
            "source requires one-based lines and a count of 1..1000".into(),
        ));
    }
    let mut total_lines = 1u32;
    let mut start = 0;
    let mut requested_end = source.text.len();
    for (offset, byte) in source.text.bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        total_lines += 1;
        if total_lines == start_line {
            start = offset + 1;
        }
        if total_lines == start_line.saturating_add(line_count) {
            requested_end = offset + 1;
        }
    }
    if start_line > total_lines {
        return Err(Error::InvalidQuery("source line is out of bounds".into()));
    }
    let mut end = requested_end.min(start + SOURCE_BYTES);
    while !source.text.is_char_boundary(end) {
        end -= 1;
    }
    bounded(SourceWindow {
        snapshot_id: snapshot.clone(),
        file_id: source.id,
        path: source.path,
        content_hash: source.content_hash,
        text: source.text[start..end].to_owned(),
        start_line,
        total_lines,
        start_byte: start as u32,
        truncated: start > 0 || end < source.text.len(),
        encoding: "utf-8".into(),
    })
}

fn elapsed(started: Instant) -> u32 {
    started.elapsed().as_millis().min(u32::MAX as u128) as u32
}
fn size(value: &impl Serialize) -> Result<usize> {
    Ok(serde_json::to_vec(value)?.len())
}
fn bounded<T: Serialize>(value: T) -> Result<T> {
    if size(&value)? > RESPONSE_BYTES {
        Err(Error::BudgetExhausted)
    } else {
        Ok(value)
    }
}
fn add_reason(coverage: &mut Coverage, reason: UnknownReason) {
    if coverage.status == Status::Complete {
        coverage.status = Status::Partial;
    }
    if let Some(count) = coverage
        .reasons
        .iter_mut()
        .find(|count| count.reason == reason)
    {
        count.count = count.count.saturating_add(1);
    } else {
        coverage.reasons.push(ReasonCount { reason, count: 1 });
    }
}
fn budget_coverage(coverage: &mut Coverage, limitation: &str) {
    add_reason(coverage, UnknownReason::BudgetExhausted);
    coverage.limitations.push(limitation.into());
}

#[cfg(test)]
mod tests;
