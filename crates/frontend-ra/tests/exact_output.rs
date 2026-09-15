use atlas_frontend::{AnalysisLevel, analyze};
use atlas_model::*;
use std::collections::BTreeMap;

fn fixture(index: usize) -> (SourceSnapshot, BuildContext) {
    let body = match index {
        0 => {
            "pub mod helper; fn leaf() {} pub fn entry() { leaf(); helper::leaf(); let callback = || leaf(); callback(); if true { missing_a(); } missing_b(); }"
        }
        1 => {
            "pub mod helper; #[cfg(feature = \"selected\")] #[cfg_attr(enabled, allow(dead_code))] mod nested { #![cfg(not(test))] pub fn entry() { #[cfg(false)] { skipped(); } #[cfg_attr(false, wrapper)] let callback = || super::helper::leaf(); callback(); } } #[cfg_attr(enabled, wrapper)] fn wrapped() { helper::leaf(); }"
        }
        2 => {
            "pub mod helper; #[cfg(unknown)] mod nested { #[cfg(false)] fn inactive() { skipped(); } pub fn entry() { helper_call(); } } #[cfg_attr(unknown, wrapper)] fn wrapped() { helper::leaf(); } fn event() { missing_a(); missing_b(); missing_a(); }"
        }
        _ => unreachable!(),
    };
    let texts = BTreeMap::from([("src/helper.rs", "pub fn leaf() {}"), ("src/lib.rs", body)]);
    let source_id = SourceId(digest("frontend-regression-source", &texts));
    let manifests = BTreeMap::from([(
        "Cargo.toml".into(),
        "[package]\nname='regression'\nversion='0.1.0'\nedition='2021'\n[features]\nselected=[]\n"
            .into(),
    )]);
    let manifest_digest = digest("manifests", &manifests);
    let source = SourceSnapshot {
        id: source_id.clone(),
        repository_id: RepositoryId(digest("frontend-regression-repository", &"fixture")),
        revision: "frozen frontend regression".into(),
        files: texts
            .into_iter()
            .map(|(path, text)| SourceFile {
                id: FileId(digest("file", &(&source_id, path))),
                path: path.into(),
                content_hash: digest("content", &text),
                text: text.into(),
            })
            .collect(),
        manifests,
        warnings: vec![],
    };
    let context = BuildContext {
        id: ContextId(digest("frontend-regression-context", &manifest_digest)),
        name: "regression".into(),
        target: "x86_64-unknown-linux-gnu".into(),
        features: vec!["selected".into()],
        default_features: true,
        cfg: BTreeMap::from([("enabled".into(), None)]),
        crates: vec![CrateInput {
            name: "regression".into(),
            root_file: "src/lib.rs".into(),
            edition: "2021".into(),
            dependencies: BTreeMap::new(),
        }],
        manifest_digest,
        trust: "read_only".into(),
        coverage: Coverage::complete(),
    };
    (source, context)
}

#[test]
fn every_fact_field_matches_the_pre_optimization_adapter() {
    // Frozen against 942a496 in both modes. This is an exact-output regression,
    // not an independent semantic oracle for the rust-analyzer producer.
    let expected = [
        [
            "frontend-regression-facts:49abcc1b47f4fd8c22e90c53fb8a4411ec54b7b565d6afc37453cf5a6e71b795",
            "frontend-regression-facts:d1f0c91436ebfaa3f8cc5e403db37e816022ccac6c49d12e3b97941a8072abb4",
        ],
        [
            "frontend-regression-facts:c87d100fa836a24b6ac1a32a839f1a8a19ae41a85a422cac7fd107d22579353b",
            "frontend-regression-facts:2c9c26a1bde5cca3f9562ee127363feab052f99ead57868ffaf274f8f3d5eb54",
        ],
        [
            "frontend-regression-facts:9587a4eef6d83ad4fb56ff845a3aedaece720a34bd2e58fb29a324f3cbc87edb",
            "frontend-regression-facts:e18413116506582f789e030c6c5c2bdf15aa287babb8169a9dff95fd092a68ea",
        ],
    ];
    let mut observed = Vec::new();
    for index in 0..expected.len() {
        let mut hashes = Vec::new();
        for level in [AnalysisLevel::Syntax, AnalysisLevel::Semantic] {
            let (source, context) = fixture(index);
            let actual = analyze(source, context, level).unwrap();
            let actual = digest("frontend-regression-facts", &actual);
            hashes.push(actual);
        }
        observed.push(hashes);
    }
    assert_eq!(observed, expected);
}
