use super::*;

#[derive(Deserialize)]
pub(super) struct PrepareParams {
    context_id: ContextId,
}

pub(super) async fn prepare_snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<PrepareParams>,
) -> Result<Json<SnapshotPreparation>, HttpError> {
    let control = QueryControl::new(Duration::from_secs(30));
    let _guard = CancelOnDrop(control.clone());
    perform_with_timeout(
        state,
        move |q| Ok(q.prepare_snapshot(&SnapshotId(id), &params.context_id, &control)?),
        Duration::from_secs(30),
    )
    .await
}

pub(super) async fn pin_snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SnapshotPinRequest>,
) -> Result<Json<SnapshotPinState>, HttpError> {
    execute(state, move |q| {
        q.set_snapshot_pin(&SnapshotId(id), &request, true)
    })
    .await
}

pub(super) async fn unpin_snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<SnapshotPinRequest>,
) -> Result<Json<SnapshotPinState>, HttpError> {
    execute(state, move |q| {
        q.set_snapshot_pin(&SnapshotId(id), &request, false)
    })
    .await
}

pub(super) async fn compiler_imports(
    State(state): State<AppState>,
    Query(pin): Query<Pinned>,
) -> Result<Json<Vec<CompilerImportSummary>>, HttpError> {
    let root = state.compiler_dir.clone();
    perform(state, move |q| {
        check_observation_scope(&q, &pin.snapshot_id, &pin.context_id)?;
        atlas_evidence::compiler::list(&root, &pin.snapshot_id, &pin.context_id).map_err(|_| {
            HttpError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable_evidence",
                "Compiler evidence is unavailable or invalid",
            )
        })
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct CompilerWindowParams {
    snapshot_id: SnapshotId,
    context_id: ContextId,
    import_id: String,
    offset: Option<u32>,
    limit: Option<u32>,
}
pub(super) async fn compiler_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<CompilerWindowParams>,
) -> Result<Json<CompilerFlowPage>, HttpError> {
    let root = state.compiler_dir.clone();
    perform(state, move |q| {
        let definition = DefinitionId(id);
        q.definition(&params.snapshot_id, &params.context_id, &definition)?;
        atlas_evidence::compiler::flow(
            &root,
            &params.snapshot_id,
            &params.context_id,
            &definition,
            &params.import_id,
            (params.offset.unwrap_or(0), params.limit.unwrap_or(200)),
        )
        .map_err(|_| {
            HttpError::new(
                StatusCode::NOT_FOUND,
                "unavailable_evidence",
                "No compatible compiler body window is available for this selection",
            )
        })
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DataflowParams {
    snapshot_id: SnapshotId,
    context_id: ContextId,
    import_id: String,
}

pub(super) async fn compiler_dataflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DataflowParams>,
) -> Result<Json<AnalysisResponse<atlas_analysis::ReachingDefinitions>>, HttpError> {
    let root = state.compiler_dir.clone();
    let guard = AnalysisGuard {
        query: QueryControl::new(Duration::from_millis(250)),
        analysis: AnalysisControl::new(Duration::from_secs(2)),
    };
    let control = guard.analysis.clone();
    perform(state, move |q| {
        let definition = DefinitionId(id);
        q.definition(&params.snapshot_id, &params.context_id, &definition)?;
        let page = atlas_evidence::compiler::flow(
            &root,
            &params.snapshot_id,
            &params.context_id,
            &definition,
            &params.import_id,
            (0, 200),
        )
        .map_err(|_| {
            HttpError::new(
                StatusCode::NOT_FOUND,
                "unavailable_evidence",
                "No compatible complete compiler body is available",
            )
        })?;
        if page.next_offset.is_some() {
            return Err(HttpError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "budget_exhausted",
                "Selected dataflow is limited to complete bodies of at most 200 blocks",
            ));
        }
        let analysis = atlas_analysis::reaching_definitions(
            &page.body,
            &page.coverage,
            atlas_analysis::DataflowLimits {
                max_blocks: 200,
                ..Default::default()
            },
            &control,
        )
        .map_err(analysis_error)?;
        Ok(AnalysisResponse {
            api_version: API_VERSION.into(),
            snapshot_id: params.snapshot_id,
            context_id: params.context_id,
            analysis,
        })
    })
    .await
}
use atlas_analysis::{
    AnalysisControl, AnalysisLimits, GraphAnalysis, PathAnalysis, SelectedGraph,
    TraceCompareRequest, TraceComparison,
};

struct AnalysisGuard {
    query: QueryControl,
    analysis: AnalysisControl,
}
impl AnalysisGuard {
    fn new() -> Self {
        Self {
            query: QueryControl::new(Duration::from_millis(250)),
            analysis: AnalysisControl::new(Duration::from_millis(250)),
        }
    }
}
impl Drop for AnalysisGuard {
    fn drop(&mut self) {
        self.query.cancel();
        self.analysis.cancel();
    }
}
pub(super) fn analysis_error(error: atlas_analysis::AnalysisError) -> HttpError {
    match error {
        atlas_analysis::AnalysisError::InvalidInput(_) => HttpError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "Invalid selected analysis input",
        ),
        atlas_analysis::AnalysisError::BudgetExhausted => HttpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "budget_exhausted",
            "Selected analysis exceeds its bounded scope",
        ),
    }
}
fn selected(graph: &GraphResponse) -> SelectedGraph<'_> {
    SelectedGraph {
        nodes: &graph.nodes,
        edges: &graph.edges,
        coverage: &graph.coverage,
        input_truncated: graph.page.truncated || graph.work.deadline_reached,
    }
}

pub(super) async fn graph_analysis(
    State(state): State<AppState>,
    Json(request): Json<GraphRequest>,
) -> Result<Json<AnalysisResponse<GraphAnalysis>>, HttpError> {
    let guard = AnalysisGuard::new();
    let query = guard.query.clone();
    let analysis = guard.analysis.clone();
    perform(state, move |q| {
        let graph = q.neighborhood_with_control(&request, &query)?;
        let result =
            atlas_analysis::analyze_graph(&selected(&graph), AnalysisLimits::default(), &analysis)
                .map_err(analysis_error)?;
        Ok(AnalysisResponse {
            api_version: API_VERSION.into(),
            snapshot_id: graph.snapshot_id,
            context_id: graph.context_id,
            analysis: result,
        })
    })
    .await
}

pub(super) async fn graph_path(
    State(state): State<AppState>,
    Json(request): Json<PathRequest>,
) -> Result<Json<AnalysisResponse<PathAnalysis>>, HttpError> {
    let guard = AnalysisGuard::new();
    let query = guard.query.clone();
    let analysis = guard.analysis.clone();
    perform(state, move |q| {
        if let Some(target) = &request.target {
            q.definition(
                &request.graph.snapshot_id,
                &request.graph.context_id,
                target,
            )?;
        }
        let graph = q.neighborhood_with_control(&request.graph, &query)?;
        let result = atlas_analysis::find_path(
            &selected(&graph),
            &request.graph.definition_id,
            request.target.as_ref(),
            AnalysisLimits::default(),
            &analysis,
        )
        .map_err(analysis_error)?;
        Ok(AnalysisResponse {
            api_version: API_VERSION.into(),
            snapshot_id: graph.snapshot_id,
            context_id: graph.context_id,
            analysis: result,
        })
    })
    .await
}

pub(super) async fn impact(
    State(state): State<AppState>,
    Json(mut request): Json<GraphRequest>,
) -> Result<Json<GraphResponse>, HttpError> {
    request.direction = Direction::Incoming;
    let control = QueryControl::new(Duration::from_millis(250));
    let _guard = CancelOnDrop(control.clone());
    execute(state, move |q| {
        let mut graph = q.neighborhood_with_control(&request, &control)?;
        graph.coverage.limitations.push("Impact consists of bounded selected-snapshot direct-call reachability candidates, not proof of changed behavior or observed execution. Other relation families and unobserved dynamic targets are excluded.".into());
        Ok(graph)
    }).await
}

pub(super) async fn trace_compare(
    State(state): State<AppState>,
    Json(request): Json<TraceAlignmentRequest>,
) -> Result<Json<TraceAlignmentResponse<TraceComparison>>, HttpError> {
    let root = state.observations_dir.clone();
    let guard = AnalysisGuard::new();
    let control = guard.analysis.clone();
    perform(state, move |q| {
        check_observation_scope(&q, &request.before_snapshot_id, &request.before_context_id)?;
        check_observation_scope(&q, &request.after_snapshot_id, &request.after_context_id)?;
        let unavailable = |_| {
            HttpError::new(
                StatusCode::NOT_FOUND,
                "unavailable_evidence",
                "The selected observation is unavailable or invalid",
            )
        };
        let before = atlas_evidence::load(
            &root,
            &request.before_snapshot_id,
            &request.before_observation_id,
        )
        .map_err(unavailable)?;
        let after = atlas_evidence::load(
            &root,
            &request.after_snapshot_id,
            &request.after_observation_id,
        )
        .map_err(unavailable)?;
        let comparison = atlas_analysis::compare_trace_windows(
            &before,
            &after,
            &TraceCompareRequest {
                before_stream_id: request.before_stream_id,
                after_stream_id: request.after_stream_id,
                before_offset: request.before_offset,
                after_offset: request.after_offset,
                max_events: request.max_events,
                max_anchors: request.max_anchors,
            },
            &control,
        )
        .map_err(analysis_error)?;
        Ok(TraceAlignmentResponse {
            before: request.before_snapshot_id,
            after: request.after_snapshot_id,
            before_context: request.before_context_id,
            after_context: request.after_context_id,
            comparison,
        })
    })
    .await
}
