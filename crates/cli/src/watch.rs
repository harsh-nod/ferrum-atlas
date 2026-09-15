//! A disposable warm analysis process; only the supervisor publishes snapshots.
use super::{ChildGuard, IndexJob, Level, directory_bytes, private_json, progress};
use anyhow::{Context, Result, bail, ensure};
use atlas_frontend::{AnalysisLevel, AnalyzerSession};
use atlas_ingest::CaptureOptions;
use atlas_model::{ContextId, FactBatch, JobStage, SnapshotId, SourceId};
use atlas_store::Store;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::{
        fs::OpenOptionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const VERSION: u32 = 1;
const MAX_CAPTURES: u32 = 32;
const MAX_FRAME_BYTES: usize = 16 * 1024;
const MAX_BATCH_BYTES: u64 = 256 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(25);

pub(super) struct WatchConfig {
    pub store: PathBuf,
    pub job: IndexJob,
    pub interval_ms: u64,
    pub iterations: Option<u64>,
    pub disk_quota_mib: u64,
}

#[derive(Debug, Default, Serialize)]
pub(super) struct WatchReport {
    pub iterations: u64,
    pub published: u64,
    pub unchanged: u64,
    pub failures: u64,
    pub worker_restarts: u64,
    pub cancelled: bool,
    pub last_snapshot: Option<SnapshotId>,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    sequence: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Changed,
    Unchanged,
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    sequence: u32,
    status: Status,
    source_id: Option<SourceId>,
    context_id: Option<ContextId>,
    error: Option<String>,
}

impl Response {
    fn validate(&self, previous: Option<&(SourceId, ContextId)>) -> Result<()> {
        match self.status {
            Status::Changed => ensure!(
                self.source_id.is_some() && self.context_id.is_some() && self.error.is_none(),
                "invalid changed response"
            ),
            Status::Unchanged => ensure!(
                self.error.is_none()
                    && previous
                        .is_some_and(|(source, context)| self.source_id.as_ref() == Some(source)
                            && self.context_id.as_ref() == Some(context)),
                "unchanged response has no matching successful capture"
            ),
            Status::Failed => ensure!(
                self.source_id.is_none() && self.context_id.is_none() && self.error.is_some(),
                "invalid failed response"
            ),
        }
        Ok(())
    }
}

pub(super) fn run(mut config: WatchConfig, cancelled: Arc<AtomicBool>) -> Result<WatchReport> {
    ensure!(
        (50..=60_000).contains(&config.interval_ms),
        "watch interval must be between 50 and 60000 ms"
    );
    ensure!(
        config
            .iterations
            .is_none_or(|count| (1..=1_000_000).contains(&count)),
        "watch iterations must be between 1 and 1000000"
    );
    validate_job(&config.job)?;
    config.job.workspace = config.job.workspace.canonicalize()?;
    config.job.parent_pid = std::process::id();
    let store = Store::open(&config.store)?;
    config.store = config.store.canonicalize()?;
    let lease = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(config.store.join("index.lock"))?;
    lease
        .try_lock()
        .context("another index job or watch is active for this store")?;
    let quota = config
        .disk_quota_mib
        .checked_mul(1024 * 1024)
        .context("disk quota overflow")?;
    ensure!(quota > 0, "disk quota must be positive");
    let mut expected_head = store.head(&config.job.profile)?;
    let mut report = WatchReport {
        last_snapshot: expected_head.clone(),
        ..Default::default()
    };
    let mut session: Option<Session> = None;
    let mut starts = 0_u64;
    let mut transport_failures = 0;
    while config
        .iterations
        .is_none_or(|count| report.iterations < count)
    {
        if cancelled.load(Ordering::Acquire) {
            report.cancelled = true;
            break;
        }
        ensure!(
            store.head(&config.job.profile)? == expected_head,
            "watch head changed outside its index lease; refusing to overwrite it"
        );
        ensure!(
            directory_bytes(&config.store)? < quota.saturating_mul(80) / 100,
            "ingestion paused: store is at 80% of its disk quota"
        );
        if session
            .as_ref()
            .is_some_and(|session| session.captures >= MAX_CAPTURES)
        {
            session = None;
        }
        let deadline = Instant::now() + Duration::from_secs(config.job.timeout);
        let result = (|| {
            if session.is_none() {
                starts += 1;
                report.worker_restarts = starts - 1;
                session = Some(Session::start(
                    &config.store,
                    &config.job,
                    deadline,
                    &cancelled,
                )?);
            }
            session.as_mut().unwrap().capture(deadline, &cancelled)
        })();
        if cancelled.load(Ordering::Acquire) {
            report.cancelled = true;
            break;
        }
        report.iterations += 1;
        match result {
            Err(error) => {
                session = None;
                transport_failures += 1;
                record_failure(&mut report, &error);
                ensure!(
                    transport_failures < 3,
                    "watch worker failed three consecutive requests; last valid snapshot is retained: {error:#}"
                );
            }
            Ok((response, batch)) => {
                transport_failures = 0;
                match response.status {
                    Status::Failed => record_failure(
                        &mut report,
                        &anyhow::anyhow!(response.error.context("failed response has no reason")?),
                    ),
                    Status::Unchanged => {
                        ensure!(
                            batch.is_none(),
                            "unchanged response unexpectedly includes facts"
                        );
                        report.unchanged += 1;
                    }
                    Status::Changed => {
                        let batch = batch.context("changed response has no facts")?;
                        progress(config.job.progress_file.as_deref(), JobStage::Validation)?;
                        let fact_digest = atlas_store::fact_digest(&batch);
                        let already_published = expected_head
                            .as_ref()
                            .map(|head| store.snapshot(head))
                            .transpose()?
                            .is_some_and(|snapshot| snapshot.fact_digest == fact_digest);
                        if already_published {
                            report.unchanged += 1;
                        } else {
                            let bytes = serde_json::to_vec(&batch)?.len() as u64;
                            ensure!(
                                directory_bytes(&config.store)?
                                    .saturating_add(bytes.saturating_mul(3))
                                    < quota.saturating_mul(90) / 100,
                                "publication paused: estimated output would exceed 90% of disk quota"
                            );
                            if cancelled.load(Ordering::Acquire) {
                                report.cancelled = true;
                                break;
                            }
                            progress(config.job.progress_file.as_deref(), JobStage::Publication)?;
                            let snapshot = store.publish(
                                &batch,
                                &config.job.profile,
                                expected_head.as_ref(),
                            )?;
                            eprintln!(
                                "Watch published {} definitions and {} relations: {}",
                                snapshot.definition_count, snapshot.relation_count, snapshot.id
                            );
                            expected_head = Some(snapshot.id.clone());
                            report.last_snapshot = Some(snapshot.id);
                            report.published += 1;
                        }
                    }
                }
            }
        }
        if config
            .iterations
            .is_some_and(|count| report.iterations >= count)
        {
            break;
        }
        let next = Instant::now() + Duration::from_millis(config.interval_ms);
        while Instant::now() < next && !cancelled.load(Ordering::Acquire) {
            thread::sleep(POLL.min(next.saturating_duration_since(Instant::now())));
        }
    }
    drop(session);
    drop(lease);
    ensure!(
        report.cancelled || report.failures == 0 || report.published + report.unchanged > 0,
        "watch completed without a successful capture; last valid snapshot is retained: {}",
        report.last_error.as_deref().unwrap_or("unknown failure")
    );
    Ok(report)
}

fn record_failure(report: &mut WatchReport, error: &anyhow::Error) {
    let message: String = format!("{error:#}").chars().take(1024).collect();
    eprintln!("Watch capture failed; last valid snapshot remains available: {message}");
    report.failures += 1;
    report.last_error = Some(message);
}

struct Session {
    child: ChildGuard,
    stream: UnixStream,
    scratch: tempfile::TempDir,
    captures: u32,
    previous: Option<(SourceId, ContextId)>,
}

impl Session {
    fn start(
        store: &Path,
        job: &IndexJob,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        let scratch = tempfile::Builder::new()
            .prefix("atlas-watch-")
            .tempdir_in(store)?;
        let listener = UnixListener::bind(scratch.path().join("control.sock"))
            .context("create private watch socket")?;
        listener.set_nonblocking(true)?;
        let job_path = scratch.path().join("job.json");
        private_json(&job_path, job)?;
        let child = Command::new(std::env::current_exe()?)
            .args(["worker-session", "--job"])
            .arg(&job_path)
            .env_clear()
            .current_dir(scratch.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("start warm analysis worker")?;
        let mut child = ChildGuard(child);
        let stream = loop {
            check_wait(deadline, cancelled)?;
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    ensure!(
                        child.0.try_wait()?.is_none(),
                        "warm worker exited before connecting"
                    );
                    thread::sleep(POLL);
                }
                Err(error) => return Err(error.into()),
            }
        };
        configure_stream(&stream)?;
        Ok(Self {
            child,
            stream,
            scratch,
            captures: 0,
            previous: None,
        })
    }

    fn capture(
        &mut self,
        deadline: Instant,
        cancelled: &AtomicBool,
    ) -> Result<(Response, Option<FactBatch>)> {
        ensure!(self.captures < MAX_CAPTURES, "worker capture limit reached");
        self.captures += 1;
        write_frame(
            &mut self.stream,
            &Request {
                version: VERSION,
                sequence: self.captures,
            },
        )?;
        let response: Response = read_frame(&mut self.stream, || check_wait(deadline, cancelled))?;
        ensure!(
            response.version == VERSION && response.sequence == self.captures,
            "watch response version or sequence mismatch"
        );
        ensure!(
            response
                .error
                .as_ref()
                .is_none_or(|error| error.len() <= 4096),
            "watch error message exceeds limit"
        );
        response.validate(self.previous.as_ref())?;
        let batch = if response.status == Status::Changed {
            let path = self.scratch.path().join("facts.json");
            ensure!(
                fs::metadata(&path)?.len() <= MAX_BATCH_BYTES,
                "worker output exceeds 256 MiB"
            );
            let parsed = serde_json::from_reader::<_, FactBatch>(File::open(&path)?);
            fs::remove_file(path)?;
            let batch = parsed?;
            atlas_store::validate(&batch)?;
            ensure!(
                response.source_id.as_ref() == Some(&batch.source.id)
                    && response.context_id.as_ref() == Some(&batch.context.id),
                "watch output identities do not match response"
            );
            ensure!(
                response.error.is_none(),
                "changed response contains an error"
            );
            self.previous = Some((batch.source.id.clone(), batch.context.id.clone()));
            Some(batch)
        } else {
            ensure!(
                !self.scratch.path().join("facts.json").exists(),
                "worker left unclaimed facts"
            );
            None
        };
        Ok((response, batch))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Reap before the TempDir field removes files still owned by the worker.
        if !matches!(self.child.0.try_wait(), Ok(Some(_))) {
            let _ = self.child.0.kill();
            let _ = self.child.0.wait();
        }
    }
}

fn check_wait(deadline: Instant, cancelled: &AtomicBool) -> Result<()> {
    ensure!(!cancelled.load(Ordering::Acquire), "watch cancelled");
    ensure!(
        Instant::now() < deadline,
        "watch analysis deadline exceeded"
    );
    Ok(())
}

fn configure_stream(stream: &UnixStream) -> Result<()> {
    stream.set_read_timeout(Some(POLL))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    Ok(())
}

fn write_frame(stream: &mut UnixStream, frame: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(frame)?;
    ensure!(
        bytes.len() < MAX_FRAME_BYTES,
        "watch protocol frame exceeds limit"
    );
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn read_frame<T: DeserializeOwned>(
    stream: &mut UnixStream,
    mut check: impl FnMut() -> Result<()>,
) -> Result<T> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        check()?;
        match stream.read(&mut chunk) {
            Ok(0) => bail!("watch protocol peer closed"),
            Ok(count) => {
                ensure!(
                    bytes.len() + count <= MAX_FRAME_BYTES,
                    "watch protocol frame exceeds limit"
                );
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
                    ensure!(
                        end + 1 == bytes.len(),
                        "watch protocol does not allow pipelined frames"
                    );
                    return Ok(serde_json::from_slice(&bytes[..end])?);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn validate_job(job: &IndexJob) -> Result<()> {
    ensure!(
        (1..=3600).contains(&job.timeout),
        "timeout must be between 1 and 3600 seconds"
    );
    ensure!(
        (256..=131072).contains(&job.memory_mib),
        "memory budget must be between 256 and 131072 MiB"
    );
    ensure!(
        !job.profile.is_empty() && job.profile.len() <= 256,
        "invalid profile name"
    );
    ensure!(job.workspace.is_dir(), "workspace must be a directory");
    Ok(())
}

pub(super) fn worker(job_path: &Path) -> Result<()> {
    ensure!(
        fs::metadata(job_path)?.len() <= 1024 * 1024,
        "worker job too large"
    );
    let job: IndexJob = serde_json::from_reader(File::open(job_path)?)?;
    validate_job(&job)?;
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
    ensure!(
        rustix::process::getppid()
            .is_some_and(|pid| pid.as_raw_nonzero().get() as u32 == job.parent_pid),
        "analysis parent exited before worker initialization"
    );
    use rustix::process::{Resource, Rlimit, setrlimit};
    let memory = job
        .memory_mib
        .checked_mul(1024 * 1024)
        .context("memory limit overflow")?;
    setrlimit(
        Resource::As,
        Rlimit {
            current: Some(memory),
            maximum: Some(memory),
        },
    )?;
    // CPU is cumulative for this finite session; the supervisor separately
    // enforces the requested wall deadline for every individual capture.
    let cpu = job.timeout * u64::from(MAX_CAPTURES);
    setrlimit(
        Resource::Cpu,
        Rlimit {
            current: Some(cpu),
            maximum: Some(cpu + 1),
        },
    )?;
    setrlimit(
        Resource::Fsize,
        Rlimit {
            current: Some(MAX_BATCH_BYTES),
            maximum: Some(MAX_BATCH_BYTES),
        },
    )?;
    let scratch = job_path.parent().context("job path has no parent")?;
    let mut stream = UnixStream::connect(scratch.join("control.sock"))?;
    configure_stream(&stream)?;
    let output = scratch.join("facts.json");
    let options = CaptureOptions {
        profile: job.profile,
        target: job.target,
        features: job.features,
        default_features: job.default_features,
        cfg: job.cfg,
        explicit_context: job.explicit_context,
        ..Default::default()
    };
    let level = match job.level {
        Level::Syntax => AnalysisLevel::Syntax,
        Level::Semantic => AnalysisLevel::Semantic,
    };
    let mut analyzer = AnalyzerSession::new();
    let mut previous = None;
    for sequence in 1..=MAX_CAPTURES {
        let request: Request = read_frame(&mut stream, || Ok(()))?;
        ensure!(
            request.version == VERSION && request.sequence == sequence,
            "invalid watch request version or sequence"
        );
        ensure!(
            !output.exists(),
            "previous watch facts have not been consumed"
        );
        let captured = (|| -> Result<Response> {
            progress(job.progress_file.as_deref(), JobStage::Capture)?;
            let (source, context) = atlas_ingest::capture(&job.workspace, &options)?;
            let key = (source.id.clone(), context.id.clone());
            let changed = previous.as_ref() != Some(&key);
            if changed {
                progress(
                    job.progress_file.as_deref(),
                    match job.level {
                        Level::Syntax => JobStage::Syntax,
                        Level::Semantic => JobStage::Semantics,
                    },
                )?;
                let facts = analyzer.analyze(source, context, level)?;
                private_json(&output, &facts)?;
                previous = Some(key.clone());
            }
            Ok(Response {
                version: VERSION,
                sequence,
                status: if changed {
                    Status::Changed
                } else {
                    Status::Unchanged
                },
                source_id: Some(key.0),
                context_id: Some(key.1),
                error: None,
            })
        })();
        let response = match captured {
            Ok(response) => response,
            Err(error) => {
                if output.exists() {
                    fs::remove_file(&output)?;
                }
                Response {
                    version: VERSION,
                    sequence,
                    status: Status::Failed,
                    source_id: None,
                    context_id: None,
                    error: Some(format!("{error:#}").chars().take(1024).collect()),
                }
            }
        };
        write_frame(&mut stream, &response)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_roundtrip_is_bounded_and_rejects_extra_fields() {
        let (mut left, mut right) = UnixStream::pair().unwrap();
        configure_stream(&left).unwrap();
        configure_stream(&right).unwrap();
        write_frame(
            &mut left,
            &Request {
                version: VERSION,
                sequence: 1,
            },
        )
        .unwrap();
        let request: Request = read_frame(&mut right, || Ok(())).unwrap();
        assert_eq!(request.sequence, 1);
        left.write_all(b"{\"version\":1,\"sequence\":2,\"command\":\"forbidden\"}\n")
            .unwrap();
        assert!(read_frame::<Request>(&mut right, || Ok(())).is_err());
        assert!(write_frame(&mut left, &"x".repeat(MAX_FRAME_BYTES)).is_err());
        left.write_all(&vec![b'x'; MAX_FRAME_BYTES + 1]).unwrap();
        assert!(read_frame::<Request>(&mut right, || Ok(())).is_err());
    }

    #[test]
    fn waiting_for_protocol_is_interruptible_and_deadline_bounded() {
        let (_left, mut right) = UnixStream::pair().unwrap();
        configure_stream(&right).unwrap();
        let cancel = AtomicBool::new(true);
        assert!(
            read_frame::<Request>(&mut right, || check_wait(
                Instant::now() + Duration::from_secs(10),
                &cancel
            ))
            .is_err()
        );
        cancel.store(false, Ordering::Release);
        let deadline = Instant::now() + Duration::from_millis(50);
        assert!(read_frame::<Request>(&mut right, || check_wait(deadline, &cancel)).is_err());
    }

    #[test]
    fn unchanged_response_requires_matching_previous_success() {
        let source = SourceId("source:previous".into());
        let context = ContextId("context:selected".into());
        let mut response = Response {
            version: VERSION,
            sequence: 2,
            status: Status::Unchanged,
            source_id: Some(source.clone()),
            context_id: Some(context.clone()),
            error: None,
        };
        assert!(response.validate(None).is_err());
        assert!(response.validate(Some(&(source, context))).is_ok());
        response.error = Some("not a success".into());
        assert!(response.validate(None).is_err());
        response.status = Status::Failed;
        assert!(response.validate(None).is_err());
        response.source_id = None;
        response.context_id = None;
        assert!(response.validate(None).is_ok());
    }
}
