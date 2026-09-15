use atlas_model::*;
use atlas_query::{QueryControl, QueryEngine};
use atlas_store::Store;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

fn fixture(definitions: usize, relations: usize) -> FactBatch {
    let context = BuildContext {
        id: ContextId("context:synthetic-index-stress-v1".into()),
        name: "synthetic".into(),
        target: "synthetic-no-compiler".into(),
        features: vec![],
        default_features: false,
        cfg: BTreeMap::new(),
        crates: vec![],
        manifest_digest: "manifest:synthetic".into(),
        trust: "read_only".into(),
        coverage: Coverage::complete(),
    };
    let file_id = FileId("file:synthetic".into());
    let mut text = String::new();
    let mut items = Vec::new();
    for index in 0..definitions {
        let name = format!("f{index:05}");
        let start = text.len() as u32;
        text.push_str(&format!("pub fn {name}() {{}}\n"));
        items.push(Definition {
            id: DefinitionId(format!("definition:{name}")),
            context_id: context.id.clone(),
            file_id: file_id.clone(),
            name: name.clone(),
            qualified_name: format!("synthetic::{name}"),
            kind: "function".into(),
            parent_id: None,
            signature: format!("pub fn {name}()"),
            signature_hash: digest("signature", &name),
            body_hash: "body:empty".into(),
            span: Span {
                file_id: file_id.clone(),
                start,
                end: text.len() as u32,
            },
            body_span: None,
            cfg: vec![],
            cfg_status: CfgStatus::Active,
            visibility: "pub".into(),
            metrics: Metrics::default(),
        });
    }
    let mut edges = Vec::new();
    for index in 0..relations {
        let source = if index < 2000 { 0 } else { index % definitions };
        let target = (index * 7 + 1) % definitions;
        edges.push(Relation {
            id: RelationId(format!("relation:{index:07}")),
            source: items[source].id.clone(),
            target: if index % 97 == 0 {
                Target::Unknown {
                    reason: UnknownReason::IndirectTargetUnknown,
                    label: "synthetic callback".into(),
                }
            } else {
                Target::Resolved {
                    id: items[target].id.clone(),
                }
            },
            kind: "calls".into(),
            span: items[source].span.clone(),
            evidence_id: EvidenceId("evidence:synthetic-generator".into()),
        });
    }
    FactBatch {
        schema_version:SCHEMA_VERSION,
        source:SourceSnapshot {id:SourceId("source:synthetic-index-stress-v1".into()),repository_id:RepositoryId("repository:synthetic".into()),revision:"generator-v1".into(),
            files:vec![SourceFile {id:file_id,path:"synthetic.rs".into(),content_hash:digest("content",&text),text}],manifests:BTreeMap::new(),warnings:vec!["Generated graph facts are not extracted semantic claims about the placeholder source.".into()]},
        context,producer:"atlas-index-stress/v1".into(),definitions:items,relations:edges,
        evidence:vec![Evidence {id:EvidenceId("evidence:synthetic-generator".into()),basis:"synthetic".into(),producer:"atlas-index-stress/v1".into(),inputs:vec![],assumptions:vec![],limitations:vec!["Graph topology is generated; it is not a real repository or frontend benchmark.".into()]}],
        flows:vec![],coverage:Coverage::partial(UnknownReason::IndirectTargetUnknown,"Synthetic topology includes deterministic unknown frontiers."),diagnostics:vec![],
    }
}

fn sample(name: &str, count: usize, mut operation: impl FnMut() -> Result<Value, String>) -> Value {
    let samples: Vec<_> = (0..count)
        .map(|_| {
            let started = Instant::now();
            let result = operation();
            let ms = started.elapsed().as_secs_f64() * 1000.;
            match result {
                Ok(result) => json!({"wall_ms":ms,"ok":true,"result":result}),
                Err(error) => json!({"wall_ms":ms,"ok":false,"error":error}),
            }
        })
        .collect();
    let mut times: Vec<_> = samples
        .iter()
        .map(|s| s["wall_ms"].as_f64().unwrap())
        .collect();
    times.sort_by(f64::total_cmp);
    json!({"name":name,"measurement":"in_process_server_logic","cache":"uncontrolled page cache; first sample included","samples":samples,
        "p50_ms":times[(count as f64*0.5).ceil() as usize-1],"p95_ms":times[(count as f64*0.95).ceil() as usize-1],"p99_ms":null})
}

fn disk_bytes(path: &Path) -> std::io::Result<u64> {
    let mut sum = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.path().symlink_metadata()?;
        if metadata.is_dir() {
            sum += disk_bytes(&entry.path())?;
        } else if metadata.is_file() {
            sum += metadata.len();
        }
    }
    Ok(sum)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: atlas-index-stress NEW_STORE SAMPLES".into());
    }
    let path = Path::new(&args[1]);
    if path.exists() {
        return Err("stress store must not already exist".into());
    }
    let count: usize = args[2].parse()?;
    if !(30..=1000).contains(&count) {
        return Err("samples must be 30..1000".into());
    }
    let generated = Instant::now();
    let batch = fixture(20_000, 80_000);
    let generation_ms = generated.elapsed().as_secs_f64() * 1000.;
    let unknowns = batch
        .relations
        .iter()
        .filter(|edge| matches!(edge.target, Target::Unknown { .. }))
        .count();
    let store = Store::open(path)?;
    let started = Instant::now();
    let snapshot = store.publish(&batch, "synthetic", None)?;
    let publication_ms = started.elapsed().as_secs_f64() * 1000.;
    drop(batch);
    let engine = QueryEngine::new(store.clone())?;
    let request = GraphRequest {
        snapshot_id: snapshot.id.clone(),
        context_id: snapshot.context.id.clone(),
        definition_id: DefinitionId("definition:f00000".into()),
        direction: Direction::Outgoing,
        depth: 1,
        max_nodes: 200,
        max_edges: 500,
    };
    let mut workloads = Vec::new();
    workloads.push(sample("search_prefix", count, || {
        engine
            .search(&snapshot.id, &snapshot.context.id, "f000", 50, None)
            .map(|r| json!({"items":r.items.len(),"partial":r.page.truncated}))
            .map_err(|e| e.code().into())
    }));
    workloads.push(sample("one_hop_high_fanout",count,|| engine.neighborhood(&request)
        .map(|r|json!({"nodes":r.nodes.len(),"edges":r.edges.len(),"partial":r.page.truncated,"deadline":r.work.deadline_reached})).map_err(|e|e.code().into())));
    let mut ring = request.clone();
    ring.definition_id = DefinitionId("definition:f12345".into());
    ring.depth = 2;
    workloads.push(sample("two_hop_ring",count,|| engine.neighborhood(&ring)
        .map(|r|json!({"nodes":r.nodes.len(),"edges":r.edges.len(),"partial":r.page.truncated,"deadline":r.work.deadline_reached})).map_err(|e|e.code().into())));
    workloads.push(sample("source_window", count, || {
        engine
            .source(
                &snapshot.id,
                &snapshot.context.id,
                &FileId("file:synthetic".into()),
                1,
                100,
            )
            .map(|r| json!({"bytes":r.text.len(),"partial":r.truncated}))
            .map_err(|e| e.code().into())
    }));
    let cancelled = QueryControl::new(Duration::from_secs(1));
    cancelled.cancel();
    workloads.push(sample("pre_cancelled_graph",count,|| engine.neighborhood_with_control(&request,&cancelled)
        .map(|r|json!({"nodes":r.nodes.len(),"edges":r.edges.len(),"partial":r.page.truncated,"deadline":r.work.deadline_reached})).map_err(|e|e.code().into())));
    workloads.push(sample("profile_reader_open_checksum", count, || {
        store
            .reader(&snapshot.id)
            .map(|_| json!({}))
            .map_err(|e| e.code().into())
    }));
    let reader = store.reader(&snapshot.id)?;
    workloads.push(sample("profile_already_open_indexed_search", count, || {
        reader
            .search("f000", None, 50)
            .map(|r| json!({"items":r.len()}))
            .map_err(|e| e.code().into())
    }));
    workloads.push(sample("profile_already_open_adjacency", count, || {
        reader
            .adjacency(&request.definition_id, &Direction::Outgoing, 501)
            .map(|r| json!({"edges":r.len()}))
            .map_err(|e| e.code().into())
    }));
    drop(reader);
    let shard = std::fs::read_dir(path.join("shards"))?
        .next()
        .ok_or("missing fixture shard")??;
    let held = path.join("held-shard");
    std::fs::rename(shard.path(), &held)?;
    let missing = engine
        .search(&snapshot.id, &snapshot.context.id, "f000", 50, None)
        .err()
        .map(|e| e.code().to_string());
    std::fs::rename(&held, shard.path())?;
    if missing.is_none() {
        return Err("missing shard incorrectly succeeded".into());
    }
    let output = json!({"kind":"synthetic_indexed_facts","generator":"v1: 20000 definitions, 80000 relations; 2000-site hub, ring links and every97th unknown",
        "frontend_extraction":false,"definitions":snapshot.definition_count,"relations":snapshot.relation_count,"evidence":1,"source_files":1,"facts_count":100002,"unknown_relations":unknowns,
        "generation_ms":generation_ms,"publication_ms":publication_ms,"store_bytes":disk_bytes(path)?,"snapshot_id":snapshot.id,"fact_digest":snapshot.fact_digest,
        "workloads":workloads,"failure_checks":{"missing_shard_error":missing,"live_snapshot_integrity":store.integrity_check()?.valid}});
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
