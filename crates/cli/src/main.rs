use anyhow::{Context, Result, bail, ensure};
use atlas_frontend::AnalysisLevel;
use atlas_ingest::CaptureOptions;
use atlas_model::*;
use atlas_query::QueryEngine;
use atlas_store::Store;
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    name = "atlas",
    version,
    about = "Source-linked Rust code maps and evidence-aware change review"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".atlas")]
    store: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Register a workspace without executing any project commands.
    Init {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long, default_value = "read-only")]
        trust: String,
    },
    /// Capture source, analyze it in a bounded worker, and atomically publish it.
    Index {
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long, value_enum, default_value_t = Level::Semantic)]
        level: Level,
        #[arg(long, default_value = "unknown")]
        target: String,
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        #[arg(long)]
        no_default_features: bool,
        #[arg(long = "cfg")]
        cfg_values: Vec<String>,
        #[arg(long)]
        context: Option<PathBuf>,
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        #[arg(long, default_value_t = 8192)]
        memory_mib: u64,
        #[arg(long, default_value_t = 4096)]
        disk_quota_mib: u64,
        #[arg(long, hide = true)]
        progress_file: Option<PathBuf>,
        #[arg(long, hide = true)]
        supervisor_pid: Option<u32>,
        #[arg(long, hide = true)]
        job_id: Option<String>,
    },
    /// Serve the browser and authenticated read-only API on loopback.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7878")]
        listen: SocketAddr,
        #[arg(long, default_value = "web/dist")]
        web_dir: PathBuf,
        #[arg(long)]
        token_file: Option<PathBuf>,
        #[arg(long, default_value_t = 8)]
        query_slots: usize,
        /// Enable the bounded index queue for the registered workspace.
        #[arg(long)]
        enable_jobs: bool,
        /// Versioned resource policy for the local analysis queue.
        #[arg(long)]
        scheduler_config: Option<PathBuf>,
    },
    /// List immutable snapshots.
    Snapshots,
    /// Search symbols or query callers/callees as JSON.
    Query {
        #[arg(value_enum)]
        kind: QueryKind,
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        symbol: Option<String>,
        #[arg(long, default_value = "")]
        text: String,
        #[arg(long, default_value_t = 2)]
        depth: u32,
    },
    /// Compare definition signatures, bodies and configuration across snapshots.
    Diff {
        #[arg(long)]
        before: String,
        #[arg(long)]
        after: String,
    },
    /// Validate persisted object references and checksums without running the workspace.
    Doctor,
    /// Plan retention or execute an exact previously reviewed GC plan.
    Gc {
        #[arg(long, conflicts_with = "execute", required_unless_present = "execute")]
        dry_run: bool,
        #[arg(long, requires = "plan")]
        execute: bool,
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long, default_value_t = 3)]
        keep_recent: usize,
        #[arg(long, default_value_t = 86400)]
        grace_seconds: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Retain an immutable snapshot under a named pin.
    Pin {
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        name: String,
    },
    /// Remove a named retention pin, not the snapshot itself.
    Unpin {
        #[arg(long)]
        name: String,
    },
    /// List retention pins.
    Pins,
    /// Write an evidence export for one pinned snapshot.
    Export {
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Import artifact-matched test outcomes and trace streams without execution.
    ImportEvidence {
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        artifact: PathBuf,
    },
    /// Record at least 30 raw local search latency samples; no scale claim.
    Benchmark {
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long, default_value_t = 30)]
        samples: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    #[command(hide = true)]
    Worker {
        #[arg(long)]
        job: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Level {
    Syntax,
    Semantic,
}
#[derive(Clone, Copy, ValueEnum)]
enum QueryKind {
    Search,
    Callers,
    Callees,
    Both,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceConfig {
    version: u32,
    workspace: PathBuf,
    trust: String,
}

#[derive(Serialize, Deserialize)]
struct IndexJob {
    workspace: PathBuf,
    profile: String,
    target: String,
    features: Vec<String>,
    default_features: bool,
    cfg: BTreeMap<String, Option<String>>,
    explicit_context: Option<BuildContext>,
    level: Level,
    memory_mib: u64,
    timeout: u64,
    parent_pid: u32,
    #[serde(default)]
    progress_file: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Commands::Worker { job, output } = &cli.command {
        return worker(job, output);
    }
    if let Commands::Index {
        supervisor_pid: Some(parent),
        ..
    } = &cli.command
    {
        rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
        ensure!(
            rustix::process::getppid()
                .is_some_and(|pid| pid.as_raw_nonzero().get() as u32 == *parent),
            "analysis supervisor exited"
        );
    }
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init { workspace, trust } => {
            ensure!(
                trust == "read-only",
                "only --trust read-only is supported; executable analysis is unavailable"
            );
            let workspace = workspace
                .canonicalize()
                .context("workspace does not exist")?;
            ensure!(workspace.is_dir(), "workspace must be a directory");
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&cli.store)?;
            let config_path = cli.store.join("config.json");
            ensure!(!config_path.exists(), "store is already initialized");
            let config = WorkspaceConfig {
                version: 1,
                workspace,
                trust,
            };
            private_json(&config_path, &config)?;
            Store::open(&cli.store)?;
            print_json(&config)?;
        }
        Commands::Index {
            workspace,
            profile,
            level,
            target,
            features,
            no_default_features,
            cfg_values,
            context,
            timeout,
            memory_mib,
            disk_quota_mib,
            progress_file,
            supervisor_pid: _,
            job_id,
        } => {
            ensure!(
                (1..=3600).contains(&timeout),
                "timeout must be between 1 and 3600 seconds"
            );
            ensure!(
                (256..=131072).contains(&memory_mib),
                "memory budget must be between 256 and 131072 MiB"
            );
            let workspace = match workspace {
                Some(path) => path.canonicalize()?,
                None => read_config(&cli.store)?.workspace,
            };
            let explicit_context = context
                .map(|p| -> Result<BuildContext> {
                    ensure!(
                        fs::metadata(&p)?.len() <= 1024 * 1024,
                        "context manifest exceeds 1 MiB"
                    );
                    Ok(serde_json::from_reader(File::open(p)?)?)
                })
                .transpose()?;
            let mut cfg = BTreeMap::new();
            for value in cfg_values {
                let (key, value) = match value.split_once('=') {
                    Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                    None => (value, None),
                };
                ensure!(!key.is_empty(), "cfg keys cannot be empty");
                cfg.insert(key, value);
            }
            let job = IndexJob {
                workspace,
                profile: profile.clone(),
                target,
                features,
                default_features: !no_default_features,
                cfg,
                explicit_context,
                level,
                memory_mib,
                timeout,
                parent_pid: std::process::id(),
                progress_file: progress_file.clone(),
            };
            let store = Store::open(&cli.store)?;
            use std::os::unix::fs::OpenOptionsExt;
            let index_lease = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(cli.store.join("index.lock"))?;
            index_lease
                .try_lock()
                .context("another index job is active for this store")?;
            let quota = disk_quota_mib
                .checked_mul(1024 * 1024)
                .context("disk quota overflow")?;
            ensure!(
                quota > 0 && directory_bytes(&cli.store)? < quota.saturating_mul(80) / 100,
                "ingestion paused: store is at 80% of its disk quota"
            );
            let head = store.head(&profile)?;
            let start = Instant::now();
            let batch = run_worker(&cli.store, &job)?;
            progress(progress_file.as_deref(), JobStage::Validation)?;
            let bytes = serde_json::to_vec(&batch)?.len() as u64;
            ensure!(
                directory_bytes(&cli.store)?.saturating_add(bytes.saturating_mul(3))
                    < quota.saturating_mul(90) / 100,
                "publication paused: estimated output would exceed 90% of disk quota"
            );
            progress(progress_file.as_deref(), JobStage::Publication)?;
            let _fence = job_id
                .as_deref()
                .map(|id| atlas_scheduler::publication_guard(&cli.store, &profile, id))
                .transpose()?;
            let snapshot = store.publish(&batch, &profile, head.as_ref())?;
            eprintln!(
                "Published {} definitions and {} relations in {} ms ({:?} coverage)",
                snapshot.definition_count,
                snapshot.relation_count,
                start.elapsed().as_millis(),
                snapshot.coverage.status
            );
            print_json(&snapshot)?;
        }
        Commands::Serve {
            listen,
            web_dir,
            token_file,
            query_slots,
            enable_jobs,
            scheduler_config,
        } => {
            ensure!(
                listen.ip().is_loopback(),
                "only loopback binding is supported"
            );
            ensure!(
                web_dir.join("index.html").is_file(),
                "viewer assets not found; run npm ci && npm run build in web, then pass --web-dir web/dist"
            );
            let token = match token_file {
                Some(path) => {
                    ensure!(
                        fs::metadata(&path)?.len() <= 4096,
                        "token file is too large"
                    );
                    fs::read_to_string(path)?.trim().to_owned()
                }
                None => atlas_server::session_token()?,
            };
            let engine = QueryEngine::new(Store::open(&cli.store)?)?;
            ensure!(
                enable_jobs || scheduler_config.is_none(),
                "--scheduler-config requires --enable-jobs"
            );
            let scheduler = if enable_jobs {
                read_config(&cli.store)?;
                let policy = match scheduler_config {
                    Some(path) => {
                        ensure!(
                            fs::metadata(&path)?.len() <= 4096,
                            "scheduler configuration too large"
                        );
                        serde_json::from_reader(File::open(path)?)?
                    }
                    None => atlas_scheduler::SchedulerConfig::default(),
                };
                Some(atlas_scheduler::Scheduler::start(
                    &cli.store,
                    &std::env::current_exe()?,
                    policy,
                )?)
            } else {
                None
            };
            let config = atlas_server::ServerConfig {
                listen,
                token,
                web_dir,
                max_concurrent_queries: query_slots,
                observations_dir: cli.store.join("observations"),
                scheduler,
            };
            config.validate()?;
            eprintln!("Ferrum Atlas: http://{listen}/#token={}", config.token);
            eprintln!(
                "Session token stays in the URL fragment and is removed by the viewer. Ctrl-C stops the server."
            );
            atlas_server::serve(engine, config).await?;
        }
        Commands::Snapshots => print_json(&Store::open(&cli.store)?.snapshots()?)?,
        Commands::Query {
            kind,
            snapshot,
            profile,
            symbol,
            text,
            depth,
        } => {
            let store = Store::open(&cli.store)?;
            let id = selected_snapshot(&store, snapshot, &profile)?;
            let query = QueryEngine::new(store)?;
            let snapshot = query.snapshot(&id)?;
            if matches!(kind, QueryKind::Search) {
                print_json(&query.search(&id, &snapshot.context.id, &text, 50, None)?)?;
            } else {
                let symbol = DefinitionId(symbol.context("--symbol is required for a call query")?);
                let direction = match kind {
                    QueryKind::Callers => Direction::Incoming,
                    QueryKind::Callees => Direction::Outgoing,
                    _ => Direction::Both,
                };
                print_json(&query.neighborhood(&GraphRequest {
                    snapshot_id: id,
                    context_id: snapshot.context.id,
                    definition_id: symbol,
                    direction,
                    depth,
                    max_nodes: 200,
                    max_edges: 500,
                })?)?;
            }
        }
        Commands::Diff { before, after } => print_json(
            &QueryEngine::new(Store::open(&cli.store)?)?.diff(&DiffRequest {
                before: SnapshotId(before),
                after: SnapshotId(after),
            })?,
        )?,
        Commands::Doctor => {
            let store = Store::open(&cli.store)?;
            let report = store.integrity_check()?;
            print_json(&report)?;
            ensure!(report.valid, "store integrity check failed");
            eprintln!(
                "Read-only analysis; compiler MIR and executable worker sandbox unavailable. No workspace commands were run."
            );
        }
        Commands::Gc {
            dry_run: _,
            execute,
            plan,
            keep_recent,
            grace_seconds,
            output,
        } => {
            let store = Store::open(&cli.store)?;
            if execute {
                ensure!(
                    output.is_none(),
                    "--output is only valid when creating a plan"
                );
                let path = plan.context("--plan is required")?;
                ensure!(
                    fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
                    "GC plan exceeds 16 MiB"
                );
                let plan = serde_json::from_reader(File::open(path)?)?;
                print_json(&store.gc_execute(&plan)?)?;
            } else {
                ensure!(plan.is_none(), "--plan is only valid with --execute");
                let plan = store.gc_plan(&atlas_store::RetentionPolicy {
                    keep_recent,
                    grace_period: Duration::from_secs(grace_seconds),
                    additional_roots: Default::default(),
                })?;
                match output {
                    Some(path) => private_json(&path, &plan)?,
                    None => print_json(&plan)?,
                }
            }
        }
        Commands::Pin { snapshot, name } => {
            let store = Store::open(&cli.store)?;
            store.pin(&SnapshotId(snapshot), &name)?;
            print_json(&store.pins()?)?;
        }
        Commands::Unpin { name } => {
            print_json(&serde_json::json!({"removed":Store::open(&cli.store)?.unpin(&name)?}))?
        }
        Commands::Pins => print_json(&Store::open(&cli.store)?.pins()?)?,
        Commands::ImportEvidence {
            snapshot,
            bundle,
            artifact,
        } => {
            ensure!(
                fs::metadata(&bundle)?.len() <= 2 * 1024 * 1024,
                "evidence bundle exceeds 2 MiB"
            );
            let bundle: ObservationBundle = serde_json::from_reader(File::open(bundle)?)?;
            let store = Store::open(&cli.store)?;
            let id = SnapshotId(snapshot);
            let metadata = store.snapshot(&id)?;
            let reader = store.reader(&id)?;
            let definitions = reader.definitions(100_001)?;
            ensure!(
                definitions.len() <= 100_000,
                "evidence import mapping exceeds 100,000 definitions"
            );
            let ids = definitions.into_iter().map(|d| d.id).collect();
            let imported = atlas_evidence::import(
                &cli.store.join("observations"),
                &bundle,
                &metadata,
                &ids,
                &artifact,
            )?;
            drop(reader);
            print_json(&imported)?;
        }
        Commands::Export { snapshot, output } => {
            ensure!(!output.exists(), "export destination already exists");
            let query = QueryEngine::new(Store::open(&cli.store)?)?;
            let id = SnapshotId(snapshot);
            let metadata = query.snapshot(&id)?;
            let mut definitions = Vec::new();
            let mut cursor = None;
            loop {
                let page = query.search(&id, &metadata.context.id, "", 50, cursor.as_deref())?;
                definitions.extend(page.items);
                cursor = page.page.next_cursor;
                if cursor.is_none() {
                    break;
                }
                ensure!(
                    definitions.len() < 100_000,
                    "export exceeds 100,000 definitions"
                );
            }
            let export = serde_json::json!({"format":"ferrum-atlas-evidence", "version":1, "snapshot":metadata, "definitions":definitions, "limitations":["Symbol manifest only: source bytes, call relations, traces and active queries are not included."]});
            private_json(&output, &export)?;
            eprintln!(
                "Exported {} pinned definitions to {}",
                definitions.len(),
                output.display()
            );
        }
        Commands::Benchmark {
            snapshot,
            profile,
            samples,
            output,
        } => {
            ensure!(
                (30..=100_000).contains(&samples),
                "use 30 to 100000 samples"
            );
            let store = Store::open(&cli.store)?;
            let id = selected_snapshot(&store, snapshot, &profile)?;
            let query = QueryEngine::new(store)?;
            let metadata = query.snapshot(&id)?;
            let mut times = Vec::with_capacity(samples);
            for _ in 0..samples {
                let start = Instant::now();
                query.search(&id, &metadata.context.id, "", 50, None)?;
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            let mut sorted = times.clone();
            sorted.sort_by(f64::total_cmp);
            let report = serde_json::json!({"version":1, "workload":"local symbol search, top 50", "cache":"uncontrolled local cache; first sample retained", "snapshot":metadata, "samples_ms":times, "p50_ms":sorted[(samples-1)/2], "p95_ms":sorted[((samples as f64 * 0.95).ceil() as usize - 1).min(samples-1)], "hardware":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"logical_cpus":std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1)}, "limitations":["Smoke measurement only; no L/S/O qualification, cold-cache claim, or p99 inference."]});
            match output {
                Some(path) => private_json(&path, &report)?,
                None => print_json(&report)?,
            }
        }
        Commands::Worker { .. } => unreachable!(),
    }
    Ok(())
}

fn read_config(store: &Path) -> Result<WorkspaceConfig> {
    let path = store.join("config.json");
    ensure!(
        fs::metadata(&path)
            .context("run atlas init --workspace <path> first, or provide --workspace")?
            .len()
            <= 1024 * 1024,
        "workspace config too large"
    );
    let config: WorkspaceConfig = serde_json::from_reader(File::open(path)?)?;
    ensure!(
        config.version == 1 && config.trust == "read-only",
        "unsupported workspace configuration"
    );
    Ok(config)
}

fn selected_snapshot(store: &Store, snapshot: Option<String>, profile: &str) -> Result<SnapshotId> {
    match snapshot {
        Some(id) => Ok(SnapshotId(id)),
        None => store
            .head(profile)?
            .context("no published snapshot; run atlas index first"),
    }
}

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn run_worker(store: &Path, job: &IndexJob) -> Result<FactBatch> {
    let scratch = tempfile::Builder::new()
        .prefix("atlas-job-")
        .tempdir_in(store)?;
    let job_path = scratch.path().join("job.json");
    let output = scratch.path().join("facts.json");
    private_json(&job_path, job)?;
    let child = Command::new(std::env::current_exe()?)
        .arg("worker")
        .arg("--job")
        .arg(job_path.canonicalize()?)
        .arg("--output")
        .arg(scratch.path().canonicalize()?.join("facts.json"))
        .env_clear()
        .current_dir(scratch.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start analysis worker")?;
    let mut child = ChildGuard(child);
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.success(),
                "analysis worker failed ({status}); published snapshots remain available"
            );
            break;
        }
        if start.elapsed() > Duration::from_secs(job.timeout) {
            bail!("analysis deadline exceeded; published snapshots remain available");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    ensure!(
        fs::metadata(&output)?.len() <= 256 * 1024 * 1024,
        "worker output exceeds 256 MiB"
    );
    Ok(serde_json::from_reader(File::open(output)?)?)
}

fn worker(job_path: &Path, output: &Path) -> Result<()> {
    ensure!(
        fs::metadata(job_path)?.len() <= 1024 * 1024,
        "worker job too large"
    );
    let job: IndexJob = serde_json::from_reader(File::open(job_path)?)?;
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
    setrlimit(
        Resource::Cpu,
        Rlimit {
            current: Some(job.timeout),
            maximum: Some(job.timeout.saturating_add(1)),
        },
    )?;
    setrlimit(
        Resource::Fsize,
        Rlimit {
            current: Some(256 * 1024 * 1024),
            maximum: Some(256 * 1024 * 1024),
        },
    )?;
    let options = CaptureOptions {
        profile: job.profile,
        target: job.target,
        features: job.features,
        default_features: job.default_features,
        cfg: job.cfg,
        explicit_context: job.explicit_context,
        ..Default::default()
    };
    progress(job.progress_file.as_deref(), JobStage::Capture)?;
    let (source, context) = atlas_ingest::capture(&job.workspace, &options)?;
    let level = match job.level {
        Level::Syntax => AnalysisLevel::Syntax,
        Level::Semantic => AnalysisLevel::Semantic,
    };
    progress(
        job.progress_file.as_deref(),
        match job.level {
            Level::Syntax => JobStage::Syntax,
            Level::Semantic => JobStage::Semantics,
        },
    )?;
    let facts = atlas_frontend::analyze(source, context, level)?;
    private_json(output, &facts)
}

fn progress(path: Option<&Path>, stage: JobStage) -> Result<()> {
    if let Some(path) = path {
        let parent = path.parent().context("progress path has no parent")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer(&mut file, &stage)?;
        file.persist(path)?;
    }
    Ok(())
}

fn private_json(path: &Path, value: &impl Serialize) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.sync_all()?;
    Ok(())
}

fn directory_bytes(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut queue = vec![path.to_owned()];
    let mut entries = 0_usize;
    while let Some(path) = queue.pop() {
        for entry in fs::read_dir(path)? {
            entries += 1;
            ensure!(entries <= 1_000_000, "store inventory limit exceeded");
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
            if metadata.is_dir() {
                queue.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

fn print_json(value: &impl Serialize) -> Result<()> {
    serde_json::to_writer_pretty(std::io::stdout(), value)?;
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_worker_guard_terminates_and_reaps_the_child() {
        let child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        drop(ChildGuard(child));
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    }

    #[test]
    fn worker_rejects_an_exited_parent_before_analysis() {
        let temp = tempfile::tempdir().unwrap();
        let job = IndexJob {
            workspace: temp.path().into(),
            profile: "test".into(),
            target: "unknown".into(),
            features: vec![],
            default_features: false,
            cfg: Default::default(),
            explicit_context: None,
            level: Level::Syntax,
            memory_mib: 256,
            timeout: 1,
            parent_pid: u32::MAX,
            progress_file: None,
        };
        let job_path = temp.path().join("job.json");
        private_json(&job_path, &job).unwrap();
        assert!(
            worker(&job_path, &temp.path().join("output.json"))
                .unwrap_err()
                .to_string()
                .contains("parent exited")
        );
        rustix::process::set_parent_process_death_signal(None).unwrap();
    }
}
