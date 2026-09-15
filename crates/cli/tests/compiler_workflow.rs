use atlas_model::{CompilerBundle, Snapshot};
use std::{fs, path::PathBuf, process::Command};

#[test]
#[ignore = "requires the separately built pinned adapter via ATLAS_RUSTC"]
fn real_compiler_bundle_imports_only_into_its_captured_context_and_source() {
    let adapter = std::env::var_os("ATLAS_RUSTC").expect("set ATLAS_RUSTC to the pinned adapter");
    let temp = tempfile::tempdir().unwrap();
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../adapters/rustc/fixtures")
        .canonicalize()
        .unwrap();
    let store_path = temp.path().join("store");
    let bundle_path = temp.path().join("mir.json");
    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_atlas"))
            .arg("--store")
            .arg(&store_path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    run(&["init", "--workspace", workspace.to_str().unwrap()]);
    let output = Command::new(adapter)
        .args(["--trusted-local", "--root"])
        .arg(&workspace)
        .args([
            "--source",
            "control_flow.rs",
            "--crate-name",
            "atlas_compiler_fixture",
            "--output",
        ])
        .arg(&bundle_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle: CompilerBundle = serde_json::from_slice(&fs::read(&bundle_path).unwrap()).unwrap();
    let captured: Snapshot = serde_json::from_value(run(&[
        "index",
        "--target",
        "x86_64-unknown-linux-gnu",
        "--no-default-features",
    ]))
    .unwrap();
    let imported = run(&[
        "import-compiler",
        "--snapshot",
        &captured.id.0,
        "--bundle",
        bundle_path.to_str().unwrap(),
    ]);
    assert!(
        imported["mapped_count"].as_u64().unwrap() >= 5,
        "{imported}"
    );
    let store = atlas_store::Store::open(&store_path).unwrap();
    let reader = store.reader(&captured.id).unwrap();
    let branch = reader
        .definitions(100)
        .unwrap()
        .into_iter()
        .find(|d| d.name == "branch")
        .unwrap();
    let flow = atlas_evidence::compiler::flow(
        &store_path.join("compiler"),
        &captured.id,
        &captured.context.id,
        &branch.id,
        imported["id"].as_str().unwrap(),
        (0, 200),
    )
    .unwrap();
    assert_eq!(flow.phase, "runtime_optimized");
    assert!(flow.body.blocks.len() >= 3);
    assert_eq!(flow.compiler, bundle.compiler);
    assert!(
        store
            .pins()
            .unwrap()
            .iter()
            .any(|pin| pin.snapshot_id == captured.id)
    );
    let wrong: Snapshot =
        serde_json::from_value(run(&["index", "--target", "x86_64-unknown-linux-gnu"])).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .arg("--store")
        .arg(&store_path)
        .args(["import-compiler", "--snapshot", &wrong.id.0, "--bundle"])
        .arg(&bundle_path)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        atlas_evidence::compiler::list(&store_path.join("compiler"), &wrong.id, &wrong.context.id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        atlas_evidence::compiler::list(
            &store_path.join("compiler"),
            &captured.id,
            &captured.context.id
        )
        .unwrap()
        .len(),
        1
    );
}
