use atlas_frontend::{AnalysisLevel, analyze};
use atlas_ingest::{CaptureOptions, capture};
use atlas_model::{Definition, FactBatch, Target, UnknownReason};
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
