use super::*;
use atlas_analysis::{
    AnalysisControl, StateMachineInference, StateMachineLimits, StateTransitionReview,
    infer_state_machine, validate_state_review,
};

type Selection = StateMachineSelection;
type ReviewRequest = StateMachineReviewRequest<StateTransitionReview>;

struct Controls {
    query: QueryControl,
    analysis: AnalysisControl,
}
impl Controls {
    fn new() -> Self {
        Self {
            query: QueryControl::new(Duration::from_millis(250)),
            analysis: AnalysisControl::new(Duration::from_secs(2)),
        }
    }
}
impl Drop for Controls {
    fn drop(&mut self) {
        self.query.cancel();
        self.analysis.cancel();
    }
}

fn infer(
    query: &QueryEngine,
    id: &str,
    selection: &Selection,
    control: &QueryControl,
    analysis: &AnalysisControl,
) -> Result<StateMachineInference, HttpError> {
    let detail = query.definition(
        &selection.snapshot_id,
        &selection.context_id,
        &DefinitionId(id.into()),
    )?;
    let source = query.source_for_analysis(
        &selection.snapshot_id,
        &selection.context_id,
        &detail.definition.span.file_id,
        256 * 1024,
        control,
    )?;
    infer_state_machine(
        &source,
        &detail.definition,
        &selection.enum_path,
        &selection.state_place,
        StateMachineLimits::default(),
        analysis,
    )
    .map_err(advanced::analysis_error)
}

fn reviewable(
    inference: &StateMachineInference,
    control: &AnalysisControl,
) -> Result<(), HttpError> {
    if control.stopped()
        || inference.envelope.truncated
        || inference.envelope.cancelled
        || inference.envelope.deadline_reached
    {
        return Err(HttpError::new(
            StatusCode::REQUEST_TIMEOUT,
            "budget_exhausted",
            "Review requires a non-truncated inference of the selected syntactic scope",
        ));
    }
    Ok(())
}

pub(super) async fn analyze(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(selection): Json<Selection>,
) -> Result<Json<AnalysisResponse<StateMachineInference>>, HttpError> {
    let controls = Controls::new();
    let query = controls.query.clone();
    let analysis = controls.analysis.clone();
    perform(state, move |q| {
        let result = infer(&q, &id, &selection, &query, &analysis)?;
        Ok(AnalysisResponse {
            api_version: API_VERSION.into(),
            snapshot_id: selection.snapshot_id,
            context_id: selection.context_id,
            analysis: result,
        })
    })
    .await
}

pub(super) async fn reviews(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(selection): Query<Selection>,
) -> Result<Json<Vec<StateTransitionReview>>, HttpError> {
    let root = state.observations_dir.join("state-reviews");
    let controls = Controls::new();
    let query = controls.query.clone();
    let analysis = controls.analysis.clone();
    perform(state, move |q| {
        let inference = infer(&q, &id, &selection, &query, &analysis)?;
        reviewable(&inference, &analysis)?;
        atlas_evidence::state_machine::list_with_stop(
            &root,
            &selection.snapshot_id,
            &selection.context_id,
            &inference,
            &|| analysis.stopped(),
        )
        .map_err(|_| review_error(&analysis))
    })
    .await
}

pub(super) async fn review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ReviewRequest>,
) -> Result<Json<StateTransitionReview>, HttpError> {
    let root = state.observations_dir.join("state-reviews");
    let controls = Controls::new();
    let query = controls.query.clone();
    let analysis = controls.analysis.clone();
    perform(state, move |q| {
        let inference = infer(&q, &id, &request.selection, &query, &analysis)?;
        reviewable(&inference, &analysis)?;
        validate_state_review(&inference, &request.review).map_err(advanced::analysis_error)?;
        q.set_snapshot_pin(
            &request.selection.snapshot_id,
            &SnapshotPinRequest {
                context_id: request.selection.context_id.clone(),
                name: "state-reviews".into(),
            },
            true,
        )?;
        reviewable(&inference, &analysis)?;
        atlas_evidence::state_machine::record_with_stop(
            &root,
            &request.selection.snapshot_id,
            &request.selection.context_id,
            &inference,
            request.review,
            &|| analysis.stopped(),
        )
        .map_err(|_| review_error(&analysis))
    })
    .await
}

fn review_error(control: &AnalysisControl) -> HttpError {
    if control.stopped() {
        return HttpError::new(
            StatusCode::REQUEST_TIMEOUT,
            "budget_exhausted",
            "State review cancelled or deadline reached",
        );
    }
    HttpError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "unavailable_evidence",
        "State transition review storage is unavailable or exceeds its limit",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_reinference_is_budget_exhaustion_not_invalid_evidence() {
        let mut inference = StateMachineInference {
            envelope: atlas_analysis::AnalysisEnvelope {
                algorithm_version: "test".into(),
                coverage: Coverage::partial(UnknownReason::SyntaxOnly, "Syntax only"),
                truncated: false,
                cancelled: false,
                deadline_reached: false,
                assumptions: vec![],
            },
            input_digest: "input:test".into(),
            definition_id: DefinitionId("definition:test".into()),
            enum_path: "State".into(),
            state_place: "state".into(),
            body_span: Span {
                file_id: FileId("file:test".into()),
                start: 0,
                end: 0,
            },
            visited_nodes: 0,
            candidates: vec![],
            unknowns: vec![],
        };
        let control = AnalysisControl::new(Duration::from_secs(2));
        assert!(reviewable(&inference, &control).is_ok());
        for (truncated, cancelled, deadline) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            inference.envelope.truncated = truncated;
            inference.envelope.cancelled = cancelled;
            inference.envelope.deadline_reached = deadline;
            let error = reviewable(&inference, &control).unwrap_err();
            assert_eq!(error.status, StatusCode::REQUEST_TIMEOUT);
        }
    }

    #[test]
    fn cancelled_review_storage_reports_budget_not_corruption() {
        let control = AnalysisControl::new(Duration::from_secs(2));
        control.cancel();
        assert_eq!(review_error(&control).status, StatusCode::REQUEST_TIMEOUT);
    }
}
