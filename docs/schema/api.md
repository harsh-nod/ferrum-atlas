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

Source windows accept either zero-based UTF-8 byte `offset` or one-based `start_line`. Ranges are half-open and tied to exact content hashes. Browser/LSP UTF-16 positions must be converted from exact text, never treated as byte offsets. Files are bounded before parsing; u32 offsets fit safely into JavaScript numbers. Decimal timestamps stay strings.

Opaque IDs use domain-separated SHA-256 over versioned deterministic inputs. `SourceFile.content_hash` is `digest("content", &text)`. Source bytes preserve CRLF. Exposed IDs are never analyzer IDs or physical database row IDs. Definition identities are snapshot/configuration-local; a diff correspondence is evidence, not an identity guarantee.

Call edges are distinct from runtime order and source-flow points. `Target::Unknown` includes a reason and label. Coverage always states missing information; a bounded empty page does not prove absence of callers. Compiler CFG and runtime observations are not inferred from syntax.
