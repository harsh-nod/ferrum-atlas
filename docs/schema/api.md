# API v1

Canonical Rust types live in `crates/model`; `cargo run -p xtask -- types` generates `web/src/api/types.ts`. `cargo run -p xtask -- check-types` rejects drift. Storage/fact schema and API versions are separate.

All data routes require a session bearer token and an allowed request Host/Origin. Every query pins a snapshot and context. Unknown analysis evidence, pagination truncation, and execution deadline status are independent fields.

| Route | Response |
| --- | --- |
| GET /v1/capabilities | Capabilities |
| GET /v1/snapshots | Snapshot[] |
| GET /v1/snapshots/{id} | Snapshot |
| POST /v1/snapshots/{id}/prepare?context_id | SnapshotPreparation; bounded cold verification |
| POST /v1/snapshots/{id}/pin or /unpin | SnapshotPinState from SnapshotPinRequest |
| GET /v1/search?snapshot_id&context_id&q&limit&cursor | QueryResponse&lt;Definition&gt; |
| GET /v1/definitions/{id}?snapshot_id&context_id | DefinitionDetail |
| GET /v1/source/{file}?snapshot_id&context_id&offset&lines | SourceWindow |
| POST /v1/graph/neighborhood | GraphResponse from GraphRequest |
| POST /v1/graph/analysis | AnalysisResponse&lt;GraphAnalysis&gt; from GraphRequest |
| POST /v1/graph/path | AnalysisResponse&lt;PathAnalysis&gt; from PathRequest |
| POST /v1/queries/impact | GraphResponse; bounded incoming direct-call candidates |
| POST /v1/diff | DiffResponse from DiffRequest |
| GET /v1/flow/{id}?snapshot_id&context_id&phase=source | FunctionFlow |
| GET /v1/bodies/{id}/flow?snapshot_id&context_id&phase=source | FunctionFlow alias |
| GET /v1/compiler?snapshot_id&context_id | CompilerImportSummary[] |
| GET /v1/compiler/bodies/{id}?snapshot_id&context_id&import_id&offset&limit | CompilerFlowPage |
| GET /v1/compiler/bodies/{id}/dataflow?snapshot_id&context_id&import_id | AnalysisResponse&lt;ReachingDefinitions&gt; |
| GET /v1/compiler/bodies/{id}/complexity?snapshot_id&context_id&import_id | AnalysisResponse&lt;CompilerComplexity&gt; |
| GET /v1/analysis/maintainability/{id}?snapshot_id&context_id | AnalysisResponse&lt;SourceMaintainability&gt; |
| GET /v1/evidence/{id}?snapshot_id&context_id | Evidence |
| GET /v1/observations?snapshot_id&context_id | ObservationSummary[] |
| GET /v1/observations/{id}?snapshot_id&context_id&offset&limit | ObservationWindow |
| GET /v1/traces/{id}/window?snapshot_id&context_id&offset&limit | ObservationWindow alias |
| POST /v1/traces/compare | TraceAlignmentResponse&lt;TraceComparison&gt; |
| GET /v1/jobs | JobRecord[]; requires explicit local scheduler enablement |
| POST /v1/jobs | JobRecord from JobRequest |
| GET /v1/jobs/{id} | JobRecord |
| GET /v1/jobs/{id}/events | Authenticated server-sent JobEvent stream |
| POST /v1/jobs/{id}/cancel | JobRecord |

Source windows accept either zero-based UTF-8 byte `offset` or one-based `start_line`. Ranges are half-open and tied to exact content hashes. Browser/LSP UTF-16 positions must be converted from exact text, never treated as byte offsets. Files are bounded before parsing; u32 offsets fit safely into JavaScript numbers. Decimal timestamps stay strings.

Opaque IDs use domain-separated SHA-256 over versioned deterministic inputs. `SourceFile.content_hash` is `digest("content", &text)`. Source bytes preserve CRLF. Exposed IDs are never analyzer IDs or physical database row IDs. Definition identities are snapshot/configuration-local; a diff correspondence is evidence, not an identity guarantee.

Call edges are distinct from runtime order and source-flow points. `Target::Unknown` includes a reason and label. Coverage always states missing information; a bounded empty page does not prove absence of callers. Compiler CFG and runtime observations are not inferred from syntax.

Definition `cfg_status` is `active`, `inactive`, or `unknown`. Inactive definitions remain visible as captured source but do not acquire selected-context behavior. Snapshot `created_at` is decimal epoch seconds, not an ISO timestamp.

Graph depth is 0-4, with at most 200 definition nodes and 500 edges; the viewer also caps rendered unknown boundary nodes within its 200-node total. Interactive graph/search budgets are 250 ms; diff gets 2 seconds. API responses are capped at 2 MiB. Source text windows are capped at 256 KiB before response encoding. Search pages contain at most 200 definitions. Snapshot enumeration rejects overflow beyond 1,000 snapshots or 2 MiB; it does not silently truncate history. Diff rejects inputs above 10,000 definitions or 16 MiB per side rather than inventing changes from independently truncated prefixes.

Imported observations are content-addressed bundles tied to a snapshot, source, context and SHA-256 artifact digest. Test outcomes remain distinct. Timestamp, elapsed, timeout, sequence and loss counters are decimal strings so JavaScript cannot round them. Streams retain clock domains and units; cross-stream order is not synthesized. Observation windows contain at most 200 records. Evidence and compiler imports are CLI-only; the HTTP API accepts no artifact uploads or arbitrary commands. An explicitly enabled scheduler can index only its registered local workspace.

Compiler flow pages expose at most 200 blocks from a verified imported body. Its
producer, input manifest, phase and panic strategy remain attached to each page.
This named-phase CFG is separate from source flow points and runtime traces.
Type display strings are not machine-readable type semantics.

Compiler list, flow, dataflow and complexity requests share cooperative evidence
loading controls: bounded directory/read chunks, canonical hash writes, mapping
scans and page construction check cancellation. Cancelled/deadline-stopped loads
return `408 budget_exhausted`, not a missing artifact or an empty complete list.
The JSON parser itself is byte-bounded to a 33 MiB import record and checked
before/after parsing; it is not individually preemptible or an OS time limit.

Compiler dataflow is opt-in, intraprocedural, whole-local reaching definitions
over one complete imported body of at most 200 blocks. The solver has a two-second
analysis deadline and independent local, fact, iteration and response limits.
Call-return assignments propagate only along normal-return edges. Unknown calls,
pointers, aliases and suspension boundaries remain explicitly uncertain. No
def-use claims are emitted before convergence; truncated output after convergence
remains marked partial. `may_be_uninitialized` is an abstract tracking gap, not a
Rust undefined-behavior finding. There is no interprocedural value/alias solver.

Selected graph algorithms operate only on their bounded input graph. SCC and
path results carry algorithm versions, assumptions and unknown frontiers. A
missing selected path is not proof of global unreachability. Trace comparisons
authorize both snapshot/context pairs, preserve independent clock domains and
match only unambiguous common correlation anchors. First observed divergence is
not a root-cause determination.

Snapshot pin requests contain `context_id` and a `name` of 1..128 ASCII letters,
digits, hyphens or underscores. Storage keys are derived from snapshot, context
and name before mutation. Both operations are idempotent; removing one scoped
pin does not remove another snapshot's pin or a CLI pin. Pin admission is bounded.

Job SSE streams accept `Last-Event-ID`, close at terminal status and have an
eight-stream admission limit. The private durable journal retains 128 jobs and
32 events per job. A service restart marks interrupted work failed, never silently
replays it. Unavailable, overloaded, cancelled and failed states remain distinct.

Preparing a snapshot verifies existing immutable bytes with a separate 30-second
deadline and the same query admission limit. It never captures or analyzes source
or runs a toolchain. Interactive deadlines are not extended to hide cold checksum
cost. Clients may prepare a selected snapshot before fact queries; corrupt or
changed files still fail subsequent reads, and readiness is not a retention pin.

## State Transition Review

`POST /v1/analysis/state-machine/:definition` accepts `StateMachineSelection`
and returns `AnalysisResponse<StateMachineInference>`. It is opt-in, syntax-only,
bounded to one exact function in a complete source file of at most 256 KiB.
Candidates and unknowns never imply an exhaustive machine or semantic resolution.

`GET /v1/analysis/state-machine/:definition/reviews` takes the selection as query
parameters. `POST` to the same path accepts
`StateMachineReviewRequest<StateTransitionReview>`. Both recompute the selected
inference. A cancelled, deadline-limited or truncated re-inference returns
`408 budget_exhausted`, not an assertion that saved evidence is corrupt or absent.
Accepted/rejected declarations bind exact candidate and input digests, persist
separately, and do not upgrade coverage. Authentication and snapshot/context
authorization precede every fact or review read. See
[state-machine operations](../operations/state-machines.md) for limits and pins.

## Maintainability

The opt-in source report uses the exact authorized function and a complete
verified source file up to 256 KiB. Versioned lexical and AST measures carry
source spans, an input digest and limitations. `metrics: null` means counting
did not complete; it must not be rendered as zero. `locations_truncated` can
accompany completed counts and caps only the inspectable location records.
The [metric contract](../operations/maintainability.md) documents scope, nesting
rules, resource limits and missing analyses.

Compiler complexity requires a complete imported body, at most 200 blocks and
2,000 normalized edges. It reports `E - N + 2P` for the explicitly documented
entry-reachable runtime successor multigraph and single synthetic-exit convention.
The named compiler phase, identity, import and input digest remain attached.
Imaginary edges do not become executable paths. Absent normalized exit endpoints
or incomplete computation withhold `metrics`; no zero or source-flow substitute
is supplied. Neither this measure nor runtime reachability proves feasibility,
termination or safety.
