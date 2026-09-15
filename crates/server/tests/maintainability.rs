use atlas_frontend::{AnalysisLevel, analyze};
use atlas_ingest::{CaptureOptions, capture};
use atlas_model::*;
use atlas_query::QueryEngine;
use atlas_server::{ServerConfig, router};
use atlas_store::Store;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::Value;
use std::fs;
use tower::ServiceExt;

struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
    config: ServerConfig,
    snapshot: Snapshot,
    definition: Definition,
}
impl Fixture {
    fn new(extra_bytes: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname='metrics_fixture'\nversion='0.1.0'\nedition='2021'\n[lib]\npath='lib.rs'\n").unwrap();
        let text = format!(
            "// file heading\r\nfn measure(x: bool) {{\r\n // excluded comment\r\n if x {{\r\n  unsafe {{ core::ptr::read_volatile(&0); }}\r\n }}\r\n}}\r\n{}",
            " ".repeat(extra_bytes)
        );
        fs::write(root.join("lib.rs"), text).unwrap();
        let (source, context) = capture(&root, &CaptureOptions::default()).unwrap();
        let facts = analyze(source, context, AnalysisLevel::Syntax).unwrap();
        let definition = facts
            .definitions
            .iter()
            .find(|d| d.name == "measure")
            .unwrap()
            .clone();
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
        }
    }
    async fn get(&self, context: &str, allowed: bool) -> (StatusCode, Value) {
        let query = QueryEngine::new(self.store.clone()).unwrap();
        let query = if allowed {
            query
        } else {
            query.with_repositories([])
        };
        let app = router(query, &self.config).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/analysis/maintainability/{}?snapshot_id={}&context_id={context}",
                        self.definition.id, self.snapshot.id
                    ))
                    .header("host", "127.0.0.1:7878")
                    .header("authorization", format!("Bearer {}", "a".repeat(64)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let result = serde_json::from_slice(
            &to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        (status, result)
    }
}

#[tokio::test]
async fn captured_source_metrics_have_pinned_scope_counts_and_exact_unsafe_location() {
    let f = Fixture::new(0);
    let (status, result) = f.get(&f.snapshot.context.id.0, true).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["snapshot_id"], f.snapshot.id.0);
    assert_eq!(result["context_id"], f.snapshot.context.id.0);
    let report = &result["analysis"];
    assert_eq!(report["definition_id"], f.definition.id.0);
    assert_eq!(report["metrics"]["source_lines"], 5);
    assert_eq!(report["metrics"]["unsafe_boundaries"], 1);
    assert!(report["metrics"]["lexical_tokens"].as_u64().unwrap() > 10);
    assert_eq!(report["envelope"]["truncated"], false);
    assert_eq!(
        report["unsafe_sites"][0]["keyword_span"]["file_id"],
        f.definition.file_id.0
    );
    assert_eq!(
        report["unsafe_sites"][0]["keyword_span"]["end"]
            .as_u64()
            .unwrap()
            - report["unsafe_sites"][0]["keyword_span"]["start"]
                .as_u64()
                .unwrap(),
        6
    );
    assert!(!f.config.observations_dir.exists());
    let (_, repeated) = f.get(&f.snapshot.context.id.0, true).await;
    assert_eq!(report["input_digest"], repeated["analysis"]["input_digest"]);
}

#[tokio::test]
async fn metric_authorization_and_context_checks_precede_source_access() {
    let f = Fixture::new(0);
    fs::rename(
        f._temp.path().join("store/sources"),
        f._temp.path().join("withheld-sources"),
    )
    .unwrap();
    assert_eq!(
        f.get(&f.snapshot.context.id.0, false).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(f.get("context:wrong", true).await.0, StatusCode::CONFLICT);
    let allowed = f.get(&f.snapshot.context.id.0, true).await.0;
    assert!(!allowed.is_success());
    assert_ne!(allowed, StatusCode::FORBIDDEN);
    assert_ne!(allowed, StatusCode::CONFLICT);
}

#[tokio::test]
async fn over_limit_source_is_not_a_complete_empty_metric_report() {
    let f = Fixture::new(270_000);
    let (status, result) = f.get(&f.snapshot.context.id.0, true).await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT, "{result}");
    assert_eq!(result["code"], "budget_exhausted");
    assert!(result.get("analysis").is_none());
}
