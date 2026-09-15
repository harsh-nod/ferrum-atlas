use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

struct Project {
    temp: tempfile::TempDir,
}
impl Project {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='pilot'\nversion='0.1.0'\nedition='2021'\n[features]\nfast=[]\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "pub mod engine;\npub fn entry(value: u32) -> u32 { engine::step(value) }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/engine.rs"),
            "pub fn step(value: u32) -> u32 { if value > 0 { value + 1 } else { 0 } }\n",
        )
        .unwrap();
        Self { temp }
    }
    fn root(&self) -> String {
        self.temp
            .path()
            .join("project")
            .to_str()
            .unwrap()
            .to_owned()
    }
    fn store(&self) -> String {
        self.temp.path().join("store").to_str().unwrap().to_owned()
    }
    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_atlas"))
            .arg("--store")
            .arg(self.store())
            .args(args)
            .output()
            .unwrap()
    }
    fn run(&self, args: &[&str]) -> Value {
        let output = self.command(args);
        assert!(
            output.status.success(),
            "atlas {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "invalid output: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }
    fn init(&self) {
        self.run(&["init", "--workspace", &self.root(), "--trust", "read-only"]);
    }
    fn index(&self, level: &str) -> Value {
        self.run(&["index", "--level", level])
    }
}

#[test]
fn real_workspace_survives_restart_and_reindexes_deterministically() {
    let project = Project::new();
    project.init();
    let first = project.index("semantic");
    let second = project.index("semantic");
    assert_eq!(first["id"], second["id"]);
    assert_eq!(first["fact_digest"], second["fact_digest"]);
    let search = project.run(&["query", "search", "--text", "entry"]);
    assert_eq!(search["items"].as_array().unwrap().len(), 1);
    let id = search["items"][0]["id"].as_str().unwrap();
    let graph = project.run(&["query", "callees", "--symbol", id]);
    let names: Vec<_> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert!(names.contains(&"entry"));
    assert!(
        names.contains(&"step"),
        "direct cross-module call must resolve: {graph}"
    );
    assert!(
        graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["target"]["kind"] == "resolved")
    );
    let snapshots = project.run(&["snapshots"]);
    assert_eq!(snapshots.as_array().unwrap().len(), 1);
    assert!(project.command(&["doctor"]).status.success());
    assert!(project.command(&["gc", "--dry-run"]).status.success());
}

#[test]
fn body_edit_has_a_pinned_before_after_diff() {
    let project = Project::new();
    project.init();
    let before = project.index("semantic");
    fs::write(
        Path::new(&project.root()).join("src/engine.rs"),
        "pub fn step(value: u32) -> u32 { value + 7 }\n",
    )
    .unwrap();
    let after = project.index("semantic");
    assert_ne!(before["id"], after["id"]);
    let diff = project.run(&[
        "diff",
        "--before",
        before["id"].as_str().unwrap(),
        "--after",
        after["id"].as_str().unwrap(),
    ]);
    let changes = diff["changes"].as_array().unwrap();
    assert!(changes.iter().any(|c| {
        c["after"]["name"] == "step"
            && c["changed_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f.as_str().is_some_and(|f| f.contains("body")))
    }));
    let old = project.run(&[
        "query",
        "search",
        "--snapshot",
        before["id"].as_str().unwrap(),
        "--text",
        "step",
    ]);
    let new = project.run(&[
        "query",
        "search",
        "--snapshot",
        after["id"].as_str().unwrap(),
        "--text",
        "step",
    ]);
    assert_ne!(old["items"][0]["body_hash"], new["items"][0]["body_hash"]);
}

#[test]
fn evidence_import_requires_the_exact_artifact_and_preserves_static_facts() {
    let project = Project::new();
    project.init();
    let snapshot = project.index("semantic");
    let search = project.run(&["query", "search", "--text", "entry"]);
    let definition = &search["items"][0]["id"];
    let artifact = project.temp.path().join("artifact");
    fs::write(&artifact, b"controlled test artifact; never executed").unwrap();
    let bundle = project.temp.path().join("observations.json");
    let content = serde_json::json!({
        "schema_version": 1,
        "snapshot_id": snapshot["id"],
        "artifact": {
            "sha256": atlas_evidence::artifact_sha256(&artifact).unwrap(),
            "source_id": snapshot["source_id"],
            "context_id": snapshot["context"]["id"],
            "producer": "controlled-import-fixture"
        },
        "tests": [{
            "name": "entry", "outcome": "timeout", "elapsed_ns": "9007199254740993",
            "timeout_ns": "1000000000", "reason": "fixture timeout", "definition_ids": [definition]
        }],
        "streams": [{
            "id": "cpu0", "process_or_device": "fixture", "thread_or_hart": "0",
            "clock_domain": "fixture-clock", "timestamp_unit": "ns",
            "events": [{"sequence": "1", "timestamp": "9007199254740993", "kind": "enter",
                "definition_id": definition, "correlation_id": null, "loss_count": "0"}]
        }],
        "limitations": ["Controlled test observation; not a real executable trace"]
    });
    fs::write(&bundle, serde_json::to_vec(&content).unwrap()).unwrap();
    let args = [
        "import-evidence",
        "--snapshot",
        snapshot["id"].as_str().unwrap(),
        "--bundle",
        bundle.to_str().unwrap(),
        "--artifact",
        artifact.to_str().unwrap(),
    ];
    let imported = project.run(&args);
    assert_eq!(imported["test_count"], 1);
    assert_eq!(imported["event_count"], 1);
    assert_eq!(project.run(&args)["id"], imported["id"]);
    fs::write(&artifact, b"different artifact").unwrap();
    assert!(!project.command(&args).status.success());
    assert_eq!(
        project.run(&["snapshots"])[0]["fact_digest"],
        snapshot["fact_digest"]
    );
}

#[test]
fn opening_and_indexing_never_runs_workspace_build_scripts() {
    let project = Project::new();
    let root = Path::new(&project.root()).to_owned();
    fs::write(
        root.join("build.rs"),
        "fn main() { panic!(\"read-only capture must not execute build scripts\"); }\n",
    )
    .unwrap();
    fs::create_dir(root.join(".cargo")).unwrap();
    fs::write(
        root.join(".cargo/config.toml"),
        "[build]\nrustc-wrapper = '/nonexistent/ferrum-atlas-read-only-check'\n",
    )
    .unwrap();
    project.init();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(project.store()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let snapshot = project.index("semantic");
    assert_eq!(snapshot["context"]["trust"], "read_only");
    assert_ne!(snapshot["coverage"]["status"], "complete");
    assert!(!root.join("target").exists());
}

#[test]
fn rejected_jobs_leave_the_previous_snapshot_browsable() {
    let project = Project::new();
    project.init();
    let snapshot = project.index("syntax");
    assert!(
        !project
            .command(&["index", "--workspace", "/nonexistent/ferrum-atlas-fixture"])
            .status
            .success()
    );
    assert!(
        !project
            .command(&["index", "--memory-mib", "1"])
            .status
            .success()
    );
    assert!(
        !project
            .command(&["index", "--disk-quota-mib", "0"])
            .status
            .success()
    );
    let snapshots = project.run(&["snapshots"]);
    assert_eq!(snapshots[0]["id"], snapshot["id"]);
    let results = project.run(&["query", "search", "--text", "entry"]);
    assert_eq!(results["items"].as_array().unwrap().len(), 1);
}

#[test]
fn profiles_exports_and_raw_benchmark_samples_are_explicit() {
    let project = Project::new();
    project.init();
    let first = project.index("syntax");
    let other = project.run(&[
        "index",
        "--level",
        "syntax",
        "--profile",
        "feature-fast",
        "--features",
        "fast",
    ]);
    assert_ne!(first["context"]["id"], other["context"]["id"]);
    let output = project.temp.path().join("evidence.json");
    let export = project.command(&[
        "export",
        "--snapshot",
        first["id"].as_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let data: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(data["snapshot"]["id"], first["id"]);
    assert!(!data["limitations"].as_array().unwrap().is_empty());
    let benchmark = project.run(&[
        "benchmark",
        "--snapshot",
        first["id"].as_str().unwrap(),
        "--samples",
        "30",
    ]);
    assert_eq!(benchmark["samples_ms"].as_array().unwrap().len(), 30);
    assert_eq!(benchmark["sample_results"].as_array().unwrap().len(), 30);
    assert!(
        benchmark["sample_results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|sample| sample["ok"] == true && sample["items"].as_u64().unwrap() > 0)
    );
    assert!(benchmark["preparation_ms"].as_f64().unwrap() >= 0.0);
    assert!(benchmark["p95_ms"].as_f64().unwrap() >= 0.0);
    assert!(
        !project
            .command(&["benchmark", "--samples", "2"])
            .status
            .success()
    );
    assert!(!project.command(&["gc"]).status.success());
    assert!(
        !project
            .command(&["serve", "--listen", "0.0.0.0:7878"])
            .status
            .success()
    );
}

#[test]
fn portable_snapshot_opens_without_original_workspace_or_analyzer_execution() {
    let project = Project::new();
    project.init();
    let snapshot = project.index("semantic");
    let archive = project.temp.path().join("portable");
    project.run(&[
        "export",
        "--format",
        "portable",
        "--snapshot",
        snapshot["id"].as_str().unwrap(),
        "--output",
        archive.to_str().unwrap(),
    ]);
    let restored = project.temp.path().join("restored");
    let output = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .arg("--store")
        .arg(&restored)
        .args(["import", "--input", archive.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let imported: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshot["id"], imported["id"]);
    fs::remove_dir_all(project.root()).unwrap();
    let store = atlas_store::Store::open(restored).unwrap();
    let reader = store
        .reader(&atlas_model::SnapshotId(
            imported["id"].as_str().unwrap().into(),
        ))
        .unwrap();
    assert!(reader.files().unwrap().iter().any(|file| {
        reader
            .source(&file.id)
            .unwrap()
            .unwrap()
            .text
            .contains("engine::step")
    }));
    assert_eq!(
        reader.definitions(100).unwrap().len() as u64,
        snapshot["definition_count"].as_u64().unwrap()
    );
}

#[test]
fn exact_gc_plan_preserves_heads_and_pins_then_reclaims_only_unpinned_history() {
    let project = Project::new();
    project.init();
    let first = project.index("syntax");
    project.run(&[
        "pin",
        "--snapshot",
        first["id"].as_str().unwrap(),
        "--name",
        "review",
    ]);
    fs::write(
        Path::new(&project.root()).join("src/engine.rs"),
        "pub fn step(value:u32)->u32 {value+2}\n",
    )
    .unwrap();
    let second = project.index("syntax");
    let plan = project.run(&[
        "gc",
        "--dry-run",
        "--keep-recent",
        "0",
        "--grace-seconds",
        "0",
    ]);
    assert!(plan["remove_snapshots"].as_array().unwrap().is_empty());
    project.run(&["unpin", "--name", "review"]);
    let plan = project.run(&[
        "gc",
        "--dry-run",
        "--keep-recent",
        "0",
        "--grace-seconds",
        "0",
    ]);
    assert!(
        plan["remove_snapshots"]
            .as_array()
            .unwrap()
            .contains(&first["id"])
    );
    let path = project.temp.path().join("gc-plan.json");
    fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    project.run(&["gc", "--execute", "--plan", path.to_str().unwrap()]);
    let snapshots = project.run(&["snapshots"]);
    assert_eq!(snapshots.as_array().unwrap().len(), 1);
    assert_eq!(snapshots[0]["id"], second["id"]);
    assert!(Path::new(&project.root()).join("src/engine.rs").is_file());
    assert!(project.command(&["doctor"]).status.success());
}
