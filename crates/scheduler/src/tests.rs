use super::*;
use std::os::unix::fs::PermissionsExt;

fn request(profile: &str) -> JobRequest {
    JobRequest {
        profile: profile.into(),
        level: JobLevel::Syntax,
        priority: JobPriority::Workspace,
        context: None,
    }
}
fn executable(temp: &tempfile::TempDir, script: &str) -> PathBuf {
    let path = temp.path().join("worker");
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
fn wait(scheduler: &Scheduler, id: &str, predicate: impl Fn(JobStatus) -> bool) -> JobRecord {
    let start = Instant::now();
    loop {
        let job = scheduler.get(id).unwrap();
        if predicate(job.status) {
            return job;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "job did not change: {job:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn request_policy_rejects_execution_and_paths() {
    assert!(validate_request(&request("../secret")).is_err());
    assert!(validate_request(&request("")).is_err());
    assert!(serde_json::from_str::<JobRequest>(r#"{"profile":"a","command":"/bin/sh"}"#).is_err());
    assert!(serde_json::from_str::<JobRequest>(r#"{"profile":"a","level":"compiler"}"#).is_err());
    assert!(
        SchedulerConfig {
            max_queued: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn duplicate_submission_coalesces_and_cancellation_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(&temp, "exec /bin/sleep 30");
    let scheduler = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    let job = scheduler.submit(request("default")).unwrap();
    assert_eq!(scheduler.submit(request("default")).unwrap().id, job.id);
    wait(&scheduler, &job.id, |s| s == JobStatus::Running);
    let now = Instant::now();
    scheduler.cancel(&job.id).unwrap();
    wait(&scheduler, &job.id, JobStatus::terminal);
    assert!(now.elapsed() < Duration::from_millis(500));
    assert_eq!(
        scheduler.cancel(&job.id).unwrap().status,
        JobStatus::Cancelled
    );
    assert!(scheduler.cancel("job:missing").is_err());
}

#[test]
fn queue_admission_and_priority_are_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(&temp, "exec /bin/sleep 30");
    let scheduler = Scheduler::start(
        temp.path(),
        &exe,
        SchedulerConfig {
            max_queued: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let first = scheduler.submit(request("active")).unwrap();
    wait(&scheduler, &first.id, |s| s == JobStatus::Running);
    let background = scheduler
        .submit(JobRequest {
            priority: JobPriority::Background,
            ..request("background")
        })
        .unwrap();
    let foreground = scheduler
        .submit(JobRequest {
            priority: JobPriority::Foreground,
            ..request("foreground")
        })
        .unwrap();
    assert!(scheduler.submit(request("overflow")).is_err());
    scheduler.cancel(&first.id).unwrap();
    wait(&scheduler, &foreground.id, |s| s == JobStatus::Running);
    assert_eq!(
        scheduler.get(&background.id).unwrap().status,
        JobStatus::Queued
    );
}

#[test]
fn newer_profile_request_cancels_old_work_before_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(&temp, "exec /bin/sleep 30");
    let scheduler = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    let old = scheduler.submit(request("default")).unwrap();
    wait(&scheduler, &old.id, |s| s == JobStatus::Running);
    drop(publication_guard(temp.path(), "default", &old.id).unwrap());
    let new = scheduler
        .submit(JobRequest {
            level: JobLevel::Semantic,
            ..request("default")
        })
        .unwrap();
    assert!(publication_guard(temp.path(), "default", &old.id).is_err());
    drop(publication_guard(temp.path(), "default", &new.id).unwrap());
    wait(&scheduler, &new.id, |s| s == JobStatus::Running);
    assert_eq!(scheduler.get(&old.id).unwrap().status, JobStatus::Cancelled);
}

#[test]
fn process_failure_is_retained_and_second_scheduler_cannot_start() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(&temp, "exit 23");
    let scheduler = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    assert!(Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).is_err());
    let id = scheduler.submit(request("default")).unwrap().id;
    assert_eq!(
        wait(&scheduler, &id, JobStatus::terminal).status,
        JobStatus::Failed
    );
    drop(scheduler);
    let restarted = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    assert_eq!(restarted.get(&id).unwrap().status, JobStatus::Failed);
}

#[test]
fn interrupted_journal_is_not_silently_replayed() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(&temp, "exit 23");
    fs::create_dir(temp.path().join("jobs")).unwrap();
    let record = JobRecord {
        id: "job:interrupted".into(),
        request: request("default"),
        status: JobStatus::Running,
        created_ms: "1".into(),
        snapshot_id: None,
        message: None,
        events: vec![],
    };
    serde_json::to_writer(
        File::create(temp.path().join("jobs/journal.json")).unwrap(),
        &Journal {
            version: 1,
            jobs: vec![record],
        },
    )
    .unwrap();
    let scheduler = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    let job = scheduler.get("job:interrupted").unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert!(job.message.unwrap().contains("restarted"));
}

#[test]
fn contended_publication_never_holds_job_mutex_or_waits_unbounded() {
    let temp = tempfile::tempdir().unwrap();
    let exe = executable(&temp, "exec /bin/sleep 30");
    let scheduler = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    let job = scheduler.submit(request("default")).unwrap();
    wait(&scheduler, &job.id, |s| s == JobStatus::Running);
    let fence = publication_guard(temp.path(), "default", &job.id).unwrap();
    let other = scheduler.clone();
    let thread = std::thread::spawn(move || other.submit(request("another")));
    std::thread::sleep(Duration::from_millis(20));
    let start = Instant::now();
    scheduler.get(&job.id).unwrap();
    assert!(start.elapsed() < Duration::from_millis(100));
    assert!(thread.join().unwrap().is_err());
    assert!(scheduler.cancel(&job.id).is_err());
    drop(fence);
    scheduler.cancel(&job.id).unwrap();
    wait(&scheduler, &job.id, JobStatus::terminal);
}

#[test]
fn malformed_journals_reject_duplicate_identities_and_unreplayable_events() {
    let record = JobRecord {
        id: "job:fixture".into(),
        request: request("default"),
        status: JobStatus::Failed,
        created_ms: "1".into(),
        snapshot_id: None,
        message: None,
        events: vec![JobEvent {
            sequence: 1,
            timestamp_ms: "1".into(),
            stage: JobStage::Finished,
            status: JobStatus::Failed,
        }],
    };
    let mut cases = vec![vec![record.clone(), record.clone()]];
    let mut overflow = record.clone();
    overflow.events[0].sequence = u32::MAX;
    cases.push(vec![overflow]);
    let mut duplicate_sequence = record.clone();
    duplicate_sequence
        .events
        .push(duplicate_sequence.events[0].clone());
    cases.push(vec![duplicate_sequence]);
    let mut malformed_time = record.clone();
    malformed_time.created_ms = "not-an-integer".into();
    cases.push(vec![malformed_time]);
    let mut oversized = record;
    oversized.message = Some("x".repeat(4097));
    cases.push(vec![oversized]);
    for jobs in cases {
        let temp = tempfile::tempdir().unwrap();
        let exe = executable(&temp, "exit 23");
        fs::create_dir(temp.path().join("jobs")).unwrap();
        let path = temp.path().join("jobs/journal.json");
        let bytes = serde_json::to_vec(&Journal { version: 1, jobs }).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn dropping_last_handle_kills_process_group_and_keeps_history_private() {
    let temp = tempfile::tempdir().unwrap();
    let pid = temp.path().join("pid");
    let exe = executable(
        &temp,
        &format!("echo $$ > '{}'\n/bin/sleep 30", pid.display()),
    );
    let scheduler = Scheduler::start(temp.path(), &exe, SchedulerConfig::default()).unwrap();
    let id = scheduler.submit(request("default")).unwrap().id;
    wait(&scheduler, &id, |s| s == JobStatus::Running);
    let start = Instant::now();
    while !pid.exists() {
        assert!(start.elapsed() < Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(10));
    }
    let process = fs::read_to_string(pid).unwrap();
    drop(scheduler);
    assert!(!Path::new(&format!("/proc/{}", process.trim())).exists());
    assert_eq!(
        fs::metadata(temp.path().join("jobs/journal.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
