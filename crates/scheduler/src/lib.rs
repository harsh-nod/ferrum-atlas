//! Durable, serial admission and cancellation for a registered local workspace.
use anyhow::{Context, Result, bail, ensure};
use atlas_model::*;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::{fs::OpenOptionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_HISTORY: usize = 128;
const MAX_EVENTS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerConfig {
    pub version: u32,
    pub timeout_seconds: u64,
    pub memory_mib: u64,
    pub disk_quota_mib: u64,
    pub max_queued: usize,
}
impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            version: 1,
            timeout_seconds: 120,
            memory_mib: 8192,
            disk_quota_mib: 4096,
            max_queued: 16,
        }
    }
}
impl SchedulerConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported scheduler configuration version"
        );
        ensure!(
            (1..=3600).contains(&self.timeout_seconds),
            "invalid analysis deadline"
        );
        ensure!(
            (256..=131072).contains(&self.memory_mib),
            "invalid memory budget"
        );
        ensure!(
            (1..=1_048_576).contains(&self.disk_quota_mib),
            "invalid disk quota"
        );
        ensure!(
            (1..=32).contains(&self.max_queued),
            "queue limit must be 1..32"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    jobs: Vec<JobRecord>,
}

struct Inner {
    root: PathBuf,
    jobs: Mutex<Vec<JobRecord>>,
    stop: AtomicBool,
    _lease: File,
}
struct Shutdown {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for Shutdown {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::Release);
        if let Ok(thread) = self.thread.get_mut()
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
    }
}
#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Inner>,
    config: SchedulerConfig,
    _shutdown: Arc<Shutdown>,
}

impl Scheduler {
    pub fn start(store: &Path, executable: &Path, config: SchedulerConfig) -> Result<Self> {
        config.validate()?;
        let store = store.canonicalize()?;
        let executable = executable.canonicalize()?;
        let root = store.join("jobs");
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&root)?;
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(root.join("fences"))?;
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(root.join("scheduler.lock"))?;
        lease
            .try_lock()
            .context("another scheduler owns this store")?;
        let path = root.join("journal.json");
        let mut jobs = if path.exists() {
            ensure!(
                fs::metadata(&path)?.len() <= 8 * 1024 * 1024,
                "job journal too large"
            );
            let journal: Journal = serde_json::from_reader(File::open(path)?)?;
            ensure!(
                journal.version == 1 && journal.jobs.len() <= MAX_HISTORY,
                "incompatible job journal"
            );
            journal.jobs
        } else {
            vec![]
        };
        let mut ids = std::collections::BTreeSet::new();
        for job in &mut jobs {
            validate_request(&job.request)?;
            ensure!(
                job.id.starts_with("job:") && job.id.len() <= 160 && ids.insert(job.id.clone()),
                "invalid or duplicate job identity"
            );
            ensure!(
                job.created_ms.parse::<u64>().is_ok()
                    && job
                        .message
                        .as_ref()
                        .is_none_or(|message| message.len() <= 4096),
                "invalid job metadata"
            );
            ensure!(
                job.events.len() <= MAX_EVENTS
                    && job.events.iter().all(|event| event.sequence > 0
                        && event.sequence < 1_000_000
                        && event.timestamp_ms.parse::<u64>().is_ok())
                    && job
                        .events
                        .windows(2)
                        .all(|events| events[0].sequence < events[1].sequence),
                "invalid event history"
            );
            if !job.status.terminal() {
                job.status = JobStatus::Failed;
                job.message = Some("Service restarted before job completion; inspect the published head before retrying.".into());
                event(job, JobStage::Finished);
            }
        }
        let inner = Arc::new(Inner {
            root,
            jobs: Mutex::new(jobs),
            stop: AtomicBool::new(false),
            _lease: lease,
        });
        persist(
            &inner,
            &inner
                .jobs
                .lock()
                .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?,
        )?;
        let worker_inner = inner.clone();
        let worker_config = config.clone();
        let thread = std::thread::Builder::new()
            .name("atlas-scheduler".into())
            .spawn(move || {
                while !worker_inner.stop.load(Ordering::Acquire) {
                    if let Err(_error) =
                        next_job(&worker_inner, &store, &executable, &worker_config)
                    {
                        worker_inner.stop.store(true, Ordering::Release);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            })?;
        let shutdown = Arc::new(Shutdown {
            inner: inner.clone(),
            thread: Mutex::new(Some(thread)),
        });
        Ok(Self {
            inner,
            config,
            _shutdown: shutdown,
        })
    }

    pub fn submit(&self, request: JobRequest) -> Result<JobRecord> {
        validate_request(&request)?;
        ensure!(
            !self.inner.stop.load(Ordering::Acquire),
            "scheduler unavailable"
        );
        let _fence = fence_lock(&self.inner.root)?;
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?;
        if let Some(job) = jobs.iter().find(|j| {
            j.request == request && !j.status.terminal() && j.status != JobStatus::Cancelling
        }) {
            return Ok(job.clone());
        }
        ensure!(
            jobs.iter()
                .filter(|j| j.status == JobStatus::Queued && j.request.profile != request.profile)
                .count()
                < self.config.max_queued,
            "analysis queue is full"
        );
        let mut updated = jobs.clone();
        for job in &mut updated {
            if job.request.profile == request.profile && !job.status.terminal() {
                job.status = if job.status == JobStatus::Queued {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Cancelling
                };
                job.message = Some("Superseded by a newer request for this profile.".into());
                event(
                    job,
                    if job.status == JobStatus::Cancelled {
                        JobStage::Finished
                    } else {
                        JobStage::Queued
                    },
                );
            }
        }
        while updated.len() >= MAX_HISTORY {
            let index = updated
                .iter()
                .position(|job| job.status.terminal())
                .context("analysis history is full")?;
            updated.remove(index);
        }
        let mut entropy = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut entropy)?;
        let mut job = JobRecord {
            id: format!(
                "job:{}",
                entropy
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            ),
            request,
            status: JobStatus::Queued,
            created_ms: timestamp(),
            snapshot_id: None,
            message: None,
            events: vec![],
        };
        event(&mut job, JobStage::Queued);
        updated.push(job.clone());
        write_fence(&self.inner.root, &job.request.profile, &job.id)?;
        persist(&self.inner, &updated)?;
        *jobs = updated;
        Ok(job)
    }

    pub fn list(&self) -> Result<Vec<JobRecord>> {
        Ok(self
            .inner
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?
            .clone())
    }
    pub fn get(&self, id: &str) -> Result<JobRecord> {
        self.inner
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?
            .iter()
            .find(|job| job.id == id)
            .cloned()
            .context("unknown job")
    }
    pub fn cancel(&self, id: &str) -> Result<JobRecord> {
        let existing = self.get(id)?;
        if existing.status.terminal() || existing.status == JobStatus::Cancelling {
            return Ok(existing);
        }
        let _fence = fence_lock(&self.inner.root)?;
        let mut jobs = self
            .inner
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?;
        let mut updated = jobs.clone();
        let job = updated
            .iter_mut()
            .find(|job| job.id == id)
            .context("unknown job")?;
        if job.status.terminal() || job.status == JobStatus::Cancelling {
            return Ok(job.clone());
        }
        write_fence(&self.inner.root, &job.request.profile, "cancelled")?;
        job.status = if job.status == JobStatus::Queued {
            JobStatus::Cancelled
        } else {
            JobStatus::Cancelling
        };
        event(
            job,
            if job.status == JobStatus::Cancelled {
                JobStage::Finished
            } else {
                JobStage::Queued
            },
        );
        let result = job.clone();
        persist(&self.inner, &updated)?;
        *jobs = updated;
        Ok(result)
    }
}

fn fence_lock(root: &Path) -> Result<File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join("publication.lock"))?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock)
                if start.elapsed() < Duration::from_millis(100) =>
            {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(anyhow::anyhow!("publication fence unavailable: {error}")),
        }
    }
    Ok(file)
}
fn write_fence(root: &Path, profile: &str, id: &str) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(root.join("fences"))?;
    serde_json::to_writer(&mut file, id)?;
    file.as_file().sync_all()?;
    file.persist(root.join("fences").join(format!("{profile}.json")))?;
    File::open(root.join("fences"))?.sync_all()?;
    Ok(())
}
/// Hold through the catalog commit. Supersession and publication share this lock.
pub fn publication_guard(store: &Path, profile: &str, id: &str) -> Result<File> {
    validate_request(&JobRequest {
        profile: profile.into(),
        level: JobLevel::Syntax,
        priority: JobPriority::Workspace,
        context: None,
    })?;
    let root = store.join("jobs");
    let lease = fence_lock(&root)?;
    let file = File::open(root.join("fences").join(format!("{profile}.json")))?;
    ensure!(file.metadata()?.len() <= 128, "invalid job fence");
    let current: String = serde_json::from_reader(file)?;
    ensure!(
        current == id,
        "analysis generation was superseded or cancelled"
    );
    Ok(lease)
}

pub fn validate_request(request: &JobRequest) -> Result<()> {
    ensure!(
        !request.profile.is_empty()
            && request.profile.len() <= 64
            && request
                .profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)),
        "invalid profile"
    );
    ensure!(
        serde_json::to_vec(request)?.len() <= 16 * 1024,
        "analysis request too large"
    );
    if let Some(context) = &request.context {
        ensure!(
            context.trust == "read_only" && context.name == request.profile,
            "context must match profile and read-only policy"
        );
    }
    Ok(())
}

fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}
fn event(job: &mut JobRecord, stage: JobStage) {
    if job.events.len() == MAX_EVENTS {
        job.events.remove(0);
    }
    let sequence = job
        .events
        .last()
        .map_or(1, |e| e.sequence.saturating_add(1));
    job.events.push(JobEvent {
        sequence,
        timestamp_ms: timestamp(),
        stage,
        status: job.status,
    });
}
fn persist(inner: &Inner, jobs: &[JobRecord]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(&inner.root)?;
    serde_json::to_writer(
        &mut file,
        &Journal {
            version: 1,
            jobs: jobs.to_vec(),
        },
    )?;
    file.as_file().sync_all()?;
    file.persist(inner.root.join("journal.json"))?;
    File::open(&inner.root)?.sync_all()?;
    Ok(())
}
fn update(inner: &Inner, id: &str, change: impl FnOnce(&mut JobRecord)) -> Result<JobRecord> {
    let mut jobs = inner
        .jobs
        .lock()
        .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?;
    let mut updated = jobs.clone();
    let job = updated
        .iter_mut()
        .find(|job| job.id == id)
        .context("unknown job")?;
    change(job);
    let result = job.clone();
    persist(inner, &updated)?;
    *jobs = updated;
    Ok(result)
}
fn cancelled(inner: &Inner, id: &str) -> bool {
    inner.stop.load(Ordering::Acquire)
        || inner.jobs.lock().map_or(true, |jobs| {
            jobs.iter()
                .find(|job| job.id == id)
                .is_none_or(|job| job.status == JobStatus::Cancelling)
        })
}
fn next_job(
    inner: &Inner,
    store: &Path,
    executable: &Path,
    config: &SchedulerConfig,
) -> Result<()> {
    let job = {
        let jobs = inner
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("scheduler unavailable"))?;
        jobs.iter()
            .enumerate()
            .filter(|(_, job)| job.status == JobStatus::Queued)
            .min_by_key(|(index, job)| (job.request.priority, *index))
            .map(|(_, job)| job.clone())
    };
    let Some(job) = job else { return Ok(()) };
    let started = update(inner, &job.id, |j| {
        if j.status == JobStatus::Queued {
            j.status = JobStatus::Running;
            event(j, JobStage::Capture);
        }
    })?;
    if started.status != JobStatus::Running {
        return Ok(());
    }
    let result = execute(inner, store, executable, config, &job);
    update(inner, &job.id, |j| {
        match result {
            Ok(snapshot) => {
                j.status = JobStatus::Succeeded;
                j.snapshot_id = Some(snapshot.id);
                j.message = None;
            }
            Err(_) if cancelled_state(j.status) || inner.stop.load(Ordering::Acquire) => {
                j.status = JobStatus::Cancelled;
            }
            Err(_) => {
                j.status = JobStatus::Failed;
                j.message = Some("Analysis failed or exceeded its budget; previously published snapshots remain available.".into());
            }
        }
        event(j, JobStage::Finished);
    })?;
    Ok(())
}
fn cancelled_state(status: JobStatus) -> bool {
    status == JobStatus::Cancelling
}

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            if let Some(pid) = rustix::process::Pid::from_raw(self.0.id() as i32) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn execute(
    inner: &Inner,
    store: &Path,
    executable: &Path,
    config: &SchedulerConfig,
    job: &JobRecord,
) -> Result<Snapshot> {
    let scratch = tempfile::tempdir_in(&inner.root)?;
    let output = tempfile::tempfile()?;
    let progress = scratch.path().join("progress.json");
    let mut command = Command::new(executable);
    command
        .arg("--store")
        .arg(store)
        .arg("index")
        .arg("--profile")
        .arg(&job.request.profile)
        .arg("--level")
        .arg(match job.request.level {
            JobLevel::Syntax => "syntax",
            JobLevel::Semantic => "semantic",
        })
        .arg("--timeout")
        .arg(config.timeout_seconds.to_string())
        .arg("--memory-mib")
        .arg(config.memory_mib.to_string())
        .arg("--disk-quota-mib")
        .arg(config.disk_quota_mib.to_string())
        .arg("--progress-file")
        .arg(&progress)
        .arg("--supervisor-pid")
        .arg(std::process::id().to_string())
        .arg("--job-id")
        .arg(&job.id)
        .env_clear()
        .current_dir(store)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::null())
        .process_group(0);
    if let Some(context) = &job.request.context {
        let path = scratch.path().join("context.json");
        serde_json::to_writer(File::create(&path)?, context)?;
        command.arg("--context").arg(path);
    }
    let mut child = ChildGuard(command.spawn()?);
    let started = Instant::now();
    let mut last_stage = JobStage::Capture;
    loop {
        if let Some(status) = child.0.try_wait()? {
            ensure!(status.success(), "analysis failed");
            ensure!(
                output.metadata()?.len() <= 2 * 1024 * 1024,
                "analysis response too large"
            );
            use std::io::{Seek, SeekFrom};
            let mut output = output;
            output.seek(SeekFrom::Start(0))?;
            return Ok(serde_json::from_reader(output)?);
        }
        if cancelled(inner, &job.id) {
            bail!("analysis cancelled")
        }
        ensure!(
            started.elapsed() <= Duration::from_secs(config.timeout_seconds + 10),
            "analysis timeout"
        );
        ensure!(
            output.metadata()?.len() <= 2 * 1024 * 1024,
            "analysis response too large"
        );
        if let Ok(file) = File::open(&progress)
            && file.metadata()?.len() <= 128
            && let Ok(stage) = serde_json::from_reader::<_, JobStage>(file)
            && stage != last_stage
        {
            update(inner, &job.id, |job| event(job, stage))?;
            last_stage = stage;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(test)]
mod tests;
