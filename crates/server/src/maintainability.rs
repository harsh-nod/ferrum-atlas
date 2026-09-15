use super::*;
use atlas_analysis::{
    AnalysisControl, MaintainabilityLimits, SourceMaintainability, analyze_maintainability,
};

struct Controls {
    query: QueryControl,
    analysis: AnalysisControl,
}
impl Drop for Controls {
    fn drop(&mut self) {
        self.query.cancel();
        self.analysis.cancel();
    }
}

pub(super) async fn source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(pin): Query<Pinned>,
) -> Result<Json<AnalysisResponse<SourceMaintainability>>, HttpError> {
    let controls = Controls {
        query: QueryControl::new(Duration::from_millis(250)),
        analysis: AnalysisControl::new(Duration::from_secs(2)),
    };
    let query = controls.query.clone();
    let analysis = controls.analysis.clone();
    perform(state, move |q| {
        let detail = q.definition(&pin.snapshot_id, &pin.context_id, &DefinitionId(id))?;
        let source = q.source_for_analysis(
            &pin.snapshot_id,
            &pin.context_id,
            &detail.definition.span.file_id,
            256 * 1024,
            &query,
        )?;
        let report = analyze_maintainability(
            &source,
            &detail.definition,
            MaintainabilityLimits::default(),
            &analysis,
        )
        .map_err(advanced::analysis_error)?;
        Ok(AnalysisResponse {
            api_version: API_VERSION.into(),
            snapshot_id: pin.snapshot_id,
            context_id: pin.context_id,
            analysis: report,
        })
    })
    .await
}
