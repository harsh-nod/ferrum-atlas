# API v1

Canonical Rust types live in `crates/model`; `cargo run -p xtask -- types` generates `web/src/api/types.ts`. `cargo run -p xtask -- check-types` rejects drift. Storage/fact schema and API versions are separate.

All data routes require a session bearer token and an allowed request Host/Origin. Every query pins a snapshot and context. Unknown analysis evidence, pagination truncation, and execution deadline status are independent fields.

| Route | Response |
| --- | --- |
| GET /v1/capabilities | Capabilities |
| GET /v1/snapshots | Snapshot[] |
| GET /v1/snapshots/{id} | Snapshot |
| GET /v1/search?snapshot_id&context_id&q&limit&cursor | QueryResponse&lt;Definition&gt; |
| GET /v1/definitions/{id}?snapshot_id&context_id | DefinitionDetail |
| GET /v1/source/{file}?snapshot_id&context_id&offset&lines | SourceWindow |
| POST /v1/graph/neighborhood | GraphResponse from GraphRequest |
| POST /v1/diff | DiffResponse from DiffRequest |
| GET /v1/flow/{id}?snapshot_id&context_id&phase=source | FunctionFlow |
| GET /v1/evidence/{id}?snapshot_id&context_id | Evidence |
| GET /v1/observations?snapshot_id&context_id | ObservationSummary[] |
| GET /v1/observations/{id}?snapshot_id&context_id&offset&limit | ObservationWindow |

Source windows accept either zero-based UTF-8 byte `offset` or one-based `start_line`. Ranges are half-open and tied to exact content hashes. Browser/LSP UTF-16 positions must be converted from exact text, never treated as byte offsets. Files are bounded before parsing; u32 offsets fit safely into JavaScript numbers. Decimal timestamps stay strings.

Opaque IDs use domain-separated SHA-256 over versioned deterministic inputs. `SourceFile.content_hash` is `digest("content", &text)`. Source bytes preserve CRLF. Exposed IDs are never analyzer IDs or physical database row IDs. Definition identities are snapshot/configuration-local; a diff correspondence is evidence, not an identity guarantee.

Call edges are distinct from runtime order and source-flow points. `Target::Unknown` includes a reason and label. Coverage always states missing information; a bounded empty page does not prove absence of callers. Compiler CFG and runtime observations are not inferred from syntax.

Definition `cfg_status` is `active`, `inactive`, or `unknown`. Inactive definitions remain visible as captured source but do not acquire selected-context behavior. Snapshot `created_at` is decimal epoch seconds, not an ISO timestamp.

Graph depth is 0-4, with at most 200 definition nodes and 500 edges; the viewer also caps rendered unknown boundary nodes within its 200-node total. Query budgets are 250 ms and 2 MiB. Source text windows are capped at 256 KiB before response encoding. Search pages contain at most 200 definitions. Snapshot enumeration rejects overflow beyond 1,000 snapshots or 2 MiB; it does not silently truncate history. Diff rejects inputs above 10,000 definitions or 16 MiB per side rather than inventing changes from independently truncated prefixes.

Imported observations are content-addressed bundles tied to a snapshot, source, context and SHA-256 artifact digest. Test outcomes remain distinct. Timestamp, elapsed, timeout, sequence and loss counters are decimal strings so JavaScript cannot round them. Streams retain clock domains and units; cross-stream order is not synthesized. Observation windows contain at most 200 records. Import is CLI-only; the HTTP API cannot execute or upload work.
