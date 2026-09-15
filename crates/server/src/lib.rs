//! Authenticated, bounded local HTTP queries over immutable snapshots.
mod advanced;
mod state_machine;
use atlas_model::*;
use atlas_query::{QueryControl, QueryEngine};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use tower_http::services::{ServeDir, ServeFile};

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub token: String,
    pub web_dir: PathBuf,
    pub max_concurrent_queries: usize,
    pub observations_dir: PathBuf,
    pub compiler_dir: PathBuf,
    pub scheduler: Option<atlas_scheduler::Scheduler>,
}

impl ServerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.listen.ip().is_loopback(),
            "only loopback binding is supported"
        );
        anyhow::ensure!(
            self.token.len() >= 32 && self.token.bytes().all(|b| b.is_ascii_alphanumeric()),
            "session token must contain at least 32 alphanumeric characters"
        );
        anyhow::ensure!(
            (1..=64).contains(&self.max_concurrent_queries),
            "query concurrency must be between 1 and 64"
        );
        Ok(())
    }
}

pub fn session_token() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        api_version: API_VERSION.into(),
        schema_version: SCHEMA_VERSION,
        analysis_levels: vec!["syntax".into(), "semantic".into(), "compiler_import".into()],
        graph_families: vec!["calls".into(), "source_flow".into(), "compiler_cfg".into(), "selected_components".into(), "trace_alignment".into()],
        trust_modes: vec!["read_only".into()],
        limitations: vec![
            "Local captured source only; missing dependencies and build outputs reduce coverage."
                .into(),
            "Compiler MIR requires an explicitly imported compatible bundle; executable analysis and hosted multi-user service are unavailable."
                .into(),
            "Source-flow points do not represent a complete compiler control-flow graph.".into(),
            "Large-corpus performance and human comprehension targets are not yet qualified."
                .into(),
        ],
    }
}

#[derive(Clone)]
struct AppState {
    engine: QueryEngine,
    token: Arc<String>,
    hosts: Arc<Vec<String>>,
    permits: Arc<Semaphore>,
    observations_dir: Arc<PathBuf>,
    compiler_dir: Arc<PathBuf>,
    scheduler: Option<atlas_scheduler::Scheduler>,
    event_streams: Arc<Semaphore>,
}

pub fn router(engine: QueryEngine, config: &ServerConfig) -> anyhow::Result<Router> {
    config.validate()?;
    let state = AppState {
        engine,
        token: Arc::new(config.token.clone()),
        hosts: Arc::new(vec![
            config.listen.to_string(),
            format!("localhost:{}", config.listen.port()),
        ]),
        permits: Arc::new(Semaphore::new(config.max_concurrent_queries)),
        observations_dir: Arc::new(config.observations_dir.clone()),
        compiler_dir: Arc::new(config.compiler_dir.clone()),
        scheduler: config.scheduler.clone(),
        event_streams: Arc::new(Semaphore::new(8)),
    };
    let api = Router::new()
        .route("/capabilities", get(|| async { Json(capabilities()) }))
        .route("/snapshots", get(snapshots))
        .route("/snapshots/{id}", get(snapshot))
        .route("/snapshots/{id}/pin", post(advanced::pin_snapshot))
        .route("/snapshots/{id}/unpin", post(advanced::unpin_snapshot))
        .route("/snapshots/{id}/prepare", post(advanced::prepare_snapshot))
        .route("/search", get(search))
        .route("/definitions/{id}", get(definition))
        .route("/source/{id}", get(source))
        .route("/graph/neighborhood", post(neighborhood))
        .route("/graph/analysis", post(advanced::graph_analysis))
        .route("/graph/path", post(advanced::graph_path))
        .route("/analysis/state-machine/{id}", post(state_machine::analyze))
        .route(
            "/analysis/state-machine/{id}/reviews",
            get(state_machine::reviews).post(state_machine::review),
        )
        .route("/queries/impact", post(advanced::impact))
        .route("/traces/compare", post(advanced::trace_compare))
        .route("/compiler", get(advanced::compiler_imports))
        .route("/compiler/bodies/{id}", get(advanced::compiler_flow))
        .route(
            "/compiler/bodies/{id}/dataflow",
            get(advanced::compiler_dataflow),
        )
        .route("/diff", post(diff))
        .route("/flow/{id}", get(flow))
        .route("/bodies/{id}/flow", get(flow))
        .route("/evidence/{id}", get(evidence))
        .route("/observations", get(observations))
        .route("/observations/{id}", get(observation_window))
        .route("/traces/{id}/window", get(observation_window))
        .route("/jobs", get(jobs).post(submit_job))
        .route("/jobs/{id}", get(job))
        .route("/jobs/{id}/events", get(job_events))
        .route("/jobs/{id}/cancel", post(cancel_job))
        .fallback(|| async {
            HttpError::new(StatusCode::NOT_FOUND, "not_found", "Unknown API endpoint")
        })
        .layer(DefaultBodyLimit::max(16 * 1024))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let assets = ServeDir::new(&config.web_dir)
        .not_found_service(ServeFile::new(config.web_dir.join("index.html")));
    Ok(Router::new()
        .nest("/v1", api)
        .fallback_service(assets)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, browser_boundary)))
}

pub async fn serve(engine: QueryEngine, config: ServerConfig) -> anyhow::Result<()> {
    let app = router(engine, &config)?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn authenticate(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !constant_time_equal(supplied.as_bytes(), state.token.as_bytes()) {
        return HttpError::new(
            StatusCode::UNAUTHORIZED,
            "not_authorized",
            "A valid session token is required",
        )
        .into_response();
    }
    next.run(req).await
}

fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0_u8, |different, (a, b)| different | (a ^ b))
        == 0
}

async fn browser_boundary(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if req
        .uri()
        .path_and_query()
        .is_some_and(|uri| uri.as_str().len() > 8192)
    {
        return HttpError::new(
            StatusCode::URI_TOO_LONG,
            "invalid_query",
            "The request URI exceeds 8192 bytes",
        )
        .into_response();
    }
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    if !host.is_some_and(|h| state.hosts.iter().any(|allowed| allowed == h)) {
        return HttpError::new(
            StatusCode::FORBIDDEN,
            "invalid_host",
            "Unexpected request host",
        )
        .into_response();
    }
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        let allowed = origin
            .to_str()
            .ok()
            .is_some_and(|origin| state.hosts.iter().any(|h| origin == format!("http://{h}")));
        if !allowed {
            return HttpError::new(
                StatusCode::FORBIDDEN,
                "invalid_origin",
                "Cross-origin requests are not allowed",
            )
            .into_response();
        }
    }
    if req
        .headers()
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return HttpError::new(
            StatusCode::FORBIDDEN,
            "invalid_origin",
            "Cross-site requests are not allowed",
        )
        .into_response();
    }
    let mut response = next.run(req).await;
    if (response.status().is_client_error() || response.status().is_server_error())
        && !response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.to_str().unwrap_or("").starts_with("application/json"))
    {
        response = HttpError::new(
            response.status(),
            "invalid_request",
            "The request could not be accepted",
        )
        .into_response();
    }
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; worker-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"));
    response
}

struct HttpError {
    status: StatusCode,
    payload: ApiError,
}
impl HttpError {
    fn new(status: StatusCode, code: &str, message: &str) -> Self {
        Self {
            status,
            payload: ApiError {
                code: code.into(),
                message: message.into(),
                correlation_id: format!("request:{}", REQUEST_ID.fetch_add(1, Ordering::Relaxed)),
            },
        }
    }
}
impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (self.status, Json(self.payload)).into_response()
    }
}
impl From<atlas_query::Error> for HttpError {
    fn from(error: atlas_query::Error) -> Self {
        let code = error.code();
        let (status, message) = match code {
            "not_authorized" => (StatusCode::FORBIDDEN, "This repository is not authorized"),
            "unknown_snapshot" | "not_found" => (
                StatusCode::NOT_FOUND,
                "The requested snapshot or fact is unavailable",
            ),
            "context_mismatch" => (
                StatusCode::CONFLICT,
                "The context does not belong to this snapshot",
            ),
            "unsupported_capability" => (
                StatusCode::NOT_IMPLEMENTED,
                "The requested analysis capability is unavailable",
            ),
            "budget_exhausted" | "cancelled" => (
                StatusCode::REQUEST_TIMEOUT,
                "The query exceeded its work budget",
            ),
            "unavailable_shard" => (
                StatusCode::SERVICE_UNAVAILABLE,
                "Snapshot data failed an integrity or availability check",
            ),
            "invalid_query" | "expired_cursor" | "invalid_cursor" => (
                StatusCode::BAD_REQUEST,
                "The query or cursor is invalid or expired",
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "The query could not be completed",
            ),
        };
        Self::new(status, code, message)
    }
}

async fn execute<T, F>(state: AppState, operation: F) -> Result<Json<T>, HttpError>
where
    T: Serialize + Send + 'static,
    F: FnOnce(QueryEngine) -> atlas_query::Result<T> + Send + 'static,
{
    perform(state, move |engine| operation(engine).map_err(Into::into)).await
}

async fn perform<T, F>(state: AppState, operation: F) -> Result<Json<T>, HttpError>
where
    T: Serialize + Send + 'static,
    F: FnOnce(QueryEngine) -> Result<T, HttpError> + Send + 'static,
{
    perform_with_timeout(state, operation, Duration::from_secs(3)).await
}

async fn perform_with_timeout<T, F>(
    state: AppState,
    operation: F,
    timeout: Duration,
) -> Result<Json<T>, HttpError>
where
    T: Serialize + Send + 'static,
    F: FnOnce(QueryEngine) -> Result<T, HttpError> + Send + 'static,
{
    let permit = state.permits.clone().try_acquire_owned().map_err(|_| {
        HttpError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "overloaded",
            "All query slots are busy",
        )
    })?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let result = operation(state.engine)?;
        let mut budget = ResponseBudget(2 * 1024 * 1024);
        serde_json::to_writer(&mut budget, &result).map_err(|_| {
            HttpError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "budget_exhausted",
                "The response exceeds the 2 MiB limit",
            )
        })?;
        Ok(result)
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result.map(Json),
        Ok(Err(_)) => Err(HttpError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "query_failed",
            "The query worker failed",
        )),
        Err(_) => Err(HttpError::new(
            StatusCode::REQUEST_TIMEOUT,
            "budget_exhausted",
            "The query exceeded its deadline",
        )),
    }
}

struct ResponseBudget(usize);
impl std::io::Write for ResponseBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::other("response budget exhausted"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Deserialize)]
struct Pinned {
    snapshot_id: SnapshotId,
    context_id: ContextId,
}
#[derive(Deserialize)]
struct SearchParams {
    snapshot_id: SnapshotId,
    context_id: ContextId,
    #[serde(default)]
    q: String,
    #[serde(default = "fifty")]
    limit: usize,
    cursor: Option<String>,
}
fn fifty() -> usize {
    50
}
#[derive(Deserialize)]
struct SourceParams {
    snapshot_id: SnapshotId,
    context_id: ContextId,
    start_line: Option<u32>,
    offset: Option<u32>,
    lines: Option<u32>,
}
#[derive(Deserialize)]
struct FlowParams {
    snapshot_id: SnapshotId,
    context_id: ContextId,
    phase: Option<String>,
}

async fn snapshots(State(state): State<AppState>) -> Result<Json<Vec<Snapshot>>, HttpError> {
    execute(state, |q| q.snapshots()).await
}

fn scheduler(state: &AppState) -> Result<atlas_scheduler::Scheduler, HttpError> {
    state.scheduler.clone().ok_or_else(|| {
        HttpError::new(
            StatusCode::NOT_IMPLEMENTED,
            "unsupported_capability",
            "Analysis jobs are disabled for this server",
        )
    })
}
fn job_error(_: anyhow::Error) -> HttpError {
    HttpError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "analysis_unavailable",
        "The analysis queue is full or unavailable",
    )
}
async fn jobs(State(state): State<AppState>) -> Result<Json<Vec<JobRecord>>, HttpError> {
    let scheduler = scheduler(&state)?;
    perform(state, move |_| scheduler.list().map_err(job_error)).await
}
async fn submit_job(
    State(state): State<AppState>,
    Json(request): Json<JobRequest>,
) -> Result<Json<JobRecord>, HttpError> {
    atlas_scheduler::validate_request(&request).map_err(|_| {
        HttpError::new(
            StatusCode::BAD_REQUEST,
            "invalid_query",
            "Invalid read-only analysis request",
        )
    })?;
    let scheduler = scheduler(&state)?;
    perform(state, move |_| scheduler.submit(request).map_err(job_error)).await
}
async fn job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobRecord>, HttpError> {
    let scheduler = scheduler(&state)?;
    perform(state, move |_| {
        scheduler
            .get(&id)
            .map_err(|_| HttpError::new(StatusCode::NOT_FOUND, "not_found", "Unknown analysis job"))
    })
    .await
}
async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobRecord>, HttpError> {
    let scheduler = scheduler(&state)?;
    perform(state, move |_| {
        scheduler.get(&id).map_err(|_| {
            HttpError::new(StatusCode::NOT_FOUND, "not_found", "Unknown analysis job")
        })?;
        scheduler.cancel(&id).map_err(job_error)
    })
    .await
}
async fn job_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<
    axum::response::Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    HttpError,
> {
    let scheduler = scheduler(&state)?;
    let after = match headers.get("last-event-id") {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or_else(|| {
                HttpError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_query",
                    "Invalid event sequence",
                )
            })?,
        None => 0,
    };
    let permit = state
        .event_streams
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            HttpError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "overloaded",
                "All progress stream slots are busy",
            )
        })?;
    event_job(scheduler.clone(), id.clone()).await?;
    let stream = futures_util::stream::unfold(
        (scheduler, id, after, permit),
        |(scheduler, id, after, permit)| async move {
            loop {
                let job = event_job(scheduler.clone(), id.clone()).await.ok()?;
                if let Some(event) = job.events.iter().find(|event| event.sequence > after) {
                    let sequence = event.sequence;
                    let event = axum::response::sse::Event::default()
                        .event("progress")
                        .id(sequence.to_string())
                        .json_data(event)
                        .ok()?;
                    return Some((Ok(event), (scheduler, id, sequence, permit)));
                }
                if job.status.terminal() {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        },
    );
    Ok(axum::response::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

async fn event_job(
    scheduler: atlas_scheduler::Scheduler,
    id: String,
) -> Result<JobRecord, HttpError> {
    let task = tokio::task::spawn_blocking(move || scheduler.get(&id));
    match tokio::time::timeout(Duration::from_secs(3), task).await {
        Ok(Ok(Ok(record))) => Ok(record),
        Ok(Ok(Err(_))) => Err(HttpError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Unknown analysis job",
        )),
        _ => Err(HttpError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "analysis_unavailable",
            "Progress is temporarily unavailable",
        )),
    }
}
async fn snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Snapshot>, HttpError> {
    execute(state, move |q| q.snapshot(&SnapshotId(id))).await
}
async fn search(
    State(state): State<AppState>,
    Query(p): Query<SearchParams>,
) -> Result<Json<QueryResponse<Definition>>, HttpError> {
    execute(state, move |q| {
        q.search(
            &p.snapshot_id,
            &p.context_id,
            &p.q,
            p.limit,
            p.cursor.as_deref(),
        )
    })
    .await
}
async fn definition(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(p): Query<Pinned>,
) -> Result<Json<DefinitionDetail>, HttpError> {
    execute(state, move |q| {
        q.definition(&p.snapshot_id, &p.context_id, &DefinitionId(id))
    })
    .await
}
async fn source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(p): Query<SourceParams>,
) -> Result<Json<SourceWindow>, HttpError> {
    let control = QueryControl::new(Duration::from_millis(250));
    let _guard = CancelOnDrop(control.clone());
    execute(state, move |q| match p.offset {
        Some(offset) => q.source_at_byte_with_control(
            &p.snapshot_id,
            &p.context_id,
            &FileId(id),
            offset,
            p.lines.unwrap_or(200),
            &control,
        ),
        None => q.source_with_control(
            &p.snapshot_id,
            &p.context_id,
            &FileId(id),
            p.start_line.unwrap_or(1),
            p.lines.unwrap_or(200),
            &control,
        ),
    })
    .await
}
struct CancelOnDrop(QueryControl);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
async fn neighborhood(
    State(state): State<AppState>,
    Json(request): Json<GraphRequest>,
) -> Result<Json<GraphResponse>, HttpError> {
    let control = QueryControl::new(Duration::from_millis(250));
    let _guard = CancelOnDrop(control.clone());
    execute(state, move |q| {
        q.neighborhood_with_control(&request, &control)
    })
    .await
}
async fn diff(
    State(state): State<AppState>,
    Json(request): Json<DiffRequest>,
) -> Result<Json<DiffResponse>, HttpError> {
    execute(state, move |q| q.diff(&request)).await
}
async fn flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(p): Query<FlowParams>,
) -> Result<Json<FunctionFlow>, HttpError> {
    execute(state, move |q| {
        q.flow(
            &p.snapshot_id,
            &p.context_id,
            &DefinitionId(id),
            p.phase.as_deref().unwrap_or("source"),
        )
    })
    .await
}
async fn evidence(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(p): Query<Pinned>,
) -> Result<Json<Evidence>, HttpError> {
    execute(state, move |q| {
        q.evidence(&p.snapshot_id, &p.context_id, &EvidenceId(id))
    })
    .await
}

#[derive(Deserialize)]
struct ObservationParams {
    snapshot_id: SnapshotId,
    context_id: ContextId,
    offset: Option<u32>,
    limit: Option<u32>,
}

fn check_observation_scope(
    q: &QueryEngine,
    id: &SnapshotId,
    context: &ContextId,
) -> Result<(), HttpError> {
    let snapshot = q.snapshot(id)?;
    if &snapshot.context.id != context {
        return Err(HttpError::new(
            StatusCode::CONFLICT,
            "context_mismatch",
            "The context does not belong to this snapshot",
        ));
    }
    Ok(())
}

async fn observations(
    State(state): State<AppState>,
    Query(p): Query<Pinned>,
) -> Result<Json<Vec<ObservationSummary>>, HttpError> {
    let root = state.observations_dir.clone();
    perform(state, move |q| {
        check_observation_scope(&q, &p.snapshot_id, &p.context_id)?;
        atlas_evidence::list(&root, &p.snapshot_id).map_err(|_| {
            HttpError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable_evidence",
                "Observation data failed an integrity or availability check",
            )
        })
    })
    .await
}

async fn observation_window(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(p): Query<ObservationParams>,
) -> Result<Json<ObservationWindow>, HttpError> {
    let root = state.observations_dir.clone();
    perform(state, move |q| {
        check_observation_scope(&q, &p.snapshot_id, &p.context_id)?;
        atlas_evidence::window(
            &root,
            &p.snapshot_id,
            &id,
            p.offset.unwrap_or(0),
            p.limit.unwrap_or(100),
        )
        .map_err(|_| {
            HttpError::new(
                StatusCode::BAD_REQUEST,
                "invalid_observation",
                "The observation or requested window is invalid or unavailable",
            )
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn config() -> ServerConfig {
        ServerConfig {
            listen: "127.0.0.1:7878".parse().unwrap(),
            token: "a".repeat(64),
            web_dir: PathBuf::from("missing-web-assets"),
            max_concurrent_queries: 4,
            observations_dir: PathBuf::from("missing-observations"),
            compiler_dir: PathBuf::from("missing-compiler"),
            scheduler: None,
        }
    }
    fn test_app() -> (tempfile::TempDir, Router) {
        let temp = tempfile::tempdir().unwrap();
        let store = atlas_store::Store::open(temp.path()).unwrap();
        let app = router(QueryEngine::new(store).unwrap(), &config()).unwrap();
        (temp, app)
    }
    fn request(uri: &str, token: bool) -> axum::http::request::Builder {
        let request = Request::builder().uri(uri).header("host", "127.0.0.1:7878");
        if token {
            request.header("authorization", format!("Bearer {}", "a".repeat(64)))
        } else {
            request
        }
    }
    #[tokio::test]
    async fn every_data_route_requires_a_token() {
        let (_temp, app) = test_app();
        for path in [
            "/v1/capabilities",
            "/v1/snapshots",
            "/v1/snapshots/missing",
            "/v1/search",
            "/v1/definitions/id",
            "/v1/source/id",
            "/v1/flow/id",
            "/v1/evidence/id",
            "/v1/observations",
            "/v1/observations/id",
            "/v1/jobs",
            "/v1/jobs/id",
            "/v1/jobs/id/events",
            "/v1/bodies/id/flow",
            "/v1/traces/id/window",
            "/v1/compiler",
            "/v1/compiler/bodies/id",
            "/v1/compiler/bodies/id/dataflow",
            "/v1/analysis/state-machine/id/reviews",
        ] {
            let response = app
                .clone()
                .oneshot(request(path, false).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        }
        for path in [
            "/v1/graph/neighborhood",
            "/v1/diff",
            "/v1/jobs",
            "/v1/jobs/id/cancel",
            "/v1/graph/analysis",
            "/v1/graph/path",
            "/v1/queries/impact",
            "/v1/traces/compare",
            "/v1/snapshots/id/pin",
            "/v1/snapshots/id/unpin",
            "/v1/snapshots/id/prepare",
            "/v1/analysis/state-machine/id",
            "/v1/analysis/state-machine/id/reviews",
        ] {
            let response = app
                .clone()
                .oneshot(
                    request(path, false)
                        .method("POST")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        }
    }
    #[tokio::test]
    async fn job_routes_require_explicit_enablement() {
        let (_temp, app) = test_app();
        let response = app
            .oneshot(request("/v1/jobs", true).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn job_events_replay_to_terminal_without_blocking_queries() {
        let temp = tempfile::tempdir().unwrap();
        let store = atlas_store::Store::open(temp.path()).unwrap();
        let mut settings = config();
        settings.scheduler = Some(
            atlas_scheduler::Scheduler::start(
                temp.path(),
                std::path::Path::new("/bin/false"),
                Default::default(),
            )
            .unwrap(),
        );
        let app = router(QueryEngine::new(store).unwrap(), &settings).unwrap();
        let response = app
            .clone()
            .oneshot(
                request("/v1/jobs", true)
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"profile":"default","level":"syntax"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let record: JobRecord = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 16384)
                .await
                .unwrap(),
        )
        .unwrap();
        let response = app
            .clone()
            .oneshot(request("/v1/snapshots", true).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let path = format!("/v1/jobs/{}/events", record.id);
        let response = app
            .clone()
            .oneshot(request(&path, true).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .contains("text/event-stream")
        );
        let body = tokio::time::timeout(
            Duration::from_secs(3),
            axum::body::to_bytes(response.into_body(), 16384),
        )
        .await
        .unwrap()
        .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("event: progress"));
        assert!(text.contains("\"status\":\"failed\""));
        let response = app
            .clone()
            .oneshot(
                request(&path, true)
                    .header("last-event-id", "invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = app
            .oneshot(
                request(&format!("/v1/jobs/{}/cancel", record.id), true)
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    #[tokio::test]
    async fn rejects_foreign_origins_and_rebound_hosts() {
        let (_temp, app) = test_app();
        let foreign = request("/v1/snapshots", true)
            .header("origin", "https://example.invalid")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(foreign).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let rebound = Request::builder()
            .uri("/v1/capabilities")
            .header("host", "example.invalid:7878")
            .header("authorization", format!("Bearer {}", "a".repeat(64)))
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.oneshot(rebound).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn authenticated_empty_store_is_browsable_and_not_cached() {
        let (_temp, app) = test_app();
        let response = app
            .oneshot(request("/v1/snapshots", true).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"[]");
    }
    #[tokio::test]
    async fn api_errors_do_not_fall_back_to_html() {
        let (_temp, app) = test_app();
        let response = app
            .oneshot(request("/v1/absent", true).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .contains("application/json")
        );
    }
    #[tokio::test]
    async fn malformed_and_oversized_requests_return_bounded_json_errors() {
        let (_temp, app) = test_app();
        for (path, body, status) in [
            (
                "/v1/graph/neighborhood",
                "{".to_owned(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/v1/diff",
                " ".repeat(16 * 1024 + 1),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    request(path, true)
                        .method("POST")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            assert!(
                response.headers()["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("application/json")
            );
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            let error: ApiError = serde_json::from_slice(&body).unwrap();
            assert!(!error.correlation_id.is_empty());
            assert_eq!(error.code, "invalid_request");
        }
        let uri = format!("/v1/search?q={}", "a".repeat(8192));
        let response = app
            .oneshot(request(&uri, true).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::URI_TOO_LONG);
    }

    #[tokio::test]
    async fn admission_control_rejects_work_before_running_the_query() {
        let temp = tempfile::tempdir().unwrap();
        let state = AppState {
            engine: QueryEngine::new(atlas_store::Store::open(temp.path()).unwrap()).unwrap(),
            token: Arc::new(config().token),
            hosts: Arc::new(vec![]),
            permits: Arc::new(Semaphore::new(1)),
            observations_dir: Arc::new(temp.path().join("observations")),
            compiler_dir: Arc::new(temp.path().join("compiler")),
            scheduler: None,
            event_streams: Arc::new(Semaphore::new(8)),
        };
        let permit = state.permits.clone().acquire_owned().await.unwrap();
        let result = perform::<Vec<Snapshot>, _>(state.clone(), |_| {
            panic!("overloaded operation must not start")
        })
        .await;
        let error = result.err().unwrap();
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        drop(permit);
        assert!(execute(state, |q| q.snapshots()).await.is_ok());
    }

    #[test]
    fn serialization_budget_accounts_for_json_escaping() {
        let mut budget = ResponseBudget(8);
        assert!(serde_json::to_writer(&mut budget, &"\u{0000}\u{0000}").is_err());
        let mut budget = ResponseBudget(4);
        assert!(serde_json::to_writer(&mut budget, &"ok").is_ok());
        assert_eq!(budget.0, 0);
    }
    #[test]
    fn binding_and_token_policy_are_explicit() {
        let mut c = config();
        c.listen = "0.0.0.0:7878".parse().unwrap();
        assert!(c.validate().is_err());
        c.listen = "127.0.0.1:7878".parse().unwrap();
        c.token = "short".into();
        assert!(c.validate().is_err());
        let a = session_token().unwrap();
        let b = session_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(constant_time_equal(a.as_bytes(), a.as_bytes()));
        assert!(!constant_time_equal(a.as_bytes(), b.as_bytes()));
    }
}
