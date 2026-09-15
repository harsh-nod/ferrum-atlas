#![feature(rustc_private)]

use atlas_rustc_adapter::{compiler::*, sha256};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn command(root: &Path, source: &str, output: &Path, panic: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_atlas-rustc"));
    command
        .args(["--trusted-local", "--root"])
        .arg(root)
        .args(["--source", source, "--output"])
        .arg(output)
        .args(["--crate-name", "fixture", "--panic", panic]);
    command
}

fn extract(panic: &str) -> CompilerBundle {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("bundle.json");
    let result = command(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
        "control_flow.rs",
        &output,
        panic,
    )
    .output()
    .unwrap();
    success(&result);
    serde_json::from_slice(&fs::read(output).unwrap()).unwrap()
}

fn success(result: &Output) {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn body<'a>(bundle: &'a CompilerBundle, name: &str) -> &'a CompilerBody {
    bundle
        .bodies
        .iter()
        .find(|body| body.def_path == format!("fixture::{name}"))
        .unwrap()
}

#[test]
fn hand_expected_branch_and_return_cfg() {
    let bundle = extract("unwind");
    assert_eq!(bundle.schema_version, 1);
    assert_eq!(bundle.phase, "runtime_optimized");
    assert_eq!(
        bundle.compiler.commit_hash,
        "55e86c996809902e8bbad512cfb4d2c18be446d9"
    );
    assert_eq!(bundle.inputs.target, "x86_64-unknown-linux-gnu");
    assert_eq!(bundle.inputs.mir_opt_level, 0);
    assert_eq!(bundle.inputs.trust, "trusted_local");
    assert!(bundle.inputs.compiled_artifact.is_none());
    let branch = body(&bundle, "branch");
    let switches: Vec<_> = branch
        .blocks
        .iter()
        .filter(|block| block.terminator.kind == "switch_int")
        .collect();
    assert_eq!(switches.len(), 1);
    assert_eq!(switches[0].terminator.successors.len(), 2);
    assert!(
        switches[0]
            .terminator
            .successors
            .iter()
            .any(|edge| edge.kind == CompilerEdgeKind::SwitchValue
                && edge.switch_value.as_deref() == Some("0"))
    );
    assert!(
        switches[0]
            .terminator
            .successors
            .iter()
            .any(|edge| edge.kind == CompilerEdgeKind::Otherwise)
    );
    assert_eq!(
        branch
            .blocks
            .iter()
            .filter(|block| block.terminator.kind == "return")
            .count(),
        1
    );
    assert!(branch.blocks.iter().all(|block| !block.is_cleanup));
    let returned = body(&bundle, "return_value");
    assert_eq!(returned.blocks.len(), 1);
    assert_eq!(returned.argument_count, 1);
    let assignment = returned.blocks[0]
        .statements
        .iter()
        .find(|statement| statement.kind == "assign")
        .unwrap();
    assert_eq!(assignment.locals.defs, [0]);
    assert_eq!(assignment.locals.uses, [1]);
    assert_eq!(returned.blocks[0].terminator.kind, "return");
    assert_eq!(returned.blocks[0].terminator.locals.uses, [0]);
    assert!(returned.blocks[0].terminator.successors.is_empty());
}

fn normal_drop_order(body: &CompilerBody, start: u32) -> Vec<u32> {
    let mut current = start;
    let mut drops = Vec::new();
    for _ in 0..body.blocks.len() {
        let block = &body.blocks[current as usize];
        if block.terminator.kind == "drop" {
            assert_eq!(block.terminator.locals.uses.len(), 1);
            drops.push(block.terminator.locals.uses[0]);
        }
        let next = block
            .terminator
            .successors
            .iter()
            .find(|edge| edge.kind == CompilerEdgeKind::Normal);
        match next {
            Some(next) => current = next.target,
            None => return drops,
        }
    }
    panic!("fixture path unexpectedly cycles");
}

#[test]
fn hand_expected_reverse_drop_order_and_cleanup_unwind() {
    let bundle = extract("unwind");
    let body = body(&bundle, "with_drop");
    let local = |name: &str| {
        body.locals
            .iter()
            .find(|local| local.names.iter().any(|candidate| candidate == name))
            .unwrap()
            .index
    };
    let expected = vec![local("second"), local("first")];
    let call = body
        .blocks
        .iter()
        .find(|block| block.terminator.kind == "call")
        .unwrap();
    assert_eq!(normal_drop_order(body, call.index), expected);
    let CompilerUnwind::Cleanup { target } = call.terminator.unwind.as_ref().unwrap() else {
        panic!("callback requires cleanup unwind")
    };
    assert!(body.blocks[*target as usize].is_cleanup);
    assert_eq!(normal_drop_order(body, *target), expected);
    assert_eq!(
        body.blocks
            .iter()
            .filter(|block| block.terminator.kind == "drop")
            .count(),
        4
    );
    assert!(
        body.blocks
            .iter()
            .any(|block| block.is_cleanup && block.terminator.kind == "unwind_resume")
    );
    assert!(body.blocks.iter().any(|block| matches!(&block.terminator.unwind, Some(CompilerUnwind::Terminate { reason }) if reason == "panic_during_cleanup")));
    assert!(matches!(
        call.terminator.call_target,
        Some(CompilerCallTarget::Indirect { .. })
    ));
    assert!(
        call.terminator
            .locals
            .unknown_effects
            .contains(&CompilerUnknownEffect::IndirectCall)
    );
    assert!(call.terminator.locals.defs.is_empty());
    assert_eq!(call.terminator.normal_return_defs.len(), 1);
}

#[test]
fn abort_configuration_has_no_cleanup_but_keeps_normal_drops() {
    let bundle = extract("abort");
    assert_eq!(bundle.inputs.panic_strategy, "abort");
    let body = body(&bundle, "with_drop");
    assert!(body.blocks.iter().all(|block| !block.is_cleanup));
    assert_eq!(
        body.blocks
            .iter()
            .filter(|block| block.terminator.kind == "drop")
            .count(),
        2
    );
    assert!(
        body.blocks
            .iter()
            .flat_map(|block| &block.terminator.successors)
            .all(|edge| edge.kind != CompilerEdgeKind::Unwind)
    );
}

#[test]
fn unknown_memory_effects_and_compiler_item_calls_stay_explicit() {
    let bundle = extract("unwind");
    let call = body(&bundle, "direct_call")
        .blocks
        .iter()
        .find(|block| block.terminator.kind == "call")
        .unwrap();
    assert!(
        matches!(&call.terminator.call_target, Some(CompilerCallTarget::FunctionDefinition { def_path, is_local: true }) if def_path == "fixture::return_value")
    );
    assert!(
        call.terminator
            .locals
            .unknown_effects
            .contains(&CompilerUnknownEffect::Call)
    );
    let writes: Vec<_> = body(&bundle, "pointer_write")
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            statement
                .locals
                .unknown_effects
                .contains(&CompilerUnknownEffect::PartialWrite)
        })
        .collect();
    assert!(!writes.is_empty());
    assert!(
        writes
            .iter()
            .all(|statement| !statement.locals.defs.contains(&1))
    );
    assert!(writes.iter().any(|statement| {
        statement
            .locals
            .unknown_effects
            .contains(&CompilerUnknownEffect::PointerAliasing)
    }));
    assert!(body(&bundle, "divide").blocks.iter().any(
        |block| block.terminator.kind == "assert" && block.terminator.assert_expected.is_some()
    ));
}

#[test]
fn bundle_roundtrip_is_deterministic_and_cross_references_are_valid() {
    let first = extract("unwind");
    assert_eq!(first, extract("unwind"));
    assert_ne!(
        first.inputs.manifest_hash,
        extract("abort").inputs.manifest_hash
    );
    let source =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/control_flow.rs")).unwrap();
    assert_eq!(first.inputs.files[0].sha256, sha256(&source));
    assert_eq!(first.inputs.files[0].byte_length, source.len() as u32);
    for body in &first.bodies {
        assert!(body.body_id.starts_with("mir:"));
        for block in &body.blocks {
            assert!(block.terminator.source_scope < body.source_scopes.len() as u32);
            for edge in &block.terminator.successors {
                assert!(edge.target < body.blocks.len() as u32);
            }
            for local in block
                .terminator
                .locals
                .uses
                .iter()
                .chain(&block.terminator.normal_return_defs)
            {
                assert!(*local < body.locals.len() as u32);
            }
        }
        for scope in &body.source_scopes {
            assert!(
                scope
                    .parent
                    .is_none_or(|parent| parent < body.source_scopes.len() as u32)
            );
        }
    }
    let mut json = serde_json::to_value(first).unwrap();
    json["unexpected_field"] = true.into();
    assert!(serde_json::from_value::<CompilerBundle>(json).is_err());
}

#[test]
fn exact_source_mapping_preserves_crlf_and_utf8_byte_offsets() {
    let temp = tempfile::tempdir().unwrap();
    let text = "// original bytes\r\npub fn identity(\u{03c0}: u8) -> u8 { \u{03c0} }\r\n";
    fs::write(temp.path().join("lib.rs"), text).unwrap();
    let output = temp.path().join("facts.json");
    success(
        &command(temp.path(), "lib.rs", &output, "unwind")
            .output()
            .unwrap(),
    );
    let bundle: CompilerBundle = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(bundle.inputs.files[0].sha256, sha256(text.as_bytes()));
    let CompilerSourceMapping::Exact {
        path,
        start_byte,
        end_byte,
    } = &body(&bundle, "identity").span
    else {
        panic!("fixture has a direct source span")
    };
    assert_eq!(path, "lib.rs");
    assert!(text[*start_byte as usize..*end_byte as usize].contains("fn identity"));
    assert_eq!(*start_byte as usize, text.find("pub fn").unwrap());
}

#[test]
fn rejects_untrusted_broken_and_escaping_inputs_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("facts.json");
    fs::write(temp.path().join("broken.rs"), "pub fn bad( {").unwrap();
    let untrusted = Command::new(env!("CARGO_BIN_EXE_atlas-rustc"))
        .args(["--root"])
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(!untrusted.status.success());
    assert!(String::from_utf8_lossy(&untrusted.stderr).contains("--trusted-local"));
    assert!(
        !command(temp.path(), "broken.rs", &output, "unwind")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!output.exists());
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(
        temp.path().join("nested/lib.rs"),
        "include!(\"../outside.rs\");",
    )
    .unwrap();
    fs::write(temp.path().join("outside.rs"), "pub fn outside() {}").unwrap();
    let failed = command(&temp.path().join("nested"), "lib.rs", &output, "unwind")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(!output.exists());
}

#[test]
fn inherited_environment_is_not_visible_to_compiled_source() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "pub const VALUE: &str = env!(\"ATLAS_TEST_SECRET\");",
    )
    .unwrap();
    let output = temp.path().join("facts.json");
    let failed = command(temp.path(), "lib.rs", &output, "unwind")
        .env("ATLAS_TEST_SECRET", "must-not-leak")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(!output.exists());
    assert!(!String::from_utf8_lossy(&failed.stderr).contains("must-not-leak"));
}

#[test]
fn source_edit_changes_input_and_body_identity_without_overwriting_outputs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("lib.rs"), "pub fn selected() -> u8 { 1 }").unwrap();
    let first = temp.path().join("first.json");
    success(
        &command(temp.path(), "lib.rs", &first, "unwind")
            .output()
            .unwrap(),
    );
    let original = fs::read(&first).unwrap();
    fs::write(temp.path().join("lib.rs"), "pub fn selected() -> u8 { 2 }").unwrap();
    let failed = command(temp.path(), "lib.rs", &first, "unwind")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(fs::read(&first).unwrap(), original);
    let second = temp.path().join("second.json");
    success(
        &command(temp.path(), "lib.rs", &second, "unwind")
            .output()
            .unwrap(),
    );
    let first: CompilerBundle = serde_json::from_slice(&original).unwrap();
    let second: CompilerBundle = serde_json::from_slice(&fs::read(second).unwrap()).unwrap();
    assert_ne!(first.inputs.manifest_hash, second.inputs.manifest_hash);
    assert_ne!(first.bodies[0].body_id, second.bodies[0].body_id);
}

#[test]
fn macro_generated_body_does_not_claim_direct_source_mapping() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "macro_rules! create { () => { pub fn generated() {} }; } create!();",
    )
    .unwrap();
    let output = temp.path().join("facts.json");
    success(
        &command(temp.path(), "lib.rs", &output, "unwind")
            .output()
            .unwrap(),
    );
    let bundle: CompilerBundle = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert!(matches!(
        body(&bundle, "generated").span,
        CompilerSourceMapping::Unavailable { .. }
    ));
}

#[test]
fn explicit_cfg_selects_a_single_named_build_and_is_recorded() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("lib.rs"),
        "#[cfg(chosen)] pub fn enabled() {} #[cfg(not(chosen))] pub fn disabled() {}",
    )
    .unwrap();
    let output = temp.path().join("facts.json");
    success(
        &command(temp.path(), "lib.rs", &output, "unwind")
            .args(["--cfg", "chosen"])
            .output()
            .unwrap(),
    );
    let bundle: CompilerBundle = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(bundle.bodies.len(), 1);
    assert_eq!(bundle.bodies[0].def_path, "fixture::enabled");
    assert!(
        bundle
            .inputs
            .rustc_args
            .windows(2)
            .any(|pair| pair == ["--cfg", "chosen"])
    );
}

#[test]
fn option_shaped_source_filename_is_passed_as_a_source_not_a_flag() {
    let temp = tempfile::tempdir().unwrap();
    let source = "-Copt-level=3.rs";
    fs::write(temp.path().join(source), "pub fn entry() {}").unwrap();
    let output = temp.path().join("facts.json");
    success(
        &command(temp.path(), source, &output, "unwind")
            .output()
            .unwrap(),
    );
    let bundle: CompilerBundle = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(bundle.bodies[0].def_path, "fixture::entry");
    assert_eq!(bundle.inputs.crate_root, source);
    assert_eq!(bundle.inputs.rustc_args[1], format!("./{source}"));
    assert!(
        !bundle
            .inputs
            .rustc_args
            .iter()
            .any(|arg| arg.starts_with('/'))
    );
}
