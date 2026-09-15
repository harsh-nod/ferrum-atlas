use atlas_analysis::{
    AnalysisControl, StateMachineInference, StateMachineLimits, StateTransitionDecision,
    StateTransitionReview, infer_state_machine,
};
use atlas_frontend::{AnalysisLevel, analyze};
use atlas_ingest::{CaptureOptions, capture};
use atlas_model::*;
use atlas_query::QueryEngine;
use atlas_server::{ServerConfig, router};
use atlas_store::Store;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{fs, time::Duration};
use tower::ServiceExt;

struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    config: ServerConfig,
    snapshot: Snapshot,
    definition: Definition,
    inference: StateMachineInference,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("Cargo.toml"), "[package]\nname='state_fixture'\nversion='0.1.0'\nedition='2021'\n[lib]\npath='lib.rs'\n").unwrap();
        fs::write(workspace.join("lib.rs"), "enum State { Idle, Running }\nfn event(mut state: State) { match state { State::Idle => { state = State::Running; }, State::Running => { state = State::Idle; } } }\n").unwrap();
        let (source, context) = capture(&workspace, &CaptureOptions::default()).unwrap();
        let facts = analyze(source, context, AnalysisLevel::Syntax).unwrap();
        let definition = facts
            .definitions
            .iter()
            .find(|d| d.name == "event")
            .unwrap()
            .clone();
        let file = facts
            .source
            .files
            .iter()
            .find(|file| file.id == definition.file_id)
            .unwrap();
        let inference = infer_state_machine(
            file,
            &definition,
            "State",
            "state",
            StateMachineLimits::default(),
            &AnalysisControl::new(Duration::from_secs(2)),
        )
        .unwrap();
        assert_eq!(inference.candidates.len(), 2);
        let store = Store::open(temp.path().join("store")).unwrap();
        let snapshot = store.publish(&facts, "main", None).unwrap();
        let config = ServerConfig {
            listen: "127.0.0.1:7878".parse().unwrap(),
            token: "a".repeat(64),
            web_dir: temp.path().join("web"),
            max_concurrent_queries: 4,
            observations_dir: temp.path().join("observations"),
            compiler_dir: temp.path().join("compiler"),
            scheduler: None,
        };
        Self {
            _temp: temp,
            store,
            config,
            snapshot,
            definition,
            inference,
        }
    }
    fn app(&self, allowed: bool) -> Router {
        let query = QueryEngine::new(self.store.clone()).unwrap();
        let query = if allowed {
            query
        } else {
            query.with_repositories([])
        };
        router(query, &self.config).unwrap()
    }
    fn selection(&self) -> Value {
        json!({ "snapshot_id": self.snapshot.id, "context_id": self.snapshot.context.id, "enum_path": "State", "state_place": "state" })
    }
    fn review(&self) -> StateTransitionReview {
        StateTransitionReview {
            input_digest: self.inference.input_digest.clone(),
            candidate_id: self.inference.candidates[0].id.clone(),
            reviewer: "Fixture reviewer".into(),
            note: "Checked direct assignment, not whole-machine behavior".into(),
            decision: StateTransitionDecision::Accepted,
        }
    }
    fn route(&self) -> String {
        format!("/v1/analysis/state-machine/{}", self.definition.id)
    }
}
async fn post(app: Router, path: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .method("POST")
                .header("host", "127.0.0.1:7878")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let value = serde_json::from_slice(
        &to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    (status, value)
}

#[tokio::test]
async fn real_state_inference_review_and_restart_keep_claims_separate_and_pinned() {
    let f = Fixture::new();
    let (status, result) = post(f.app(true), &f.route(), f.selection()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        result["analysis"]["candidates"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        result["analysis"]["envelope"]["coverage"]["status"],
        "partial"
    );
    let payload = json!({ "selection": f.selection(), "review": f.review() });
    for _ in 0..2 {
        let (status, result) = post(
            f.app(true),
            &format!("{}/reviews", f.route()),
            payload.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["decision"], "accepted");
    }
    let reviews = atlas_evidence::state_machine::list(
        &f.config.observations_dir.join("state-reviews"),
        &f.snapshot.id,
        &f.snapshot.context.id,
        &f.inference,
    )
    .unwrap();
    assert_eq!(reviews.len(), 1);
    assert!(!f.store.pins().unwrap().is_empty());
    let query = format!(
        "{}/reviews?snapshot_id={}&context_id={}&enum_path=State&state_place=state",
        f.route(),
        f.snapshot.id,
        f.snapshot.context.id
    );
    let reopened = Store::open(f._temp.path().join("store")).unwrap();
    let app = router(QueryEngine::new(reopened).unwrap(), &f.config).unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri(query)
                .header("host", "127.0.0.1:7878")
                .header("authorization", format!("Bearer {}", "a".repeat(64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let persisted: Vec<StateTransitionReview> =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(persisted.len(), 1);
}

#[tokio::test]
async fn state_review_rejects_wrong_context_scope_stale_digest_and_unknown_candidates() {
    let f = Fixture::new();
    assert_eq!(
        post(f.app(false), &f.route(), f.selection()).await.0,
        StatusCode::FORBIDDEN
    );
    let mut wrong = f.selection();
    wrong["context_id"] = json!("context:wrong");
    assert_eq!(
        post(f.app(true), &f.route(), wrong).await.0,
        StatusCode::CONFLICT
    );
    for field in ["input_digest", "candidate_id", "reviewer"] {
        let mut review = serde_json::to_value(f.review()).unwrap();
        review[field] = json!("");
        assert_eq!(
            post(
                f.app(true),
                &format!("{}/reviews", f.route()),
                json!({"selection": f.selection(), "review": review})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(!f.config.observations_dir.exists());
    assert!(f.store.pins().unwrap().is_empty());
}

#[test]
fn immutable_review_storage_detects_corruption_symlinks_and_bounds_the_snapshot() {
    let f = Fixture::new();
    let root = f.config.observations_dir.join("state-reviews");
    assert!(
        atlas_evidence::state_machine::record_with_stop(
            &root,
            &f.snapshot.id,
            &f.snapshot.context.id,
            &f.inference,
            f.review(),
            &|| true,
        )
        .is_err()
    );
    assert!(!root.exists());
    let record = |review| {
        atlas_evidence::state_machine::record(
            &root,
            &f.snapshot.id,
            &f.snapshot.context.id,
            &f.inference,
            review,
        )
    };
    for index in 0..100 {
        let mut review = f.review();
        review.note = index.to_string();
        record(review).unwrap();
    }
    let mut duplicate = f.review();
    duplicate.note = "0".into();
    record(duplicate).unwrap();
    assert!(record(f.review()).is_err());
    let folder = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
    let object = fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|p| p.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let bytes = fs::read(&object).unwrap();
    fs::write(&object, b"{}").unwrap();
    assert!(
        atlas_evidence::state_machine::list(
            &root,
            &f.snapshot.id,
            &f.snapshot.context.id,
            &f.inference
        )
        .is_err()
    );
    fs::remove_file(&object).unwrap();
    let outside = f._temp.path().join("outside.json");
    fs::write(&outside, bytes).unwrap();
    std::os::unix::fs::symlink(&outside, &object).unwrap();
    assert!(
        atlas_evidence::state_machine::list(
            &root,
            &f.snapshot.id,
            &f.snapshot.context.id,
            &f.inference
        )
        .is_err()
    );
}
