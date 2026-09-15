use atlas_frontend::{AnalysisLevel, analyze};
use atlas_ingest::{CaptureOptions, capture};
use atlas_model::{CfgStatus, Definition, FactBatch, Target, UnknownReason};
use std::{collections::BTreeSet, fs};

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    for (path, text) in files {
        fs::create_dir_all(dir.path().join(path).parent().unwrap()).unwrap();
        fs::write(dir.path().join(path), text).unwrap();
    }
    dir
}

fn facts(dir: &tempfile::TempDir, level: AnalysisLevel) -> FactBatch {
    let (source, context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
    analyze(source, context, level).unwrap()
}

fn definition<'a>(facts: &'a FactBatch, name: &str) -> &'a Definition {
    let matches = facts
        .definitions
        .iter()
        .filter(|item| item.name == name)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one definition named {name}");
    matches[0]
}

fn targets(facts: &FactBatch, name: &str) -> Vec<String> {
    let id = &definition(facts, name).id;
    facts
        .relations
        .iter()
        .filter(|edge| &edge.source == id)
        .filter_map(|edge| match &edge.target {
            Target::Resolved { id } => Some(
                facts
                    .definitions
                    .iter()
                    .find(|item| &item.id == id)
                    .unwrap()
                    .qualified_name
                    .clone(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn resolves_modules_aliases_recursion_and_same_names_by_hir_identity() {
    let dir = project(&[
        (
            "Cargo.toml",
            "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nalias={package='helper',path='helper'}\n",
        ),
        (
            "src/lib.rs",
            "mod left; mod right; use left::same as selected; pub fn entry() { selected(); right::same(); alias::leaf(); } fn recursive() { recursive(); }",
        ),
        ("src/left.rs", "pub fn same() {}"),
        ("src/right.rs", "pub fn same() {}"),
        (
            "helper/Cargo.toml",
            "[package]\nname='helper'\nversion='0.1.0'\nedition='2021'\n",
        ),
        ("helper/src/lib.rs", "pub fn leaf() {}"),
    ]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    let targets = targets(&facts, "entry")
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        targets,
        BTreeSet::from([
            "src::left::same".into(),
            "src::right::same".into(),
            "helper::src::lib::leaf".into()
        ])
    );
    let recursive = definition(&facts, "recursive");
    assert!(
        facts
            .relations
            .iter()
            .any(|edge| edge.source == recursive.id
                && matches!(&edge.target, Target::Resolved { id } if id == &recursive.id))
    );
    let same = facts
        .definitions
        .iter()
        .filter(|item| item.name == "same")
        .collect::<Vec<_>>();
    assert_eq!(same.len(), 2);
    assert_ne!(same[0].id, same[1].id);
    assert!(facts.relations.iter().all(|edge| edge.kind == "calls"));
    assert!(
        facts
            .evidence
            .iter()
            .any(|evidence| evidence.basis == "resolved")
    );
}

#[test]
fn resolves_inherent_methods_and_keeps_dynamic_and_foreign_calls_unknown() {
    let dir = project(&[(
        "src/lib.rs",
        "struct Engine; impl Engine { fn step(&self) {} } trait Dynamic { fn act(&self); } unsafe extern \"C\" { fn external(); } fn entry(engine: Engine, dynamic: &dyn Dynamic, callback: fn()) { engine.step(); dynamic.act(); callback(); unsafe { external(); } }",
    )]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert_eq!(
        targets(&facts, "entry"),
        vec!["src::lib::impl Engine::step"]
    );
    let reasons = facts
        .relations
        .iter()
        .filter_map(|edge| match &edge.target {
            Target::Unknown { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert!(
        reasons.contains(&UnknownReason::IndirectTargetUnknown),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&UnknownReason::ExternalBoundary),
        "{reasons:?}"
    );
}

#[test]
fn does_not_guess_shadowed_function_pointer_targets() {
    let dir = project(&[(
        "src/lib.rs",
        "fn target() {} fn caller(target: fn()) { target(); }",
    )]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert!(targets(&facts, "caller").is_empty());
    assert!(facts.relations.iter().any(|edge| matches!(
        edge.target,
        Target::Unknown {
            reason: UnknownReason::IndirectTargetUnknown,
            ..
        }
    )));
}

#[test]
fn syntax_mode_never_emits_resolved_targets_and_is_repeatable() {
    let dir = project(&[("src/lib.rs", "fn leaf() {} fn caller() { leaf(); }")]);
    let first = facts(&dir, AnalysisLevel::Syntax);
    assert_eq!(first, facts(&dir, AnalysisLevel::Syntax));
    assert_eq!(first.relations.len(), 1);
    assert!(matches!(
        first.relations[0].target,
        Target::Unknown {
            reason: UnknownReason::SyntaxOnly,
            ..
        }
    ));
    assert_eq!(
        facts(&dir, AnalysisLevel::Semantic),
        facts(&dir, AnalysisLevel::Semantic)
    );
}

#[test]
fn cfg_and_macros_retain_source_and_never_manufacture_complete_negatives() {
    let dir = project(&[(
        "src/lib.rs",
        "#[cfg(firmware)] fn target() {} #[cfg(not(firmware))] fn target() {} fn caller() { target(); generated!(); } generate_items!();",
    )]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert_eq!(
        facts
            .definitions
            .iter()
            .filter(|item| item.name == "target")
            .count(),
        2
    );
    assert!(
        facts
            .definitions
            .iter()
            .filter(|item| item.name == "target")
            .all(|item| !item.cfg.is_empty()),
        "{:#?}",
        facts.definitions
    );
    assert!(facts.relations.iter().any(|edge| matches!(
        edge.target,
        Target::Unknown {
            reason: UnknownReason::CfgUnknown,
            ..
        }
    )));
    assert!(facts.relations.iter().any(|edge| matches!(
        edge.target,
        Target::Unknown {
            reason: UnknownReason::MacroUnavailable,
            ..
        }
    )));
    assert_ne!(facts.coverage.status, atlas_model::Status::Complete);
}

#[test]
fn flow_and_calls_belong_to_their_own_function_or_closure() {
    let dir = project(&[(
        "src/lib.rs",
        "fn leaf() {} async fn outer(flag: bool) { fn nested() { if true { leaf(); return; } } let closure = || { if true { leaf(); } }; if flag { return; } unsafe { leaf(); } async {}.await; }",
    )]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    let outer = definition(&facts, "outer");
    assert_eq!(outer.metrics.branches, 1);
    assert_eq!(outer.metrics.returns, 1);
    assert_eq!(outer.metrics.awaits, 1);
    assert_eq!(outer.metrics.unsafe_blocks, 1);
    assert_eq!(targets(&facts, "outer").len(), 1);
    assert_eq!(targets(&facts, "nested").len(), 1);
    assert_eq!(
        facts
            .definitions
            .iter()
            .filter(|item| item.kind == "closure")
            .count(),
        1
    );
    assert!(
        facts
            .flows
            .iter()
            .all(|flow| flow.phase == "source"
                && flow.coverage.status == atlas_model::Status::Partial)
    );
}

#[test]
fn broken_unicode_crlf_source_remains_browsable_with_byte_exact_spans() {
    let text = "pub fn caf\u{e9}() { let text = \"\u{1f680}\"; }\r\nfn broken( {\r\n";
    let dir = project(&[("src/lib.rs", text)]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    let cafe = definition(&facts, "caf\u{e9}");
    assert_eq!(
        &text[cafe.span.start as usize..cafe.span.end as usize],
        "pub fn caf\u{e9}() { let text = \"\u{1f680}\"; }"
    );
    assert_eq!(facts.source.files[0].text, text);
    assert!(!facts.diagnostics.is_empty());
}

#[test]
fn separate_local_items_have_distinct_owner_keys() {
    let dir = project(&[(
        "src/lib.rs",
        "fn first() { fn local() {} local(); } fn second() { fn local() {} local(); }",
    )]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert_eq!(targets(&facts, "first"), vec!["src::lib::first::local"]);
    assert_eq!(targets(&facts, "second"), vec!["src::lib::second::local"]);
}

#[test]
fn known_features_defaults_and_test_cfg_choose_only_the_selected_definition() {
    let dir = project(&[
        (
            "Cargo.toml",
            "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n[features]\ndefault=['fast']\nfast=['extra']\nextra=[]\n",
        ),
        (
            "src/lib.rs",
            "#[cfg(feature=\"extra\")] fn selected() { fast_leaf(); } #[cfg(not(feature=\"extra\"))] fn selected() { slow_leaf(); } fn entry() { selected(); } fn fast_leaf() {} fn slow_leaf() {} #[cfg(test)] mod tests { #[cfg(unknown_inside_inactive)] fn omitted() { absent(); } }",
        ),
    ]);
    for (options, expected_leaf) in [
        (CaptureOptions::default(), "fast_leaf"),
        (
            CaptureOptions {
                default_features: false,
                ..Default::default()
            },
            "slow_leaf",
        ),
        (
            CaptureOptions {
                default_features: false,
                features: vec!["fast".into()],
                ..Default::default()
            },
            "fast_leaf",
        ),
    ] {
        let (source, context) = capture(dir.path(), &options).unwrap();
        let facts = analyze(source, context, AnalysisLevel::Semantic).unwrap();
        let selected = facts
            .definitions
            .iter()
            .find(|item| item.name == "selected" && item.cfg_status == CfgStatus::Active)
            .unwrap();
        assert!(
            facts
                .relations
                .iter()
                .any(|edge| edge.source == definition(&facts, "entry").id
                    && matches!(&edge.target, Target::Resolved { id } if id == &selected.id)),
            "{:#?}",
            facts.relations
        );
        assert!(facts.relations.iter().any(|edge| edge.source == selected.id && matches!(&edge.target, Target::Resolved { id } if id == &definition(&facts, expected_leaf).id)));
        let inactive = facts
            .definitions
            .iter()
            .find(|item| item.name == "selected" && item.cfg_status == CfgStatus::Inactive)
            .unwrap();
        assert!(
            !facts
                .relations
                .iter()
                .any(|edge| edge.source == inactive.id)
        );
        assert_eq!(
            definition(&facts, "omitted").cfg_status,
            CfgStatus::Inactive
        );
        assert!(!facts.relations.iter().any(|edge| matches!(
            edge.target,
            Target::Unknown {
                reason: UnknownReason::CfgUnknown,
                ..
            }
        )));
    }
}

#[test]
fn explicit_cfg_values_and_combinators_preserve_three_state_logic() {
    let dir = project(&[(
        "src/lib.rs",
        "#[cfg(all(firmware,target_os=\"none\",not(test)))] fn selected() {} #[cfg(not(all(firmware,target_os=\"none\",not(test))))] fn selected() {} fn entry() { selected(); } #[cfg(any(test,all()))] fn always() {} #[cfg_attr(test,cfg(unavailable))] fn untouched() {}",
    )]);
    let options = CaptureOptions {
        cfg: std::collections::BTreeMap::from([
            ("firmware".into(), None),
            ("target_os".into(), Some("none".into())),
        ]),
        ..Default::default()
    };
    let (source, context) = capture(dir.path(), &options).unwrap();
    let facts = analyze(source, context, AnalysisLevel::Semantic).unwrap();
    assert_eq!(definition(&facts, "always").cfg_status, CfgStatus::Active);
    assert_eq!(
        definition(&facts, "untouched").cfg_status,
        CfgStatus::Active
    );
    assert_eq!(targets(&facts, "entry").len(), 1);
    assert_eq!(
        facts
            .definitions
            .iter()
            .filter(|item| item.name == "selected" && item.cfg_status == CfgStatus::Active)
            .count(),
        1
    );
    let (source, context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
    let facts = analyze(source, context, AnalysisLevel::Semantic).unwrap();
    assert!(
        facts
            .definitions
            .iter()
            .filter(|item| item.name == "selected")
            .all(|item| item.cfg_status == CfgStatus::Unknown)
    );
    assert!(targets(&facts, "entry").is_empty());
}

#[test]
fn incomplete_dependency_feature_resolution_stays_unknown() {
    let dir = project(&[
        (
            "Cargo.toml",
            "[package]\nname='fixture'\nversion='0.1.0'\nedition='2021'\n[features]\ndefault=['other/fast']\n[dependencies]\nother={path='other'}\n",
        ),
        ("src/lib.rs", "fn entry() { other::selected(); }"),
        (
            "other/Cargo.toml",
            "[package]\nname='other'\nversion='0.1.0'\nedition='2021'\n[features]\nfast=[]\n",
        ),
        (
            "other/src/lib.rs",
            "#[cfg(feature=\"fast\")] pub fn selected() {} #[cfg(not(feature=\"fast\"))] pub fn selected() {}",
        ),
    ]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert!(targets(&facts, "entry").is_empty());
    assert!(
        facts
            .definitions
            .iter()
            .filter(|item| item.name == "selected")
            .all(|item| item.cfg_status == CfgStatus::Unknown)
    );
}

#[test]
fn explicit_cyclic_or_missing_crate_inputs_fail_without_panicking() {
    let dir = project(&[("src/lib.rs", "fn entry() {}")]);
    let (source, mut context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
    context.crates[0]
        .dependencies
        .insert("self_cycle".into(), "src/lib.rs".into());
    assert!(
        analyze(source.clone(), context.clone(), AnalysisLevel::Semantic)
            .unwrap_err()
            .to_string()
            .contains("dependency graph")
    );
    context.crates[0].dependencies.clear();
    context.crates[0].root_file = "missing.rs".into();
    assert!(analyze(source, context, AnalysisLevel::Semantic).is_err());
}

#[test]
fn shared_source_in_distinct_crate_instances_is_not_conflated() {
    let dir = project(&[
        (
            "src/lib.rs",
            "mod common; pub fn target() {} pub fn library() { common::entry(); }",
        ),
        (
            "src/main.rs",
            "mod common; fn target() {} fn main() { common::entry(); }",
        ),
        ("src/common.rs", "pub fn entry() { crate::target(); }"),
    ]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert_eq!(definition(&facts, "entry").cfg_status, CfgStatus::Unknown);
    assert!(targets(&facts, "entry").is_empty());
    assert!(targets(&facts, "library").is_empty());
    assert!(targets(&facts, "main").is_empty());
    assert!(
        facts
            .coverage
            .limitations
            .iter()
            .any(|text| text.contains("multiple crate instances"))
    );
}

#[test]
fn rejects_oversized_or_ambiguous_source_inputs_before_database_loading() {
    let dir = project(&[("src/lib.rs", "fn entry() {}")]);
    let (mut source, context) = capture(dir.path(), &CaptureOptions::default()).unwrap();
    source.files.push(source.files[0].clone());
    assert!(
        analyze(source.clone(), context.clone(), AnalysisLevel::Syntax)
            .unwrap_err()
            .to_string()
            .contains("duplicate source")
    );
    source.files.pop();
    source.files[0].text = " ".repeat(4 * 1024 * 1024 + 1);
    assert!(
        analyze(source, context, AnalysisLevel::Semantic)
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
}

#[test]
fn workspace_dependency_overrides_and_virtual_feature_selection_stay_unknown() {
    let dir = project(&[
        (
            "Cargo.toml",
            "[workspace]\nmembers=['member','helper']\n[workspace.dependencies]\nhelper={path='helper',default-features=false}\n",
        ),
        (
            "member/Cargo.toml",
            "[package]\nname='member'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nhelper.workspace=true\n[features]\nfast=[]\n",
        ),
        (
            "member/src/lib.rs",
            "pub fn entry() { helper::selected(); } #[cfg(feature=\"fast\")] fn enabled() {}",
        ),
        (
            "helper/Cargo.toml",
            "[package]\nname='helper'\nversion='0.1.0'\nedition='2021'\n[features]\ndefault=['fast']\nfast=[]\n",
        ),
        (
            "helper/src/lib.rs",
            "#[cfg(feature=\"fast\")] pub fn selected() {} #[cfg(not(feature=\"fast\"))] pub fn selected() {}",
        ),
    ]);
    let first = facts(&dir, AnalysisLevel::Semantic);
    assert!(targets(&first, "entry").is_empty());
    assert!(
        first
            .definitions
            .iter()
            .filter(|item| item.name == "selected")
            .all(|item| item.cfg_status == CfgStatus::Unknown)
    );
    fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers=['member','helper']\n[workspace.dependencies]\nhelper={path='helper'}\n").unwrap();
    let options = CaptureOptions {
        features: vec!["member/fast".into()],
        ..Default::default()
    };
    let (source, context) = capture(dir.path(), &options).unwrap();
    let second = analyze(source, context, AnalysisLevel::Semantic).unwrap();
    assert_eq!(
        definition(&second, "enabled").cfg_status,
        CfgStatus::Unknown
    );
}

#[test]
fn explicit_feature_cfg_matches_the_semantic_database() {
    let dir = project(&[(
        "src/lib.rs",
        "#[cfg(feature=\"custom\")] fn selected() {} #[cfg(not(feature=\"custom\"))] fn selected() {} fn entry() { selected(); }",
    )]);
    let options = CaptureOptions {
        cfg: std::collections::BTreeMap::from([("feature".into(), Some("custom".into()))]),
        ..Default::default()
    };
    let (source, context) = capture(dir.path(), &options).unwrap();
    let facts = analyze(source, context, AnalysisLevel::Semantic).unwrap();
    let selected = facts
        .definitions
        .iter()
        .find(|item| item.name == "selected" && item.cfg_status == CfgStatus::Active)
        .unwrap();
    assert!(selected.cfg[0].contains("#[cfg(feature="));
    assert!(
        facts
            .relations
            .iter()
            .any(|edge| matches!(&edge.target, Target::Resolved { id } if id == &selected.id))
    );
}

#[test]
fn shared_source_in_distinct_module_instances_is_not_conflated() {
    let dir = project(&[
        (
            "src/lib.rs",
            "#[path=\"common.rs\"] mod left; #[path=\"common.rs\"] mod right; fn target() {} fn entry() { left::call(); right::call(); }",
        ),
        ("src/common.rs", "pub fn call() { super::target(); }"),
    ]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert_eq!(definition(&facts, "call").cfg_status, CfgStatus::Active);
    assert!(targets(&facts, "call").is_empty());
    assert!(targets(&facts, "entry").is_empty());
}

#[test]
fn syntax_diagnostics_respect_the_crate_edition() {
    let dir = project(&[
        (
            "Cargo.toml",
            "[package]\nname='legacy'\nversion='0.1.0'\nedition='2015'\n",
        ),
        ("src/lib.rs", "fn async() {} fn entry() { async(); }"),
    ]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert!(facts.diagnostics.is_empty(), "{:?}", facts.diagnostics);
    assert_eq!(targets(&facts, "entry"), vec!["src::lib::async"]);
}

#[test]
fn unavailable_attribute_macros_do_not_claim_original_function_behavior() {
    let dir = project(&[(
        "src/lib.rs",
        "#[unavailable] fn transformed() { leaf(); } #[cfg_attr(all(),unavailable)] fn conditional() { leaf(); } #[cfg_attr(test,unavailable)] fn unchanged() { leaf(); } fn leaf() {} fn entry() { transformed(); conditional(); unchanged(); }",
    )]);
    let facts = facts(&dir, AnalysisLevel::Semantic);
    assert!(targets(&facts, "transformed").is_empty());
    assert!(targets(&facts, "conditional").is_empty());
    assert_eq!(targets(&facts, "unchanged"), vec!["src::lib::leaf"]);
    assert_eq!(targets(&facts, "entry"), vec!["src::lib::unchanged"]);
    assert!(
        facts
            .relations
            .iter()
            .any(|edge| edge.source == definition(&facts, "transformed").id
                && matches!(
                    edge.target,
                    Target::Unknown {
                        reason: UnknownReason::MacroUnavailable,
                        ..
                    }
                ))
    );
}
