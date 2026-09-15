#![cfg(target_os = "linux")]

use rustix::process::{Pid, Signal, kill_process};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ProcessGuard(Child);
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct WorkerGuard(u32);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !terminated(self.0) {
            let _ = kill_process(Pid::from_raw(self.0 as i32).unwrap(), Signal::KILL);
        }
    }
}

fn state(pid: u32) -> Option<char> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(") ")?
        .1
        .chars()
        .next()
}

fn terminated(pid: u32) -> bool {
    matches!(state(pid), None | Some('Z' | 'X'))
}

fn wait_until(condition: impl Fn() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(Duration::from_millis(1));
    }
}

fn job(workspace: &Path, parent_pid: u32, memory_mib: u64) -> serde_json::Value {
    json!({
        "workspace": workspace,
        "profile": "cancellation-test",
        "target": "unknown",
        "features": [],
        "default_features": true,
        "cfg": {},
        "explicit_context": null,
        "level": "syntax",
        "memory_mib": memory_mib,
        "timeout": 30,
        "parent_pid": parent_pid,
    })
}

fn worker_command(job: &Path, output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_atlas"));
    command
        .arg("worker")
        .arg("--job")
        .arg(job)
        .arg("--output")
        .arg(output);
    command.stdin(Stdio::null()).stdout(Stdio::null());
    command
}

// Re-executed as a controlled worker parent by the cancellation test below.
#[test]
fn worker_cancellation_driver() {
    let Some(root) = std::env::var_os("ATLAS_CANCELLATION_DRIVER_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    rustix::process::set_parent_process_death_signal(Some(Signal::KILL)).unwrap();
    let expected_parent: u32 = std::env::var("ATLAS_CANCELLATION_TEST_PARENT")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        rustix::process::getppid().unwrap().as_raw_nonzero().get() as u32,
        expected_parent
    );
    let job_path = root.join("job.json");
    fs::write(
        &job_path,
        serde_json::to_vec(&job(&root.join("source"), std::process::id(), 1024)).unwrap(),
    )
    .unwrap();
    let mut child = ProcessGuard(
        worker_command(&job_path, &root.join("facts.json"))
            .spawn()
            .unwrap(),
    );
    fs::write(root.join("worker.pid"), child.0.id().to_string()).unwrap();
    loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "worker exited before its controlled cancellation"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn worker_is_killed_when_its_parent_exits_even_while_worker_is_stopped() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    fs::write(source.join("lib.rs"), "pub fn entry() {}\n").unwrap();
    // A bounded 20 MiB capture gives the parent time to observe the open root.
    // No compiler build or timing-dependent analysis workload is required.
    let content = "// captured fixture\n".repeat(2048);
    for index in 0..512 {
        fs::write(source.join(format!("source-{index}.rs")), &content).unwrap();
    }
    let mut parent = ProcessGuard(
        Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("worker_cancellation_driver")
            .arg("--nocapture")
            .env("ATLAS_CANCELLATION_DRIVER_ROOT", temp.path())
            .env(
                "ATLAS_CANCELLATION_TEST_PARENT",
                std::process::id().to_string(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid_path = temp.path().join("worker.pid");
    wait_until(
        || {
            fs::read_to_string(&pid_path)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
                .is_some()
        },
        "worker PID was not published",
    );
    let worker = WorkerGuard(fs::read_to_string(pid_path).unwrap().parse().unwrap());
    // Capture opens the source root after installing its parent-death signal.
    wait_until(
        || {
            fs::read_dir(format!("/proc/{}/fd", worker.0))
                .ok()
                .is_some_and(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .any(|entry| fs::read_link(entry.path()).is_ok_and(|path| path == source))
                })
        },
        "worker did not enter captured-source processing",
    );
    kill_process(Pid::from_raw(worker.0 as i32).unwrap(), Signal::STOP).unwrap();
    wait_until(|| state(worker.0) == Some('T'), "worker did not stop");
    parent.0.kill().unwrap();
    parent.0.wait().unwrap();
    wait_until(
        || terminated(worker.0),
        "orphaned analysis worker survived its parent's exit",
    );
    assert!(
        !temp.path().join("facts.json").exists(),
        "cancelled worker published output"
    );
}

#[test]
fn worker_rejects_a_stale_parent_and_runs_with_a_small_memory_limit() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("lib.rs"), "fn entry() {}\n").unwrap();
    let job_path = temp.path().join("job.json");
    let output = temp.path().join("facts.json");
    fs::write(
        &job_path,
        serde_json::to_vec(&job(temp.path(), 0, 256)).unwrap(),
    )
    .unwrap();
    let result = worker_command(&job_path, &output).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("analysis parent exited"));
    assert!(!output.exists());
    fs::write(
        &job_path,
        serde_json::to_vec(&job(temp.path(), std::process::id(), 256)).unwrap(),
    )
    .unwrap();
    let result = worker_command(&job_path, &output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(output.is_file());
}
