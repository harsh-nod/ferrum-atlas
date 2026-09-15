use atlas_model::*;
use atlas_query::{QueryControl, QueryEngine};
use atlas_store::Store;
use std::collections::BTreeMap;
use std::time::Duration;

fn fixture(text: String) -> (tempfile::TempDir, QueryEngine, Snapshot, FileId) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let file = FileId("file:source-control".into());
    let context = BuildContext {
        id: ContextId("context:source-control".into()),
        name: "test".into(),
        target: "host".into(),
        features: vec![],
        default_features: true,
        cfg: BTreeMap::new(),
        crates: vec![],
        manifest_digest: "manifest:test".into(),
        trust: "read_only".into(),
        coverage: Coverage::complete(),
    };
    let batch = FactBatch {
        schema_version: SCHEMA_VERSION,
        source: SourceSnapshot {
            id: SourceId(digest("source", &text)),
            repository_id: RepositoryId("repository:source-control".into()),
            revision: "test".into(),
            files: vec![SourceFile {
                id: file.clone(),
                path: "src/lib.rs".into(),
                content_hash: digest("content", &text),
                text,
            }],
            manifests: BTreeMap::new(),
            warnings: vec![],
        },
        context,
        producer: "source-control-test/1".into(),
        definitions: vec![],
        relations: vec![],
        evidence: vec![],
        flows: vec![],
        coverage: Coverage::complete(),
        diagnostics: vec![],
    };
    let snapshot = store.publish(&batch, "main", None).unwrap();
    (temp, QueryEngine::new(store).unwrap(), snapshot, file)
}

#[test]
fn source_requests_reject_precancelled_and_expired_controls() {
    let (_temp, query, snapshot, file) = fixture("fn main() {}\r\n".into());
    let cancelled = QueryControl::new(Duration::from_secs(30));
    cancelled.cancel();
    for control in [cancelled, QueryControl::new(Duration::ZERO)] {
        assert_eq!(
            query
                .source_with_control(&snapshot.id, &snapshot.context.id, &file, 1, 1, &control)
                .unwrap_err()
                .code(),
            "budget_exhausted"
        );
        assert_eq!(
            query
                .source_at_byte_with_control(
                    &snapshot.id,
                    &snapshot.context.id,
                    &file,
                    0,
                    1,
                    &control
                )
                .unwrap_err()
                .code(),
            "budget_exhausted"
        );
        assert_eq!(
            query
                .source_for_analysis(
                    &snapshot.id,
                    &snapshot.context.id,
                    &file,
                    256 * 1024,
                    &control
                )
                .unwrap_err()
                .code(),
            "budget_exhausted"
        );
    }
}

#[test]
fn exact_analysis_sources_are_authorized_scoped_and_byte_bounded() {
    let text = "// \u{03bb}\u{1f600}\r\nfn main() {}\r\n".to_owned();
    let (_temp, query, snapshot, file) = fixture(text.clone());
    let control = QueryControl::new(Duration::from_secs(30));
    let source = query
        .source_for_analysis(
            &snapshot.id,
            &snapshot.context.id,
            &file,
            text.len(),
            &control,
        )
        .unwrap();
    assert_eq!(source.text, text);
    assert_eq!(source.content_hash, digest("content", &text));
    for cap in [0, 256 * 1024 + 1] {
        assert_eq!(
            query
                .source_for_analysis(&snapshot.id, &snapshot.context.id, &file, cap, &control)
                .unwrap_err()
                .code(),
            "invalid_query"
        );
    }
    assert_eq!(
        query
            .source_for_analysis(
                &snapshot.id,
                &snapshot.context.id,
                &file,
                text.len() - 1,
                &control
            )
            .unwrap_err()
            .code(),
        "budget_exhausted"
    );
    assert_eq!(
        query
            .source_for_analysis(
                &snapshot.id,
                &ContextId("context:wrong".into()),
                &file,
                256 * 1024,
                &control
            )
            .unwrap_err()
            .code(),
        "context_mismatch"
    );
    assert_eq!(
        query
            .source_for_analysis(
                &snapshot.id,
                &snapshot.context.id,
                &FileId("file:missing".into()),
                256 * 1024,
                &control
            )
            .unwrap_err()
            .code(),
        "not_found"
    );
    let denied = query.with_repositories([]);
    assert_eq!(
        denied
            .source_for_analysis(
                &snapshot.id,
                &snapshot.context.id,
                &file,
                256 * 1024,
                &control
            )
            .unwrap_err()
            .code(),
        "not_authorized"
    );
}

#[test]
fn giant_source_windows_are_honest_and_analysis_never_gets_a_prefix() {
    let prefix = "x".repeat(2 * 1024 * 1024);
    let text = format!("{prefix}\u{03bb}\r\nend\r\n");
    let (_temp, query, snapshot, file) = fixture(text.clone());
    let control = QueryControl::new(Duration::from_secs(30));
    let window = query
        .source_with_control(&snapshot.id, &snapshot.context.id, &file, 1, 1, &control)
        .unwrap();
    assert_eq!(window.text.len(), 256 * 1024);
    assert_eq!(window.total_lines, 3);
    assert!(window.truncated);
    let end = query
        .source_at_byte_with_control(
            &snapshot.id,
            &snapshot.context.id,
            &file,
            (prefix.len() + 4) as u32,
            1,
            &control,
        )
        .unwrap();
    assert_eq!(end.text, "end\r\n");
    assert_eq!(end.start_line, 2);
    assert!(end.truncated);
    assert_eq!(
        query
            .source_at_byte_with_control(
                &snapshot.id,
                &snapshot.context.id,
                &file,
                (prefix.len() + 1) as u32,
                1,
                &control
            )
            .unwrap_err()
            .code(),
        "invalid_query"
    );
    assert_eq!(
        query
            .source_for_analysis(
                &snapshot.id,
                &snapshot.context.id,
                &file,
                256 * 1024,
                &control
            )
            .unwrap_err()
            .code(),
        "budget_exhausted"
    );
    assert_eq!(
        query
            .source_with_control(
                &snapshot.id,
                &snapshot.context.id,
                &file,
                1,
                1,
                &QueryControl::new(Duration::ZERO)
            )
            .unwrap_err()
            .code(),
        "budget_exhausted"
    );
}
