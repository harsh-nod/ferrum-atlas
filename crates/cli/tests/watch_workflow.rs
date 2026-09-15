#![cfg(target_os = "linux")]

use atlas_model::SnapshotId;
use atlas_store::Store;
use rustix::process::{Pid, Signal, kill_process};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Process(Option<Child>);
impl Process {
    fn id(&self) -> u32 {
        self.0.as_ref().unwrap().id()
    }
    fn finish(mut self) -> Output {
        wait_until(
            || self.0.as_mut().unwrap().try_wait().unwrap().is_some(),
            "watch did not exit",
        );
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_until(mut condition: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn cli(store: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_atlas"));
    command.arg("--store").arg(store).stdin(Stdio::null());
    command
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("source");
    let store = temp.path().join("store");
    fs::create_dir(&workspace).unwrap();
    fs::write(
        workspace.join("Cargo.toml"),
        "[package]\nname='watch_fixture'\nversion='0.1.0'\nedition='2021'\n[lib]\npath='lib.rs'\n",
    )
    .unwrap();
    fs::write(
        workspace.join("lib.rs"),
        "pub fn leaf() {}\npub fn entry() { leaf(); }\n",
    )
    .unwrap();
    success(
        &cli(&store)
            .args(["init", "--workspace"])
            .arg(&workspace)
            .output()
            .unwrap(),
    );
    (temp, workspace, store)
}

fn watch(store: &Path, iterations: Option<u64>, interval: u64, timeout: u64) -> Process {
    let mut command = cli(store);
    command.args([
        "watch",
        "--interval-ms",
        &interval.to_string(),
        "--timeout",
        &timeout.to_string(),
        "--memory-mib",
        "512",
    ]);
    if let Some(iterations) = iterations {
        command.args(["--iterations", &iterations.to_string()]);
    }
    Process(Some(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ))
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn report(process: Process) -> Value {
    let output = process.finish();
    success(&output);
    serde_json::from_slice(&output.stdout).unwrap()
}

fn head(store: &Store) -> Option<SnapshotId> {
    store.head("default").unwrap()
}

fn worker_pid(parent: u32) -> Option<u32> {
    for task in fs::read_dir(format!("/proc/{parent}/task"))
        .ok()?
        .filter_map(Result::ok)
    {
        let children = fs::read_to_string(task.path().join("children")).unwrap_or_default();
        for child in children
            .split_whitespace()
            .filter_map(|value| value.parse::<u32>().ok())
        {
            let command = fs::read(format!("/proc/{child}/cmdline")).unwrap_or_default();
            if command
                .windows(b"worker-session".len())
                .any(|word| word == b"worker-session")
            {
                return Some(child);
            }
        }
    }
    None
}

fn signal(pid: u32, signal: Signal) {
    kill_process(Pid::from_raw(pid as i32).unwrap(), signal).unwrap();
}

#[test]
fn bounded_watch_skips_unchanged_publication_and_reuses_existing_head() {
    let (_temp, _workspace, root) = fixture();
    let first = report(watch(&root, Some(3), 50, 10));
    assert_eq!(first["iterations"], 3);
    assert_eq!(first["published"], 1);
    assert_eq!(first["unchanged"], 2);
    assert_eq!(first["worker_restarts"], 0);
    assert_eq!(first["failures"], 0);
    let store = Store::open(&root).unwrap();
    let previous = head(&store);
    let again = report(watch(&root, Some(2), 50, 10));
    assert_eq!(again["published"], 0);
    assert_eq!(again["unchanged"], 2);
    assert_eq!(head(&store), previous);
    assert_eq!(store.snapshots().unwrap().len(), 1);
    assert!(store.integrity_check().unwrap().valid);
}

#[test]
fn unchanged_source_still_upgrades_a_syntax_head_to_semantic_facts() {
    let (_temp, _workspace, root) = fixture();
    success(
        &cli(&root)
            .args(["index", "--level", "syntax", "--memory-mib", "512"])
            .output()
            .unwrap(),
    );
    let store = Store::open(&root).unwrap();
    let syntax = head(&store).unwrap();
    let result = report(watch(&root, Some(1), 50, 10));
    assert_eq!(result["published"], 1);
    assert_ne!(head(&store).unwrap(), syntax);
    assert_eq!(store.snapshots().unwrap().len(), 2);
}

#[test]
fn live_edits_keep_worker_and_refresh_spans_then_match_clean_indexing() {
    let (_temp, workspace, root) = fixture();
    let store = Store::open(&root).unwrap();
    let process = watch(&root, None, 100, 10);
    wait_until(|| head(&store).is_some(), "first watch snapshot missing");
    let first = head(&store).unwrap();
    let mut worker = None;
    wait_until(
        || {
            worker = worker_pid(process.id());
            worker.is_some()
        },
        "warm worker PID unavailable",
    );
    let edited = "// moved spans\n\npub fn leaf() {}\npub fn entry() { leaf(); leaf(); }\n";
    fs::write(workspace.join("lib.rs"), edited).unwrap();
    wait_until(
        || head(&store).as_ref().is_some_and(|id| id != &first),
        "edit not published",
    );
    let second = head(&store).unwrap();
    assert_eq!(worker_pid(process.id()), worker);
    let reader = store.reader(&second).unwrap();
    let entry = reader
        .definitions(20)
        .unwrap()
        .into_iter()
        .find(|definition| definition.name == "entry")
        .unwrap();
    let source = reader.source(&entry.span.file_id).unwrap().unwrap();
    assert_eq!(source.text, edited);
    assert!(entry.span.start as usize > edited.find("pub fn leaf").unwrap());
    drop(reader);
    fs::write(workspace.join("extra.rs"), "pub fn new_target() {}\n").unwrap();
    let final_text = "mod extra;\npub fn entry() { extra::new_target(); }\n";
    fs::write(workspace.join("lib.rs"), final_text).unwrap();
    let final_hash = atlas_model::digest("content", final_text);
    wait_until(
        || {
            head(&store).as_ref().is_some_and(|id| {
                id != &second
                    && store
                        .reader(id)
                        .unwrap()
                        .files()
                        .unwrap()
                        .iter()
                        .any(|file| file.path == "lib.rs" && file.content_hash == final_hash)
            })
        },
        "path-set edit not published",
    );
    assert_eq!(worker_pid(process.id()), worker);
    let final_warm = head(&store).unwrap();
    signal(process.id(), Signal::INT);
    let result = report(process);
    assert_eq!(result["cancelled"], true);
    assert!(result["published"].as_u64().unwrap() >= 3);
    success(
        &cli(&root)
            .args(["index", "--memory-mib", "512"])
            .output()
            .unwrap(),
    );
    assert_eq!(head(&store).unwrap(), final_warm);
    assert!(store.integrity_check().unwrap().valid);
}

#[test]
fn finite_session_restarts_after_thirty_two_captures_without_duplicate_publication() {
    let (_temp, _workspace, root) = fixture();
    let result = report(watch(&root, Some(33), 50, 10));
    assert_eq!(result["iterations"], 33);
    assert_eq!(result["published"], 1);
    assert_eq!(result["unchanged"], 32);
    assert_eq!(result["worker_restarts"], 1);
    assert_eq!(result["failures"], 0);
    assert_eq!(Store::open(&root).unwrap().snapshots().unwrap().len(), 1);
}

#[test]
fn watch_holds_index_lease_and_ctrl_c_kills_even_a_stopped_worker() {
    let (_temp, _workspace, root) = fixture();
    let store = Store::open(&root).unwrap();
    let process = watch(&root, None, 1000, 10);
    wait_until(|| head(&store).is_some(), "first watch snapshot missing");
    let previous = head(&store);
    let blocked = cli(&root)
        .args(["index", "--timeout", "1"])
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("active"));
    let blocked = watch(&root, Some(1), 50, 1).finish();
    assert!(!blocked.status.success());
    let mut worker = None;
    wait_until(
        || {
            worker = worker_pid(process.id());
            worker.is_some()
        },
        "worker PID unavailable",
    );
    let worker = worker.unwrap();
    signal(worker, Signal::STOP);
    signal(process.id(), Signal::INT);
    let result = report(process);
    assert_eq!(result["cancelled"], true);
    wait_until(
        || !Path::new(&format!("/proc/{worker}")).exists(),
        "cancelled worker was not reaped",
    );
    assert_eq!(head(&store), previous);
    success(
        &cli(&root)
            .args(["index", "--memory-mib", "512"])
            .output()
            .unwrap(),
    );
}

#[test]
fn request_deadline_kills_worker_and_preserves_last_valid_snapshot() {
    let (_temp, _workspace, root) = fixture();
    let store = Store::open(&root).unwrap();
    let process = watch(&root, Some(2), 500, 2);
    wait_until(|| head(&store).is_some(), "first watch snapshot missing");
    let previous = head(&store);
    let mut worker = None;
    wait_until(
        || {
            worker = worker_pid(process.id());
            worker.is_some()
        },
        "worker PID unavailable",
    );
    signal(worker.unwrap(), Signal::STOP);
    let result = report(process);
    assert_eq!(result["published"], 1);
    assert_eq!(result["failures"], 1);
    assert!(result["last_error"].as_str().unwrap().contains("deadline"));
    assert_eq!(head(&store), previous);
    assert!(store.integrity_check().unwrap().valid);
    wait_until(
        || !Path::new(&format!("/proc/{}", worker.unwrap())).exists(),
        "timed-out worker was not reaped",
    );
}

#[test]
fn disk_pressure_and_invalid_intervals_fail_before_publication() {
    let (_temp, _workspace, root) = fixture();
    fs::write(root.join("occupied.bin"), vec![0_u8; 900 * 1024]).unwrap();
    let output = cli(&root)
        .args(["watch", "--iterations", "1", "--disk-quota-mib", "1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("quota"));
    assert!(Store::open(&root).unwrap().snapshots().unwrap().is_empty());
    let output = cli(&root)
        .args(["watch", "--iterations", "1", "--interval-ms", "0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("interval"));
}

#[test]
fn failed_capture_preserves_head_and_recovers_in_the_same_worker() {
    let (_temp, workspace, root) = fixture();
    let store = Store::open(&root).unwrap();
    let mut process = watch(&root, None, 50, 10);
    let stderr = process.0.as_mut().unwrap().stderr.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let diagnostics = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = send.send(line);
        }
    });
    wait_until(|| head(&store).is_some(), "first watch snapshot missing");
    let previous = head(&store).unwrap();
    let worker = worker_pid(process.id()).unwrap();
    let oversized = workspace.join("too_large.rs");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(8 * 1024 * 1024)
        .unwrap();
    loop {
        let line = receive
            .recv_timeout(Duration::from_secs(15))
            .expect("failed capture was not reported");
        if line.contains("Watch capture failed") {
            break;
        }
    }
    assert_eq!(head(&store).unwrap(), previous);
    assert_eq!(worker_pid(process.id()), Some(worker));
    fs::remove_file(oversized).unwrap();
    fs::write(workspace.join("lib.rs"), "pub fn recovered() {}\n").unwrap();
    wait_until(
        || head(&store).as_ref().is_some_and(|id| id != &previous),
        "watch did not recover",
    );
    assert_eq!(worker_pid(process.id()), Some(worker));
    signal(process.id(), Signal::INT);
    let result = report(process);
    diagnostics.join().unwrap();
    assert!(result["failures"].as_u64().unwrap() >= 1);
    assert_eq!(result["worker_restarts"], 0);
    assert_eq!(result["cancelled"], true);
    assert!(store.integrity_check().unwrap().valid);
}
