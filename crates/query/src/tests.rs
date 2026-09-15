use super::*;

fn batch(count: usize) -> FactBatch {
    let text = "fn root() {}\r\n// \u{03bb}\u{1f600}\r\n".to_string();
    let file = FileId("file:test".into());
    let context = BuildContext {
        id: ContextId("context:test".into()),
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
    let definitions = (0..count)
        .map(|n| Definition {
            id: DefinitionId(format!("definition:{n:05}")),
            context_id: context.id.clone(),
            file_id: file.clone(),
            name: format!("symbol{n:05}"),
            qualified_name: format!("test::symbol{n:05}"),
            kind: "function".into(),
            parent_id: None,
            signature: "fn root()".into(),
            signature_hash: "signature:test".into(),
            body_hash: "body:test".into(),
            span: Span {
                file_id: file.clone(),
                start: 0,
                end: 12,
            },
            body_span: None,
            cfg: vec![],
            cfg_status: CfgStatus::Active,
            visibility: "private".into(),
            metrics: Metrics::default(),
        })
        .collect::<Vec<_>>();
    let relations = (1..count)
        .map(|n| Relation {
            id: RelationId(format!("relation:{n:05}")),
            source: definitions[0].id.clone(),
            target: Target::Resolved {
                id: definitions[n].id.clone(),
            },
            kind: "calls".into(),
            span: Span {
                file_id: file.clone(),
                start: 0,
                end: 2,
            },
            evidence_id: EvidenceId("evidence:test".into()),
        })
        .collect();
    FactBatch {
        schema_version: SCHEMA_VERSION,
        source: SourceSnapshot {
            id: SourceId("source:test".into()),
            repository_id: RepositoryId("repository:test".into()),
            revision: "one".into(),
            files: vec![SourceFile {
                id: file,
                path: "src/lib.rs".into(),
                content_hash: digest("content", &text),
                text,
            }],
            manifests: BTreeMap::new(),
            warnings: vec![],
        },
        context,
        producer: "fixture/1".into(),
        definitions,
        relations,
        evidence: vec![Evidence {
            id: EvidenceId("evidence:test".into()),
            basis: "resolved".into(),
            producer: "fixture/1".into(),
            inputs: vec![],
            assumptions: vec![],
            limitations: vec![],
        }],
        flows: vec![],
        coverage: Coverage::complete(),
        diagnostics: vec![],
    }
}

fn engine(count: usize) -> (tempfile::TempDir, QueryEngine, Snapshot) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let snapshot = store.publish(&batch(count), "main", None).unwrap();
    (temp, QueryEngine::new(store).unwrap(), snapshot)
}

fn graph(snapshot: &Snapshot) -> GraphRequest {
    GraphRequest {
        snapshot_id: snapshot.id.clone(),
        context_id: snapshot.context.id.clone(),
        definition_id: DefinitionId("definition:00000".into()),
        direction: Direction::Outgoing,
        depth: 2,
        max_nodes: 200,
        max_edges: 500,
    }
}

#[test]
fn search_paginates_without_duplicates_and_binds_all_scopes() {
    let (_temp, engine, snapshot) = engine(25);
    let first = engine
        .search(&snapshot.id, &snapshot.context.id, "SYMBOL", 7, None)
        .unwrap();
    assert_eq!(first.items.len(), 7);
    assert!(first.page.truncated);
    let cursor = first.page.next_cursor.clone().unwrap();
    assert_eq!(
        engine
            .search(
                &snapshot.id,
                &snapshot.context.id,
                "different",
                7,
                Some(&cursor)
            )
            .unwrap_err()
            .code(),
        "invalid_cursor"
    );
    let scoped = engine
        .clone()
        .with_repositories([snapshot.repository_id.clone()]);
    assert_eq!(
        scoped
            .search(
                &snapshot.id,
                &snapshot.context.id,
                "symbol",
                7,
                Some(&cursor)
            )
            .unwrap_err()
            .code(),
        "invalid_cursor"
    );
    let mut ids = first.items.into_iter().map(|d| d.id).collect::<Vec<_>>();
    let mut next = Some(cursor);
    while let Some(cursor) = next {
        let page = engine
            .search(
                &snapshot.id,
                &snapshot.context.id,
                "symbol",
                7,
                Some(&cursor),
            )
            .unwrap();
        ids.extend(page.items.into_iter().map(|d| d.id));
        next = page.page.next_cursor;
    }
    assert_eq!(ids.len(), 25);
    assert_eq!(ids.into_iter().collect::<BTreeSet<_>>().len(), 25);
}

#[test]
fn authenticated_cursors_reject_tampering_expiry_and_new_process_keys() {
    let (_temp, engine, snapshot) = engine(3);
    let first = engine
        .search(&snapshot.id, &snapshot.context.id, "", 1, None)
        .unwrap();
    let cursor = first.page.next_cursor.unwrap();
    let mut bytes = cursor.clone().into_bytes();
    bytes[10] = if bytes[10] == b'0' { b'1' } else { b'0' };
    assert_eq!(
        engine
            .search(
                &snapshot.id,
                &snapshot.context.id,
                "",
                1,
                Some(std::str::from_utf8(&bytes).unwrap())
            )
            .unwrap_err()
            .code(),
        "invalid_cursor"
    );
    let mut decoded = cursor::decode(&cursor, &engine.secret).unwrap();
    decoded.expires = cursor::now() - 1;
    let expired = cursor::encode(&decoded, &engine.secret).unwrap();
    assert_eq!(
        engine
            .search(&snapshot.id, &snapshot.context.id, "", 1, Some(&expired))
            .unwrap_err()
            .code(),
        "expired_cursor"
    );
    let other = QueryEngine::new(engine.store.clone()).unwrap();
    assert_eq!(
        other
            .search(&snapshot.id, &snapshot.context.id, "", 1, Some(&cursor))
            .unwrap_err()
            .code(),
        "invalid_cursor"
    );
}

#[test]
fn authorization_happens_before_fact_lookup_and_context_is_pinned() {
    let (_temp, engine, snapshot) = engine(3);
    let denied = engine.clone().with_repositories([]);
    assert!(denied.snapshots().unwrap().is_empty());
    assert_eq!(
        denied.snapshot(&snapshot.id).unwrap_err().code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .snapshot(&SnapshotId("missing".into()))
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .search(&snapshot.id, &snapshot.context.id, "", 10, None)
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        denied.neighborhood(&graph(&snapshot)).unwrap_err().code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .source(
                &snapshot.id,
                &snapshot.context.id,
                &FileId("file:test".into()),
                1,
                1
            )
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .definition(
                &snapshot.id,
                &snapshot.context.id,
                &DefinitionId("missing".into())
            )
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .evidence(
                &snapshot.id,
                &snapshot.context.id,
                &EvidenceId("missing".into())
            )
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .flow(
                &snapshot.id,
                &snapshot.context.id,
                &DefinitionId("missing".into()),
                "source"
            )
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        denied
            .diff(&DiffRequest {
                before: snapshot.id.clone(),
                after: snapshot.id.clone()
            })
            .unwrap_err()
            .code(),
        "not_authorized"
    );
    assert_eq!(
        engine
            .search(&snapshot.id, &ContextId("wrong".into()), "", 10, None)
            .unwrap_err()
            .code(),
        "context_mismatch"
    );
}

#[test]
fn source_preserves_crlf_unicode_and_checks_byte_boundaries() {
    let (_temp, engine, snapshot) = engine(1);
    let file = FileId("file:test".into());
    let first = engine
        .source(&snapshot.id, &snapshot.context.id, &file, 1, 1)
        .unwrap();
    assert_eq!(first.text, "fn root() {}\r\n");
    assert_eq!(first.start_byte, 0);
    assert!(first.truncated);
    let second = engine
        .source(&snapshot.id, &snapshot.context.id, &file, 2, 1)
        .unwrap();
    assert_eq!(second.text, "// \u{03bb}\u{1f600}\r\n");
    assert_eq!(second.start_byte, 14);
    assert_eq!(second.total_lines, 3);
    let selected = engine
        .source_at_byte(&snapshot.id, &snapshot.context.id, &file, 19, 1)
        .unwrap();
    assert_eq!(selected.start_line, 2);
    assert!(
        engine
            .source_at_byte(&snapshot.id, &snapshot.context.id, &file, 18, 1)
            .is_err()
    );
    assert!(
        engine
            .source(&snapshot.id, &snapshot.context.id, &file, 0, 1)
            .is_err()
    );
    assert!(
        engine
            .source(&snapshot.id, &snapshot.context.id, &file, 1, 1001)
            .is_err()
    );
}

#[test]
fn graph_enforces_high_degree_budgets_and_cancellation_is_partial() {
    let (_temp, engine, snapshot) = engine(600);
    let mut request = graph(&snapshot);
    request.max_nodes = 12;
    request.max_edges = 8;
    let result = engine.neighborhood(&request).unwrap();
    assert_eq!(result.edges.len(), 8);
    assert!(result.nodes.len() <= 12);
    assert!(result.page.truncated);
    assert_eq!(result.coverage.status, Status::Partial);
    let nodes: BTreeSet<_> = result.nodes.iter().map(|d| &d.id).collect();
    for edge in &result.edges {
        assert!(nodes.contains(&edge.source));
        if let Target::Resolved { id } = &edge.target {
            assert!(nodes.contains(id));
        }
    }
    let control = QueryControl::new(Duration::from_secs(1));
    control.cancel();
    let result = engine
        .neighborhood_with_control(&request, &control)
        .unwrap();
    assert!(result.work.deadline_reached);
    assert!(result.page.truncated);
    assert_eq!(result.coverage.status, Status::Partial);
}

#[test]
fn unknown_edges_and_missing_flows_never_claim_complete_answers() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let mut facts = batch(2);
    facts.relations[0].target = Target::Unknown {
        reason: UnknownReason::IndirectTargetUnknown,
        label: "callback".into(),
    };
    let snapshot = store.publish(&facts, "main", None).unwrap();
    let engine = QueryEngine::new(store).unwrap();
    let result = engine.neighborhood(&graph(&snapshot)).unwrap();
    assert_eq!(result.edges.len(), 1);
    assert_eq!(result.coverage.status, Status::Partial);
    let flow = engine
        .flow(
            &snapshot.id,
            &snapshot.context.id,
            &facts.definitions[0].id,
            "source",
        )
        .unwrap();
    assert_eq!(flow.coverage.status, Status::Unavailable);
    assert_eq!(
        engine
            .flow(
                &snapshot.id,
                &snapshot.context.id,
                &facts.definitions[0].id,
                "mir"
            )
            .unwrap_err()
            .code(),
        "unsupported_capability"
    );
}

#[test]
fn diff_identifies_changes_and_keeps_duplicate_keys_ambiguous() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let facts = batch(3);
    let before = store.publish(&facts, "main", None).unwrap();
    let mut changed = facts.clone();
    changed.definitions[1].body_hash = "body:changed".into();
    let after = store.publish(&changed, "main", Some(&before.id)).unwrap();
    let engine = QueryEngine::new(store.clone()).unwrap();
    let diff = engine
        .diff(&DiffRequest {
            before: before.id.clone(),
            after: after.id.clone(),
        })
        .unwrap();
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(diff.changes[0].changed_fields, vec!["body"]);
    assert_eq!(diff.impact_candidates.len(), 1);
    let mut ambiguous = changed.clone();
    ambiguous.definitions[2].qualified_name = ambiguous.definitions[1].qualified_name.clone();
    let third = store.publish(&ambiguous, "main", Some(&after.id)).unwrap();
    let diff = engine
        .diff(&DiffRequest {
            before: after.id,
            after: third.id,
        })
        .unwrap();
    assert!(
        diff.changes
            .iter()
            .any(|change| change.correspondence == "ambiguous")
    );
    assert!(
        diff.changes
            .iter()
            .filter(|change| change.correspondence == "ambiguous")
            .all(|change| change.kind != "modified")
    );
}

#[test]
fn giant_source_windows_are_bounded_at_utf8_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let mut facts = batch(1);
    facts.source.files[0].text = "\u{03bb}".repeat(200_000);
    facts.source.files[0].content_hash = digest("content", &facts.source.files[0].text);
    let snapshot = store.publish(&facts, "main", None).unwrap();
    let engine = QueryEngine::new(store).unwrap();
    let source = engine
        .source(
            &snapshot.id,
            &snapshot.context.id,
            &FileId("file:test".into()),
            1,
            1,
        )
        .unwrap();
    assert!(source.truncated);
    assert!(source.text.len() <= SOURCE_BYTES);
    assert!(source.text.ends_with('\u{03bb}'));
}
