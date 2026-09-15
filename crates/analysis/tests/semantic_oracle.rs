use atlas_frontend::{AnalysisLevel, analyze};
use atlas_ingest::{CaptureOptions, capture};
use atlas_model::{FactBatch, Target, UnknownReason};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    process::Command,
};

const SOURCE: &str = include_str!("fixtures/semantic_oracle.rs");
#[derive(Deserialize)]
struct Oracle {
    version: u32,
    provenance: String,
    calls: Vec<ExpectedCall>,
}
#[derive(Deserialize)]
struct ExpectedCall {
    caller: String,
    text: String,
    target: Option<String>,
    unknown: Option<UnknownReason>,
}

fn fixture() -> (tempfile::TempDir, FactBatch) {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("src")).unwrap();
    fs::write(
        directory.path().join("Cargo.toml"),
        "[package]\nname='independent_oracle'\nversion='0.1.0'\nedition='2021'\n",
    )
    .unwrap();
    fs::write(directory.path().join("src/lib.rs"), SOURCE).unwrap();
    let (source, context) = capture(directory.path(), &CaptureOptions::default()).unwrap();
    let facts = analyze(source, context, AnalysisLevel::Semantic).unwrap();
    (directory, facts)
}

fn check_oracle(facts: &FactBatch) -> std::result::Result<(), String> {
    let oracle: Oracle =
        serde_json::from_str(include_str!("fixtures/semantic_oracle.json")).unwrap();
    assert_eq!(oracle.version, 1);
    assert!(oracle.provenance.contains("Hand-authored"));
    let definitions = facts
        .definitions
        .iter()
        .map(|item| (&item.id, item.qualified_name.as_str()))
        .collect::<BTreeMap<_, _>>();
    let callers = oracle
        .calls
        .iter()
        .map(|call| call.caller.as_str())
        .collect::<BTreeSet<_>>();
    let mut unmatched = facts
        .relations
        .iter()
        .filter(|relation| {
            definitions
                .get(&relation.source)
                .is_some_and(|name| callers.contains(name))
        })
        .collect::<Vec<_>>();
    for expected in &oracle.calls {
        let matches = unmatched
            .iter()
            .enumerate()
            .filter(|(_, relation)| {
                definitions.get(&relation.source) == Some(&expected.caller.as_str())
                    && SOURCE.get(relation.span.start as usize..relation.span.end as usize)
                        == Some(expected.text.as_str())
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!(
                "expected one independently specified site {} in {}, got {}",
                expected.text,
                expected.caller,
                matches.len()
            ));
        }
        let (index, relation) = matches[0];
        let valid = match &relation.target {
            Target::Resolved { id } => {
                expected.unknown.is_none()
                    && definitions.get(id).copied() == expected.target.as_deref()
            }
            Target::Unknown { reason, .. } => {
                expected.target.is_none() && expected.unknown.as_ref() == Some(reason)
            }
        };
        if !valid {
            return Err(format!(
                "wrong target for independent site {} in {}: {:?}",
                expected.text, expected.caller, relation.target
            ));
        }
        unmatched.remove(index);
    }
    if !unmatched.is_empty() {
        return Err(format!(
            "{} extra call sites not present in the independent contract",
            unmatched.len()
        ));
    }
    Ok(())
}

#[test]
fn hand_authored_call_contract_matches_semantic_facts() {
    let (_, facts) = fixture();
    check_oracle(&facts).unwrap();
}

#[test]
fn deliberate_wrong_target_mutation_is_caught_by_independent_oracle() {
    let (_, mut facts) = fixture();
    check_oracle(&facts).unwrap();
    let wrong = facts
        .definitions
        .iter()
        .find(|item| item.qualified_name == "src::lib::right::same")
        .unwrap()
        .id
        .clone();
    let selected = facts
        .relations
        .iter_mut()
        .find(|relation| {
            SOURCE.get(relation.span.start as usize..relation.span.end as usize)
                == Some("selected()")
        })
        .unwrap();
    selected.target = Target::Resolved { id: wrong };
    assert!(check_oracle(&facts).unwrap_err().contains("wrong target"));
}

#[test]
fn dropped_callsite_and_false_resolved_pointer_mutations_are_caught() {
    let (_, facts) = fixture();
    let mut missing = facts.clone();
    let index = missing
        .relations
        .iter()
        .position(|relation| {
            SOURCE.get(relation.span.start as usize..relation.span.end as usize)
                == Some("selected()")
        })
        .unwrap();
    missing.relations.remove(index);
    assert!(check_oracle(&missing).is_err());
    let mut guessed = facts;
    let wrong = guessed
        .definitions
        .iter()
        .find(|item| item.qualified_name == "src::lib::left::same")
        .unwrap()
        .id
        .clone();
    guessed
        .relations
        .iter_mut()
        .find(|relation| {
            SOURCE.get(relation.span.start as usize..relation.span.end as usize)
                == Some("pointer()")
        })
        .unwrap()
        .target = Target::Resolved { id: wrong };
    assert!(check_oracle(&guessed).is_err());
}

#[test]
fn pinned_rustc_compiles_and_runs_only_the_trusted_hand_authored_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("oracle.rs");
    let binary = directory.path().join("oracle-tests");
    fs::write(&source, SOURCE).unwrap();
    let compile = Command::new("rustc")
        .args(["--edition=2021", "--test"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(binary)
        .arg("--test-threads=1")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{} {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}
