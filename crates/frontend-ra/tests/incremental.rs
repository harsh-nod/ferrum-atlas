use atlas_frontend::{AnalysisLevel, AnalyzerSession, analyze};
use atlas_ingest::{CaptureOptions, capture};
use std::fs;

#[test]
fn warm_dependency_updates_equal_clean_analysis_across_edit_sequences() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    let manifest =
        "[package]\nname='incremental'\nversion='0.1.0'\nedition='2021'\n[features]\nfast=[]\n";
    fs::write(temp.path().join("Cargo.toml"), manifest).unwrap();
    let edits = [
        "mod dep; pub fn entry()->u32 { dep::value() }",
        "mod dep; pub fn entry()->u32 { dep::value() + 1 }",
        "mod dep;\r\n\r\n pub fn entry()->u32 { dep::value() + 1 }\r\n",
        "mod dep; use dep::value as alias; pub fn entry()->u32 { alias() }",
        "mod dep; pub fn renamed()->u32 { dep::value() }",
        "mod dep; #[cfg(feature=\"fast\")] pub fn entry()->u32 { dep::value() }",
        "mod dep; pub fn entry() { let f=dep::value; f(); }",
        "mod dep; pub fn entry( { dep::value() }",
    ];
    let dependencies = [
        "pub fn value()->u32 { 1 }",
        "const VALUE:u32=2; pub fn value()->u32 { VALUE }",
        "pub fn value()->u64 { 3 }",
        "trait T {fn value(&self)->u32;} struct A; impl T for A {fn value(&self)->u32{4}} pub fn value()->u32 { A.value() }",
    ];
    let mut warm = AnalyzerSession::new();
    let mut seed = 0x41544c41u64;
    for step in 0..48 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        fs::write(
            temp.path().join("src/lib.rs"),
            edits[(seed as usize >> 8) % edits.len()],
        )
        .unwrap();
        fs::write(
            temp.path().join("src/dep.rs"),
            dependencies[(seed as usize >> 16) % dependencies.len()],
        )
        .unwrap();
        let extra = temp.path().join("src/extra.rs");
        if step % 3 == 0 {
            fs::write(&extra, "pub fn extra() {}\n").unwrap();
        } else if extra.exists() {
            fs::remove_file(&extra).unwrap();
        }
        let mut options = CaptureOptions::default();
        if step % 5 == 0 {
            options.features.push("fast".into());
        }
        if step % 7 == 0 {
            options.cfg.insert("test".into(), None);
        }
        let (source, context) = capture(temp.path(), &options).unwrap();
        let incremental = warm
            .analyze(source.clone(), context.clone(), AnalysisLevel::Semantic)
            .unwrap();
        let clean = analyze(source, context, AnalysisLevel::Semantic).unwrap();
        assert_eq!(
            incremental, clean,
            "clean/incremental mismatch at edit {step}"
        );
    }
}

#[test]
fn same_path_body_edits_refresh_resolved_targets_and_source_spans() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname='stable'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    let mut session = AnalyzerSession::new();
    for body in ["a()", "b()", "\r\n b()", "a()"] {
        fs::write(
            temp.path().join("src/lib.rs"),
            format!("fn a(){{}} fn b(){{}} pub fn entry(){{{body}}}"),
        )
        .unwrap();
        let (source, context) = capture(temp.path(), &CaptureOptions::default()).unwrap();
        let warm = session
            .analyze(source.clone(), context.clone(), AnalysisLevel::Semantic)
            .unwrap();
        let clean = analyze(source, context, AnalysisLevel::Semantic).unwrap();
        assert_eq!(warm, clean);
    }
}
