# Rust Code Analyzer and Visualizer

Implementation specification and parallel-agent delivery plan

- Date: 2026-09-14
- Status: proposed design, not an implemented or benchmarked system
- Working name: `rustscope` (placeholder; check naming before distribution)
- Audience: implementation agents, integration lead, Rust engineers, reviewers
- Primary purpose: help humans understand and review large amounts of Rust, including agent-written code, using source-linked structural, semantic, and execution evidence

## Navigation

| Read for | Sections |
| --- | --- |
| Product direction | [Decision](#1-executive-decision), [scope](#2-assumptions-and-non-goals), [human workflows](#3-human-workflows-and-product-acceptance) |
| Performance and architecture | [Scale contract](#4-scale-and-performance-contract), [architecture](#5-system-architecture), [frontends](#6-analysis-frontends-and-build-fidelity) |
| Shared implementation contracts | [Evidence](#7-evidence-model-and-graph-semantics), [identity/schema](#8-identity-schema-and-versioning), [storage](#9-storage-and-publication) |
| Runtime engine | [Incrementality](#10-incremental-analysis-and-scheduling), [APIs](#11-query-engine-and-apis), [algorithms](#12-graph-algorithms-and-bounded-deep-analysis) |
| Human-facing experience | [Viewer](#13-viewer-design), [agent-code review](#14-understanding-agent-written-code), [trace/test evidence](#15-traces-tests-and-firmware-extensions) |
| Qualification | [Security](#16-security-privacy-and-trust-modes), [correctness](#17-correctness-and-regression-strategy), [benchmarks](#18-performance-qualification-and-observability) |
| Swarm execution | [Repository ownership](#19-repository-layout-and-ownership), [workstreams/tasks](#20-parallel-agent-workstreams), [milestones](#21-milestones-and-timelines) |
| Delivery management | [Integration/monitoring](#22-integration-monitoring-and-rerouting), [risks/decisions](#23-risk-register-and-decisions), [release definition](#24-operations-and-release-definition), [kickoff](#25-implementation-kickoff-checklist) |

## 1. Executive Decision

Build a persistent, configuration-aware code intelligence system with a focused interactive viewer. Do not start by writing a new Rust compiler, making a whole-repository force-directed graph, or putting an LLM in every query path.

The system has four deliberately separate layers:

1. **Analysis:** extract syntax, resolved symbols, relationships, and selected compiler-derived facts.
2. **Persistence:** store reusable, versioned evidence and indexes without keeping every repository's semantic model in RAM.
3. **Queries:** return bounded, source-linked answers about a selected symbol, change, path, or subsystem.
4. **Presentation:** coordinate source, graphs, change review, and optional execution traces so humans can inspect those answers.

Reuse rust-analyzer behind a pinned adapter for source semantics. Build our own persistent fact model, index lifecycle, query engine, comprehension workflows, and viewer. Add an optional, toolchain-pinned compiler adapter for MIR control flow. Do not make compiler execution a prerequisite for opening source.

rust-analyzer already provides a lazy semantic model, configuration-specific crate analysis, and consumer-facing analysis interfaces. Its Rust interfaces are explicitly unstable; its implementation internals are not a persistence format. That supports reuse behind an adapter, not exporting its internal IDs or serializing its database. [rust-analyzer architecture](https://rust-analyzer.github.io/book/contributing/architecture.html)

**The performance proposition:** pay expensive semantic costs once per relevant input change; serve most navigation from durable indexes; analyze only active or changed regions deeply; move bounded results to the browser. This can make repeated exploration much faster without claiming that we have invented a faster type checker.

**The correctness proposition:** every answer states its revision, configuration, evidence, and limitations. An attractive diagram is not a proof of behavior. An observed execution is not the set of all possible executions.

## 2. Assumptions and Non-Goals

Interpret "extremely large databases" as large codebases plus their indexed symbols, edges, revision history, configurations, and traces. This is not a database-engine schema visualization product.

Initial assumptions, to validate in milestone M0:

- Linux is the first supported analysis host; the browser is the primary client.
- Offline/local operation is required. A multi-user service is a later deployment of the same query contracts.
- Cargo workspaces are first-class. Explicit build-context manifests cover custom firmware builds and non-Cargo projects.
- Humans need accurate selected views, not every possible generic instantiation or every possible path materialized at once.
- Proprietary source stays within an approved machine or service boundary by default.
- The existing MEC firmware is a useful difficult pilot, not a special case hard-coded into the engine.
- Performance targets below are acceptance targets. No speedup, dataset size, or completion date in this document has been measured yet.

Explicit non-goals for the initial release:

- Replacing rustc, the borrow checker, rust-analyzer's editor completion, or a debugger.
- Proving arbitrary program correctness or deriving a complete specification from arbitrary Rust.
- Automatically proving that a code change cannot regress unobserved behavior.
- Fully sound whole-program pointer analysis, symbolic execution, or concurrency verification.
- Automatically editing source, merging PRs, or trusting an agent's explanation as evidence.
- A global graph containing millions of visible nodes, or a 3D graph as the default interface.
- GPU acceleration of parsing or type inference before CPU and I/O profiling justify it.

## 3. Human Workflows and Product Acceptance

### 3.1 Questions the product must answer

| ID | Human question | Required answer |
| --- | --- | --- |
| U1 | Where should I start reading? | Entry points, crate/module map, selected subsystem, source-linked reading trail |
| U2 | What calls this, and what does it call? | Incoming/outgoing call sites, configuration, resolution status, bounded expansion |
| U3 | What happens inside this function? | Source-level flow; optional compiler CFG; branches, exits, error propagation, suspension |
| U4 | What changes when this value or state changes? | Local definitions/uses, writes, selected dependencies, explicit aliasing limitations |
| U5 | What did this agent-written PR actually change? | Symbol/body/API/configuration deltas and evidence-backed impact candidates |
| U6 | Which behavior is insufficiently understood or tested? | Unresolved boundaries, unsupported analysis, exact test provenance, unobserved paths |
| U7 | Why did two executions diverge? | Comparable trace windows, state/event alignment, source mapping and timing uncertainty |
| U8 | Which abstractions make this code difficult to maintain? | Concrete cycles, large functions, nesting, duplication candidates and dependency concentration |
| U9 | Is this path actually in the firmware build? | Selected target/features, reachability evidence, binary membership when available |
| U10 | Can I share my understanding? | Revision-pinned bookmarks, reading trails and exported evidence, not screenshots alone |

### 3.2 Comprehension acceptance experiment

Use at least eight representative engineers in the first usability study. Include readers unfamiliar with the selected code. Counterbalance task order and tools to reduce learning effects.

Compare the viewer with editor navigation plus text search on the same revision/configuration. Measure task completion time, correct answers, unsupported conclusions, number of context switches, and confidence calibration. Include at least one task whose correct answer is "the evidence is incomplete."

Initial product target: at least 25% lower median task time on U1/U2/U5 without reduced answer accuracy. Treat this as an exploratory target, not a statistically established claim from a small study. Record per-task results and uncertainty; expand the study before making broad claims.

### 3.3 First demo must be an actual workflow

Open a real workspace, select an entry point, expand two call levels, click an edge to its call site, inspect an unresolved boundary, change a function, and compare the updated snapshot. Show a compiler CFG only when compiler evidence exists. A static mock graph does not satisfy the demo.

## 4. Scale and Performance Contract

### 4.1 Dataset tiers

LOC means physical Rust source lines for corpus sizing, with generated code and dependencies reported separately. Facts mean stored entity, relationship, occurrence, and evidence records. LOC-to-fact ratios vary; do not extrapolate from LOC alone.

| Tier | Qualification dataset | Reference resources | Intended support |
| --- | --- | --- | --- |
| F | Hand-authored fixtures and approximately 100k LOC pilot | 8 CPU cores, 32 GiB RAM, local SSD | First vertical slice and semantic correctness |
| L | Approximately 1M unique LOC; independently test 10M facts | 16 physical CPU cores, 64 GiB RAM, 1 TB NVMe | Production local use |
| S | Approximately 10M unique LOC; independently test 100M facts | 32 physical CPU cores, 128 GiB RAM, 4 TB NVMe | Large single-node service |
| O | Approximately 100M unique LOC; independently test 1B facts | Up to 10 workers, each S-sized; 10 Gb/s network; object storage | Organization-scale release |
| X | Approximately 1B unique LOC or 10B facts, with retained history | Capacity plan and resources to be measured | Research/stretch; not implied by O passing |

Also specify configuration count, number of independent repositories, crate fan-out, macro expansion ratio, largest single crate, retained revisions, and concurrent clients for every run. Ten million copied lines are a storage stress test, not a realistic semantic benchmark.

### 4.2 Initial latency and resource targets

All targets are provisional until M0 records a baseline. Warm query targets assume a published snapshot and the relevant index pages resident. They exclude asynchronous analysis and are not valid for an uncached, unindexed repository.

| Operation | L target | O target | Conditions |
| --- | --- | --- | --- |
| Exact symbol lookup | p95 <= 100 ms server time | p95 <= 250 ms | Authorized scope, top 50 results |
| One-hop call neighborhood | p95 <= 150 ms | p95 <= 300 ms | At most 200 nodes, 500 edges |
| Text/symbol search | p95 <= 200 ms | p95 <= 500 ms | Top 50, supported bounded query syntax |
| Source window fetch | p95 <= 100 ms | p95 <= 250 ms | At most 2,000 lines / 512 KiB |
| Cached function CFG | p95 <= 150 ms | p95 <= 300 ms | At most 200 visible blocks after grouping |
| Small symbol-level diff | p95 <= 500 ms | p95 <= 1 s | Both manifests indexed; <= 100 changed symbols |
| Warm body-edit update | p95 <= 2 s | <= 5 s active-workspace target | Ordinary non-macro edit; from snapshot capture to published affected facts |
| New local viewer session | p95 <= 2 s useful content | p95 <= 3 s on specified LAN | Existing index; assets measured separately |
| Foreground cancellation | <= 100 ms cooperative stages | <= 250 ms including RPC | Unresponsive worker may be terminated by supervisor |
| Graph interaction | p95 frame <= 33 ms | Same browser contract | 200 nodes / 500 edges, specified reference client |

Initial L cold semantic indexing target: <= 10 minutes and <= 32 GiB peak total analyzer RSS with dependencies/toolchains already available. Report download, sandbox setup, build scripts, proc-macro compilation, compiler extraction, and publication separately. Full compiler indexing has no universal ten-minute guarantee.

Active semantic workers receive individual memory budgets; the service must not silently exceed the machine budget. Viewer target: <= 500 MiB browser process growth during the standard session, with bounded retained source buffers and graph histories.

### 4.3 How to claim "faster"

Measure three different things:

1. **Analysis cost:** same source/configuration, comparable facts, cold and incremental timings against a pinned baseline.
2. **Query responsiveness:** persistent lookup versus equivalent baseline navigation, with warm and cold cache conditions disclosed.
3. **Human comprehension:** U1-U10 task outcomes, not just frames per second.

Measure p50/p95/p99, CPU-seconds, peak RSS, bytes read/written, index size, facts emitted, unresolved facts, and hardware cost. A faster run that skips proc macros or loses targets is not an equivalent semantic result. Do not publish "100x faster" without naming the exact workload and its evidence coverage.

## 5. System Architecture

```text
Source snapshot + lockfile + explicit build context + optional artifacts
                              |
                      Snapshot / input catalog
                              |
                 Resource-aware analysis scheduler
                 /              |               \
       Syntax worker      Semantic worker     Compiler worker
       (no execution)     (pinned RA adapter)  (optional, isolated)
                 \              |               /
                    Versioned fact batches
                              |
                Validation + immutable shard builder
                              |
                  Atomic snapshot publication
                              |
           Persistent indexes + source/artifact object store
                              |
                 Bounded, authorized query API
                    /                     \
             Browser viewer             CLI / editor links
                    |
        Optional trace, test, annotation, and explanation overlays
```

Separate processes for the service, semantic workers, and executable build/compiler work. A worker crash must not take down read-only browsing. A slow query must not monopolize analysis scheduling.

Start with a modular monolith for storage/query orchestration and supervised local workers. Retain process/RPC contracts that allow moving workers later. Do not begin with one microservice per graph relation.

## 6. Analysis Frontends and Build Fidelity

### 6.1 Three analysis levels

| Level | Implementation | Output | Limitations |
| --- | --- | --- | --- |
| A0: syntax | Pinned rust-analyzer syntax adapter | Items, spans, imports, lexical references, branch structure, raw cfg expressions | No semantic target claims |
| A1: semantic | Pinned rust-analyzer semantic adapter | Resolved definitions, types, traits, call sites, macro origins, selected configuration | Incomplete build context and dynamic dispatch remain explicit |
| A2: compiler | Optional pinned rustc driver | Selected MIR phase, basic blocks, terminators, local dataflow, source scopes | Requires compatible toolchain/build; not all generated behavior maps cleanly to source |

The default viewer works with A0. A1 upgrades existing objects with evidence; it does not replace ambiguous lexical facts with guessed targets. A2 is scheduled on demand or imported from approved CI artifacts.

For A2, use an external compiler adapter in a separate toolchain workspace. Pin the exact compiler commit, components, extraction phase, target and flags. Do not parse human-readable `rustc` diagnostic or MIR dumps as the long-term machine contract. MIR supplies explicit basic blocks and terminators; our adapter must translate that version's representation into our own schema. [MIR overview](https://rustc-dev-guide.rust-lang.org/mir/index.html), [external rustc drivers](https://rustc-dev-guide.rust-lang.org/rustc-driver/external-rustc-drivers.html)

### 6.2 BuildContext is a first-class input

Capture the following analysis input record. Keep source manifests and executable-producer outputs as referenced objects; the logical BuildContext identifies the configuration, not every unrelated file's contents. Per-product cache keys include actual dependencies, so an unrelated source edit does not invalidate all configuration reuse.

- Source tree digest, optional Git commit, dirty overlay digest, submodule/path-dependency digests.
- Cargo.lock digest and exact dependency source identities; missing/unlocked resolution status.
- Workspace manifest, selected package/target kind, crate roots, edition and resolver settings.
- Host triple separately from target triple or target specification digest.
- Selected features, default-feature policy, per-crate resolved features and cfg values.
- Relevant rustflags, panic strategy, optimization profile, custom target options and compiler version.
- Build-script/proc-macro input policy and captured output digests.
- Generated source content and origin, dependency metadata, approved environment inputs.
- Toolchain/sysroot identity, adapter version and trust policy digest.

Cargo metadata provides structured package and dependency information. Consume its JSON with a structured parser, but do not mistake package metadata for a complete record of every effective compiler invocation. [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html)

Build scripts can contribute generated material, cfg values, environment values and other build instructions. Capture these outputs or mark their absence; do not silently substitute host defaults. [Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html)

`cfg` selection can change which implementation exists. Store the raw condition and the selected configuration's result; an unknown custom cfg is not automatically false. [Rust conditional compilation](https://doc.rust-lang.org/reference/conditional-compilation.html)

### 6.3 Avoid the all-features trap

Analyze configurations explicitly. Default, all-features, firmware target and test target may be different programs; all-features can even be invalid. Offer named profiles and side-by-side configuration comparison. Never merge mutually exclusive configurations into an unlabeled call graph.

For the MEC pilot, derive profiles from its actual build process: target Rust firmware, host tests, simulator options and any bridge/diagnostic modes that still exist at the pinned revision. Do not assume x86 host analysis describes the live firmware path.

### 6.4 Language coverage and honest fallbacks

| Construct | Required representation |
| --- | --- |
| Direct/inherent call | Resolved source definition and call-site span |
| Generic trait call | Trait item plus substitution/context when known; implementation set only under recorded assumptions |
| `dyn Trait` / function pointer | Indirect-call site, possible targets when justified, otherwise unresolved target |
| Closure | Closure identity tied to its enclosing definition, captures where available |
| `async` / `.await` | Future construction versus polling/suspension; compiler lowering in its own view |
| `?`, panic, return, unwind | Distinct control-flow exits with panic strategy and extraction phase |
| Destructor / implicit drop | Compiler-derived or explicitly unavailable; not omitted from an advertised complete CFG |
| Macro expansion | Invocation and expansion locations, hygiene/origin chain, failed expansion reason |
| Generated source | Read-only generated file with producer/artifact identity |
| FFI / inline assembly | Boundary node and available external symbol; no invented Rust body |
| Interrupt / jump table / MMIO | Domain-adapter annotation or artifact/trace evidence, not an ordinary Rust call by default |
| Unsafe operation | Syntactic/semantic location and surrounding contract annotations; not an automatic bug finding |
| Broken/incomplete source | Partial items and diagnostics; unchanged valid index remains separately browsable |

Do not promise a complete interprocedural call graph for arbitrary Rust. Completeness always refers to a specified fact category, configuration and bounded scope, not all runtime behavior.

## 7. Evidence Model and Graph Semantics

### 7.1 Facts, interpretations and observations

Store these separately:

1. **Extracted fact:** a source item exists; a selected resolver binds a call to a definition; a compiler block has a successor.
2. **Derived result:** a bounded reachability query, strongly connected component, local dataflow result, complexity measurement.
3. **Interpretation:** a state-machine hypothesis, design annotation, agent-written summary, suggested responsibility boundary.
4. **Observation:** an event from an identified executable/run, or a test outcome with artifacts.

Each evidence record carries producer/version, inputs, scope, assumptions, source/artifact locations and collection status. No unexplained scalar "confidence: 97%". Use categorical basis and explicit limitations unless a probability is empirically calibrated.

### 7.2 Graph families

Keep distinct graph types with separate legends and query endpoints:

- Containment/dependency: crate, module, item, imports and package relationships.
- Calls: call sites and possible/resolved targets; edges do not imply execution order.
- Source flow: human-readable branches and exits; may omit compiler lowering.
- Compiler CFG: blocks/terminators at a named MIR phase.
- Local data dependence: definitions, uses and conservative memory effects.
- State transitions: event/guard/action transitions over a declared state abstraction.
- Runtime sequence: events, durations and known causal links from a particular run.
- Change impact: changed facts and potentially affected consumers under named relations.

An edge label distinguishes `resolved`, `possible`, `unresolved`, `annotated`, `observed`, and `compiler-derived`. These are evidence labels, not a single ordering of quality. An observed call and a resolved call can independently support the same relationship.

### 7.3 Unknowns must be queryable

Reasons include `missing_dependency`, `cfg_unknown`, `macro_unavailable`, `indirect_target_unknown`, `external_boundary`, `unsupported_construct`, `budget_exhausted`, `analysis_failed`, and `artifact_mismatch`.

Show omitted counts where known. If a count itself is unknown, say so. An empty result after timeout must not render as "no callers."

## 8. Identity, Schema and Versioning

### 8.1 Identity layers

| Identity | Meaning | Lifetime |
| --- | --- | --- |
| `RepositoryId` | Authorized source namespace | Repository lifetime |
| `SourceSnapshotId` | Content-addressed source manifest, including dirty/untracked inputs when selected | Immutable |
| `BuildContextId` | Canonical effective analysis/build configuration | Immutable |
| `AnalysisSnapshotId` | Source + contexts + producer set + published shard manifest | Immutable generation |
| `FileContentId` | Hash of exact bytes | Content lifetime |
| `FileOccurrenceId` | Repository-relative path in a source snapshot | Snapshot-local |
| `DefinitionId` | Definition in one snapshot and crate instance | Snapshot/config-local |
| `CallSiteId` | Source/expansion occurrence of a call | Snapshot/config-local |
| `InstanceId` | Optional instantiated function/closure with compiler context | Artifact/context-local |
| `CorrespondenceId` | Evidence linking definitions across revisions | Independent versioned relation |

Use canonical, domain-separated SHA-256 content digests for persisted objects. Include canonicalization/schema versions in hashed envelopes. Within a shard use dense integer IDs; globally expose opaque strings. Do not expose rust-analyzer IDs, pointer addresses or SQLite row IDs.

Canonical input hashes exclude wall-clock collection timestamps, job IDs and machine-local temporary paths. Include any normalized value that affects semantics. Store original provenance separately; path remapping is permitted only when its semantic effects, including path-sensitive macros, are preserved or explicitly invalidate reuse.

Derive a deterministic definition key from repository/package origin, crate instance, semantic owner path, item kind, and a snapshot-local disambiguator. Spans alone are insufficient. Same-named local items, macro-generated siblings, renamed items and multiple versions of a crate must remain distinct.

There is no universal stable symbol ID across arbitrary edits. Cross-revision matching starts with exact structural matches, then explicit rename/move evidence, then scored structural candidates. Ambiguous cases remain ambiguous. Never transfer an approval or proof solely because names match.

### 8.2 Minimum logical schema

```text
SourceSnapshot(id, repository_id, manifest_digest, revision_label, captured_at)
BuildContext(id, canonical_manifest_digest, trust_policy_id)
AnalysisSnapshot(id, source_id, contexts[], producer_set_id, shard_manifest_digest)
FileOccurrence(id, source_id, relative_path_bytes, file_content_id, origin_id)
Definition(id, context_id, file_occurrence_id, kind, owner_id, name,
           signature_digest, body_digest, source_span_id, origin_id)
Occurrence(id, context_id, owner_id, span_id, kind, resolution_status)
Relation(id, source_id, target_id_or_unknown, kind, occurrence_id,
         evidence_id, context_id)
Evidence(id, basis, producer_id, input_digests[], assumptions[], limitations[])
Coverage(scope_id, context_id, fact_family, status, reason_counts, known_total)
ControlFlow(body_id, phase, blocks_digest, source_map_digest, evidence_id)
Correspondence(old_definition_id, new_definition_id, method, status, evidence_id)
Artifact(id, digest, build_context_id, source_id, kind, provenance)
TestRun(id, artifact_ids[], test_identity, outcome, elapsed, log_digest)
Trace(id, artifact_id, event_schema_version, stream_manifest, loss_manifest)
Annotation(id, author, scope_id, evidence_refs[], content_digest, review_state)
```

The schema distinguishes a relation's logical source node from `SourceSnapshotId`; use typed IDs in code so the similar names cannot be interchanged. A resolved reference points into its snapshot's authorized manifest or an explicit external stub.

Use tagged unions for unknown targets, statuses and evidence kinds, not nullable fields with undocumented combinations. Normalized evidence is shared by references rather than copying its payload onto every edge.

### 8.3 Spans and serialization

- Store zero-based UTF-8 byte offsets in half-open ranges, tied to exact file-content digests.
- Return line/column information as derived data with encoding declared. Convert correctly for editor/LSP UTF-16 positions.
- Represent offsets/timestamps that may exceed JavaScript's safe integer range as decimal strings in JSON; typed binary formats may use `u64`.
- Preserve source bytes and line endings; never apply normalized offsets to original CRLF text.
- Preserve non-UTF-8 repository path bytes using an encoded transport form plus a display label.
- Macro/generated locations support multiple origins and approximate mapping status.
- Version storage, API, fact schema and analyzer producer separately. Readers reject unsupported required fields/versions instead of guessing.

### 8.4 Required invariants

1. Queries never combine shards from different publication generations accidentally.
2. Every reported source location matches the referenced content digest.
3. Every semantic fact has a context and producer.
4. Every unknown/truncation is distinguishable from an empty complete answer.
5. Re-indexing identical deterministic inputs produces identical normalized fact digests.
6. Failed or nondeterministic executable producers cannot poison a reusable deterministic cache entry.
7. Forward and reverse relation indexes agree for the same published fact set.
8. A query cannot expose an unauthorized repository through names, counts, cached results or links.

## 9. Storage and Publication

### 9.1 Default storage choices

Start with SQLite for local catalog/control records and immutable SQLite fact shards for the first release. Add Tantivy for symbol/text postings. Store source, generated source, traces and build artifacts in a content-addressed filesystem store. Shards are written privately, finalized, validated, then published read-only.

SQLite WAL belongs only to the local mutable catalog, with a controlled writer/checkpoint policy; do not place a multi-host WAL database on NFS. Publish finalized shard files without live WAL sidecars. Pin a currently supported patched SQLite build and verify it at runtime. [SQLite WAL behavior and limitations](https://www.sqlite.org/wal.html)

Tantivy is a Rust full-text search library, not our source-of-truth graph database. Index symbol names, paths, documentation and optional source text; retain scope/configuration fields and immutable index generations. [Tantivy project](https://github.com/quickwit-oss/tantivy)

For O scale, use a service-owned transactional catalog, default PostgreSQL, plus immutable shards in object storage and bounded local NVMe caches. The catalog stores manifests, ownership and jobs, not every billion-scale edge. Do not share SQLite catalog files between machines.

### 9.2 Minimum indexes

| Query | Index |
| --- | --- |
| Exact definition / source lookup | Definition ID and file-content indexes |
| Callers | `(target_id, relation_kind, source_id, occurrence_id)` |
| Callees | `(source_id, relation_kind, target_id, occurrence_id)` |
| Source-to-symbol | `(file_occurrence_id, start_byte)` with range filtering |
| Owner/module expansion | `(owner_id, kind, name, definition_id)` |
| Trait implementations | `(trait_definition_id, context_id, implementing_type_key)` |
| Changed facts | Manifest/object digests, per-body/signature summaries |
| Name/text search | Postings with scope/configuration filters |
| Evidence inspection | Evidence ID plus input/artifact references |

Dense adjacency arrays are a later physical optimization, not a different logical model. Prototype sorted forward/reverse adjacency segments behind the same `FactReader` boundary at M4. Adopt them only if equivalent queries show material improvement in memory, bytes/fact or latency. Keep evidence and source payloads separately addressable. Avoid inventing a custom binary format before the simpler baseline is measured.

### 9.3 Atomic publication protocol

1. Freeze a source/context manifest; never analyze an arbitrarily changing directory as if it were one revision.
2. Schedule tasks using input digests and a generation token.
3. Write immutable result batches under temporary task-owned locations.
4. Validate schema, references, counts, checksums and coverage reports.
5. Build all indexes required for the advertised capability; persist and verify their objects.
6. Commit one catalog transaction referencing the complete immutable manifest.
7. Advance the named workspace head using compare-and-swap against the intended parent generation.
8. Notify clients; keep existing sessions pinned until they choose the new generation.

If only some crates succeed, publish a self-consistent partial snapshot with explicit stubs/coverage gaps. Do not silently reuse incompatible old semantic facts under a new source revision. Users may open the prior complete snapshot separately.

External tasks execute at least once with deduplication, not a fictional exactly-once process guarantee. A fencing token prevents a timed-out worker from publishing over its replacement.

### 9.4 History, retention and disk budgets

Deduplicate unchanged source and fact objects across compatible snapshots. Maintain explicit reference roots for published snapshots, active queries, bookmarks requiring retention and pinned test evidence.

Garbage collection marks referenced objects, applies a grace period, and deletes only unreferenced store-owned objects. Never delete user worktrees/build outputs as an index cleanup operation. Keep a dry-run report and crash-recovery tests.

Initial policies: retain active workspace head plus configurable recent generations, retain pinned review/test snapshots, cap unpinned trace retention separately. Pause new ingestion at 80% of the configured disk quota and require reclamation before 90%; keep enough reserved space for manifest/catalog recovery. Show actual bytes by category.

Capacity planning equation:

```text
disk = source_CAS + fact_records + forward_reverse_indexes + text_postings
     + retained_delta_objects + artifact_trace_objects + build_scratch
     + compaction_and_recovery_headroom
```

Illustration only: 1B edges at 32 bytes/edge already require 32 GB before indexes, evidence, replicas or snapshots. Record measured bytes per fact family; do not size the deployment from this illustrative edge encoding.

### 9.5 Physical partitioning and reusable facts

Analysis scheduling units and storage shards are not identical. A semantic worker may require a crate plus its dependency closure; stored facts can be split into smaller independently addressable chunks without pretending that type inference is independently splittable at the same boundary.

Start with repository/context/crate partitioning. Target finalized shards of approximately 256 MiB to 1 GiB, adjusted from measured query locality and rebuild costs. Split unusually large fact sets by stable owner-key ranges; do not scatter every related item randomly across the cluster. Keep dependency crates reusable only when their full semantic input context is compatible.

The logical schema exposes snapshot-local identities, but reusable physical facts must not embed a new global snapshot ID in every row. Store shard-local definition/occurrence keys, content identities and relative origin data; bind these through the immutable snapshot manifest into API identities. This indirection allows unchanged shards to be reused across revisions without conflating API identity or source versions.

Changed source requires refreshed span/origin mapping even when semantic payloads are reusable. A formatting edit cannot retain old offsets merely because a body fingerprint is unchanged. Test physical reuse and logical rebinding together.

Support small immutable delta shards for changed/deleted owners, with explicit newest-generation resolution and tombstones. Cap the read amplification from delta chains; initial policy compacts after eight layers or when delta bytes exceed 20% of the base partition. Tune these thresholds from benchmarks. Compaction publishes a new equivalent manifest and never modifies objects read by an active snapshot.

### 9.6 Distributed routing and catalog consistency

For O, a snapshot manifest binds definition directories, forward/reverse indexes, text-search generations and source objects together. Cross-shard target references resolve through that manifest, not a mutable global "latest definition" table.

Route repository-scoped queries only to authorized relevant partitions. Maintain compact shard-routing metadata for exact symbol lookup and reverse-call lookup. Organization-wide search uses a separate scoped postings directory, bounded fan-out and top-k merge; return partial/unavailable status if participating shards do not answer in budget. Do not report an exact global total from an incomplete merge.

Replicate published immutable objects according to availability requirements and cache hot partitions on local NVMe. Workers never need write access to the entire object namespace. Publish through the catalog owner using generation/fencing tokens. A manifest that references an unavailable shard is reported unavailable/partial, not silently replaced with an older shard.

Do not eagerly compute a global SCC decomposition for every edit. Maintain cheap crate/module summaries incrementally; schedule larger component analyses for selected snapshots/scopes. A routing catalog or search aggregator that becomes a bottleneck is profiled and partitioned explicitly rather than hidden behind unlimited RPC fan-out.

## 10. Incremental Analysis and Scheduling

### 10.1 Two cache layers

The warm semantic worker maintains its own in-process incremental state. Our persistent layer caches normalized extraction products, indexes and dependency fingerprints. These are different caches with different keys and eviction rules.

Never serialize a live semantic database and assume it survives analyzer/toolchain upgrades. A worker can be discarded and recreated from a manifest. Persisted outputs remain usable only under their producer/input compatibility rules.

### 10.2 Invalidation policy

| Change | Minimum invalidation |
| --- | --- |
| Whitespace/comments | Source spans/display; syntax-derived documentation; semantic mapping must be revalidated |
| Ordinary private body | Body facts, local flow, call edges and summaries depending on that body |
| Signature/type/visibility | Definition plus affected semantic consumers and graph summaries |
| Trait impl set / imports / macros | Affected resolution domains, potentially broad reverse dependencies |
| Const body / generated item / opaque inferred type | Consumers of the semantic result, not merely lexical API signatures |
| Features/target/cfg | New context or re-analysis of affected crate instances |
| Build script/proc macro/toolchain | All dependent outputs according to captured input provenance |
| Deleted/renamed source | Tombstone old occurrences; validate correspondence; rebuild relevant indexes |

Do not implement incremental correctness as "body changed, therefore dependents cannot change." Cross-crate generic bodies, compile-time evaluation and analysis summaries can depend on bodies. Reuse only when the producing analysis exposes trustworthy dependencies or a conservative rule proves the product's inputs unchanged. Otherwise re-index the affected crate/dependent scope.

### 10.3 Scheduler

Prioritize open-file/selected-function work, changed workspace indexing, CI-published snapshots, then background deep analysis and compaction. Each task declares CPU, memory, disk, trust level, expected output size and deadline.

Use per-tenant and global admission limits. Account for compiler subprocesses in the same budget; do not multiply CPU count by both worker count and Cargo job count. Reuse active dependency closures opportunistically, but do not invent serialized crate summaries as a drop-in replacement for the semantic frontend's real dependency inputs.

Coalesce rapid edits, cancel stale jobs, preserve useful immutable objects, and prevent completed stale jobs from advancing the head. A file-watcher overflow triggers reconciliation; the watcher is a hint, not the authority for snapshot contents.

Large crates that exceed a worker's memory budget are an explicit failure/partial-analysis condition. More machines do not automatically split a single compiler query. Record this limit separately from aggregate organization scale.

## 11. Query Engine and APIs

### 11.1 Query rules

- Every request pins an analysis snapshot and context, or explicitly resolves a named head once at request start.
- Authorize before routing, lookup, scoring, counting and returning results.
- Every graph request has node, edge, depth, time and response-byte budgets.
- Use deterministic order and keyset pagination; cursors bind query hash, snapshot, context, authorization scope and expiry.
- Cancel downstream work when the client disconnects or deadline expires.
- Exact negative answers require complete coverage for the requested relation/scope.
- Expensive transitive/dataflow requests become asynchronous jobs rather than holding an interactive request indefinitely.
- No arbitrary SQL or unrestricted recursive graph language exposed in v1.

Default graph budgets: 200 visible nodes, 500 edges, depth 2, 250 ms interactive computation and 2 MiB response. Group or paginate beyond those limits. Explicit exports may use larger server-side budgets without sending an unbounded graph to the browser.

### 11.2 Endpoint contract

Use Rust, Tokio and Axum for the first HTTP service; keep the query engine independent of HTTP. Axum is an HTTP routing/handling library integrated with Tokio and Tower, not a substitute for our scheduler or query budget enforcement. [Axum documentation](https://docs.rs/axum/latest/axum/)

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/capabilities` | Supported schema, analysis and query capabilities |
| `GET /v1/snapshots/{id}` | Contexts, coverage, immutable manifest and provenance |
| `GET /v1/search` | Scoped symbol/text search |
| `GET /v1/definitions/{id}` | Definition, source and evidence |
| `GET /v1/source/{file_occurrence_id}` | Bounded source window, content digest and span map |
| `POST /v1/graph/neighborhood` | Typed bounded expansion |
| `GET /v1/bodies/{id}/flow` | Source flow or named compiler phase |
| `POST /v1/diff` | Two pinned snapshot/context pairs |
| `POST /v1/queries/impact` | Bounded impact result or async job |
| `GET /v1/evidence/{id}` | Evidence payload and assumptions |
| `POST /v1/jobs` | Authorized analysis request, subject to trust policy |
| `GET /v1/jobs/{id}/events` | Progress via server-sent events |
| `POST /v1/jobs/{id}/cancel` | Idempotent cancellation |
| `GET /v1/traces/{id}/window` | Bounded event window |

### 11.3 Example result envelope

The IDs below are illustrative, not actual hashes.

```json
{
  "api_version": "1",
  "snapshot_id": "analysis:example",
  "context_id": "context:firmware",
  "query_id": "query:example",
  "items": [{
    "edge_id": "relation:example",
    "kind": "calls",
    "source": "definition:dispatch",
    "target": {"kind": "resolved", "id": "definition:handler"},
    "call_site": "occurrence:example",
    "evidence_ids": ["evidence:resolver-example"]
  }],
  "coverage": {
    "scope": "definition:dispatch",
    "fact_family": "call_targets",
    "status": "partial",
    "reasons": [{"kind": "indirect_target_unknown", "count": 1}]
  },
  "page": {"truncated": false, "next_cursor": null},
  "work": {"deadline_reached": false, "elapsed_ms": 12}
}
```

Separate analysis incompleteness, result pagination and query execution truncation. Finishing a page does not imply all call targets are known.

### 11.4 Errors and upgrades

Define typed errors for invalid query, unknown snapshot, context mismatch, unsupported capability, expired cursor, not authorized, budget exhaustion, unavailable shard and incompatible artifact. Return correlation IDs, not internal paths or credentials.

Generate TypeScript transport types from the canonical schema and validate both client and server fixtures. Changes to schemas require a version decision and consumer contract tests. Never let UI agents hand-copy divergent definitions.

## 12. Graph Algorithms and Bounded Deep Analysis

Use adjacency indexes for one-hop expansion. Use bounded BFS for reachability/path exploration, with explicit unknown frontiers and visited-set budgets. Compute strongly connected components for selected graphs in background jobs; collapse recursive components for presentation.

Do not materialize all-pairs reachability, enumerate all execution paths, or expand every possible monomorphization. Index size should grow with captured facts, not quadratically with symbol count.

For organization-scale calls, maintain a manifest-versioned target-to-source-shard directory so callers queries do not scan every shard. High-degree targets use paged postings. Broad transitive queries remain asynchronous and may be incomplete under budget.

Dataflow v1 is intraprocedural, over a selected MIR body with well-defined transfer functions. Unknown calls, pointers, aliasing, atomics and external state produce conservative effects/unknowns. Context-sensitive interprocedural analysis is optional future work with an explicit abstract domain, convergence strategy and precision budget.

State-machine extraction is opt-in: select a state enum/variable and event boundary, derive supported syntactic transitions, then let a reviewer validate guards/actions. A `match` over an enum alone does not prove the complete state machine, especially when mutation happens through aliases or interrupts. Store inferred and reviewed transitions separately.

All graph algorithms have deterministic fixture tests and budget-exhaustion behavior. When using approximations, return the algorithm version and assumptions with the result.

## 13. Viewer Design

### 13.1 Main workspace

Build the usable exploration surface as the first screen, not a marketing landing page.

```text
Repository / revision / context       Search           Index status
------------------------------------------------------------------
Scope tree       | Source / diff                | Evidence inspector
and bookmarks    |                              | Types / targets
                 |------------------------------| Assumptions
                 | Focused graph / flow / trace | Unknown boundaries
------------------------------------------------------------------
Selection breadcrumb / query status / background analysis progress
```

Panels are resizable, collapsible and keyboard reachable. A narrow viewport uses one primary view with tabs and an inspector drawer instead of squeezing three columns. Preserve selection and back/forward history when changing views.

Default views:

1. **Explore:** module structure, source and selected call neighborhood.
2. **Flow:** selected function's source flow or compiler CFG, explicitly labeled.
3. **Change:** before/after source and semantic delta, with impact candidates.
4. **Evidence:** test, trace, annotation and analysis provenance for the selection.
5. **Health:** indexing coverage, failures, unresolved categories and resource use.

### 13.2 Renderer and editor choices

Use React and TypeScript for the shell, CodeMirror 6 for source presentation, Cytoscape.js for bounded interactive graphs, and ELK layout in a Web Worker for directional graphs. Pin versions and validate interoperability in M0. These are proposed dependencies, not a claim that their combined performance has been measured.

CodeMirror supplies the editor view component; Cytoscape.js supplies graph visualization APIs; elkjs supplies graph layout algorithms and worker integration. Keep adapters around editor, graph and layout so replacing a renderer does not change semantic contracts. Follow the current CodeMirror upstream location rather than assuming its archived GitHub mirror is active. [CodeMirror view project](https://github.com/codemirror/view), [Cytoscape.js documentation](https://js.cytoscape.org/), [elkjs project](https://github.com/kieler/elkjs)

Use Canvas-based graph rendering initially. Add a WebGL renderer only if bounded-view benchmarks justify it. GPU rendering does not make unreadable graphs useful or solve semantic indexing. Do not require GPU compute for the analyzer.

### 13.3 Interaction contract

- Clicking a node selects its exact definition; clicking an edge opens its call site and evidence.
- Expand incoming/outgoing relationships independently, with relation and evidence filters.
- Cluster by crate/module, then optionally by recursive component or user-defined subsystem.
- Preserve node positions when adding a small neighborhood; lay out only the changed region where feasible.
- Render high-degree nodes with counts and paged expansion instead of thousands of edges.
- Tooltips show qualified names; the inspector shows full signatures and context, not clipped labels.
- Back/forward, breadcrumbs, pin/unpin, fit-to-selection, zoom and copy-link are first-class controls.
- Source scrolling and graph selection synchronize without repeatedly moving the user's viewport.
- Multiple call sites between the same functions remain inspectable even when aggregated into one drawn edge.
- Evidence categories use line style, icons and labels as well as color.
- The URL stores snapshot/context/selection/view state, not a huge serialized graph or secret source text.

Use familiar icon buttons with tooltips for navigation and graph tools, segmented controls for view modes, and checkboxes for filters. Use a restrained operational UI, readable text and unframed working areas. Avoid oversized cards, nested cards and decorative backgrounds.

### 13.4 Source and layout budgets

Fetch source windows rather than whole generated files by default. Normal files may load fully up to a configured size; giant files switch to a windowed viewer with explicit missing context. Maintain a byte-to-line index on the server.

Virtualize long trees, tables, search results and trace rows. Keep layout work out of the main browser thread. Abort outdated layout requests when selection changes. Cache layouts by snapshot, graph selection, filters and algorithm version.

At 200 nodes / 500 edges, interactions must meet the frame budget in section 4. At 5,000 raw candidate nodes, the server must return grouping/pagination instead of making the browser attempt the full layout. For a single huge CFG, group loops/regions or show a textual block table with navigation.

### 13.5 Accessibility and verification

Every graph has an equivalent navigable relationship table or tree. Support keyboard-only workflows, visible focus, screen-reader labels, reduced motion, high contrast, browser zoom and non-color evidence distinctions.

Use Playwright to exercise source-edge navigation, deep links, stale revisions, loading/error states, narrow viewports and keyboard access. Inspect desktop/mobile screenshots for overlap. For Canvas/WebGL, verify nonblank pixels and actual visible nodes/edges, not merely the existence of a canvas element. Test layout cancellation and selection persistence.

## 14. Understanding Agent-Written Code

### 14.1 Review starts with a delta

Given two snapshots, group changes into:

- Added/removed/renamed definitions and modules.
- Signature/type/visibility changes.
- Body-only edits, including added/removed call sites and changed branch structure.
- Error handling, unsafe operations, synchronization and external-effect changes.
- Configuration/dependency changes that alter active code without changing a function's text.
- New or changed tests, their artifacts, and the symbols they actually cover when evidence exists.

Show unchanged context selectively around changed nodes. A semantic diff describes changes in the extracted model; it is not proof of behavioral equivalence. A byte-identical executable can support reuse of that executable's runtime evidence only when the run environment, relevant non-code inputs and test contract are also compatible.

### 14.2 Evidence-backed comprehension aids

Offer deterministic summaries before generative ones: signature, callers/callees, side effects known to the selected analysis, branch/exit counts, cfg, changed dependencies and available test evidence.

Store optional agent provenance only when supplied by a trusted integration: PR/commit, producing system, task reference and declared intent. Do not infer that code was agent-written from style. Avoid ingesting private prompts by default.

An optional LLM explanation service receives a bounded evidence bundle, not the whole repository. It must cite exact symbols/spans, identify unknowns, and separate interpretation from extracted facts. Cache by evidence digest and model/prompt version. Invalidate when the supporting facts change.

Repository comments, documentation and annotations are untrusted data, not instructions to the explanation agent. No tool execution, source edits or credential access is permitted in this read-only explanation path. Human approval of a summary does not approve a new revision automatically.

### 14.3 Maintainability and complexity metrics

| Metric | Calculation / scope | Caveat |
| --- | --- | --- |
| Function size | Nonblank/noncomment source lines and token count | Report generated/macro code separately |
| Nesting | Explicit versioned syntax traversal | Not a standardized universal cognitive score |
| Cyclomatic complexity | `E - N + 2P` on a defined CFG, with exit convention documented | Source flow and compiler CFG are different measures |
| Fan-in / fan-out | Distinct callers/callees plus call-site counts | Unresolved calls change interpretation |
| Dependency cycles | SCCs on named dependency graph | Cycles are not automatically defects |
| API surface | Public items and externally visible signatures | Internal visibility/reexports need resolution |
| Unsafe boundary concentration | Unsafe locations and enclosing boundaries | Quantity alone does not measure unsafety |
| Duplication candidates | Normalized syntax fingerprints followed by structural comparison | Similar code may intentionally differ in ordering or timing |
| Change concentration | Versioned history over selected path/symbol correspondences | Rename ambiguity and history availability matter |
| Test evidence | Observed symbol/branch/event coverage for identified artifacts | Unobserved is not necessarily untested; observed is not exhaustive |

Do not collapse these into a misleading universal quality score. Show distributions, trends, outliers and the concrete code behind a finding. Any composite ranking must expose its weights and be presented as triage, not objective correctness.

### 14.4 Reading trails and contracts

Allow reviewers to create a source-linked sequence of definitions with notes, invariants and unanswered questions. A trail is pinned to a snapshot and may propose migration through reviewed cross-revision correspondences.

Support contract annotations for ownership, framing, preconditions, ordering, progress guarantees and external effects. Mark whether a contract is a comment, reviewed annotation, test assertion or formal-verification artifact. Imported proof results require tool/version, assumptions, exact input digest and checked result; "formally verified" is not a repository-wide checkbox.

## 15. Traces, Tests and Firmware Extensions

### 15.1 Trace support is an overlay, not the semantic foundation

The core product must work without traces. Trace ingestion can be implemented independently once IDs, artifact provenance and source mapping contracts are fixed.

Minimum event record:

```text
TraceEvent {
  trace_id, stream_id, sequence_number,
  process_or_device_id, thread_or_hart_id,
  clock_domain, timestamp, timestamp_unit,
  event_kind, executable_artifact_id,
  address_or_instrumented_location,
  correlation_id?, payload_ref?, loss_marker?
}
```

Support explicit events for function entry/exit, state transition, packet/opcode, queue switch, synchronization, external effects and checkpoints. A format adapter may only populate fields present in the input; missing values remain absent.

### 15.2 Source mapping and timing correctness

Resolve addresses against the exact executable/debug-artifact digest, load mappings, inline frames and source snapshot. Retain unresolved addresses. Never symbolize a trace against the newest build merely because function names match.

Maintain per-stream sequence order. Cross-stream ordering requires synchronization/correlation evidence or calibrated clock mappings with uncertainty. Do not impose a total causal order by sorting unrelated clocks. Distinguish CPU time, wall time, simulator cycles, device cycles and sampling estimates.

Track event loss, sampling, buffering delay and instrumentation overhead. A period with no events is not automatically a stall. Inclusive/exclusive function time is available only when event nesting and timing permit it; otherwise show samples or intervals with their limitations.

### 15.3 Comparing executions

Align comparable runs using semantic anchors such as packet sequence, queue identity, state transition and checkpoint hashes. Use bounded windows and indexed anchors, not an all-pairs comparison of huge traces. Report the first observed divergence under the alignment rule, not "the root cause" without further evidence.

A change in event count might reflect different work, instrumentation, loss, branch behavior or timing. Show these possibilities explicitly. Keep original and Rust firmware artifacts/run inputs separate even if the workload name is identical.

### 15.4 Replay integration boundary

This product visualizes and indexes replay evidence; it does not automatically implement a firmware simulator. A replay adapter may invoke an existing authorized harness as a separately scheduled job.

Replay validity requires all relevant external inputs, ordering constraints, initial state, modeled device behavior and expected outputs. Missing queue-manager timing, interrupt interleavings or device side effects can invalidate conclusions. Passing a trace replay proves conformance to that captured experiment and model, not all simulator or hardware behavior.

### 15.5 Test status model

Store `pass`, `fail`, `expected_fail`, `unexpected_pass`, `timeout`, `infrastructure_error`, `not_run`, and `unknown` separately. Preserve timeout/cap values, elapsed time, reason, firmware digest, test revision and logs.

Derived comparisons may classify original-pass/Rust-fail cases only when both controls are comparable. Distinguish missing controls from control failures. Reuse historical results only through an explicit evidence-compatibility decision, never simply because a PR was rebased.

### 15.6 MEC pilot acceptance

At a freshly pinned MEC revision, validate these reading tasks with a domain reviewer:

1. Follow firmware initialization to packet dispatch and a selected PM4 handler.
2. Follow AQL dispatch and explain generated/specialized paths versus generic paths.
3. Identify a queue-switch or interrupt boundary that static Rust calls alone do not capture.
4. Compare target firmware with host-test configuration without mixing implementations.
5. Inspect a packet-framing or continuation contract with source and tests side by side.
6. Show a supplied original/Rust trace pair with artifact identities and timing domains.

Treat assembly jump tables, hardware dispatch and MMIO as explicit boundaries. Domain extensions emit evidence through the public schema, not ad hoc hidden graph edges. A command registry entry is evidence of dispatch metadata, not proof that its handler is fully implemented.

## 16. Security, Privacy and Trust Modes

### 16.1 Trust modes

| Mode | Permitted actions | Default use |
| --- | --- | --- |
| Read-only | Read allowlisted source snapshots; parse; use imported verified metadata | Opening an unknown repository |
| Isolated analysis | Approved Cargo/compiler/proc-macro tasks inside restricted workers | Building missing semantic context |
| Trusted local | Explicitly approved broader local build integration | Controlled developer environments only |

The default must not execute Cargo, rustc wrappers, build scripts, proc macros, Git hooks, credential helpers or repository-specified commands merely because a user opened a directory. Parse manifests in-process or import a previously captured context before requesting executable analysis.

Procedural macros execute code at compile time and have build-script-like security concerns. Cargo configuration can also select executables and wrappers; disabling `build.rs` alone is not a sufficient trust boundary. [Rust procedural macros](https://doc.rust-lang.org/stable/reference/procedural-macros.html), [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html)

### 16.2 Executable worker isolation

Use a dedicated unprivileged identity and tested OS isolation: read-only source/toolchain mounts, private writable scratch/output, denied host-home/SSH-agent/credential access, no network by default, and CPU/memory/process/disk/time limits. A separate process by itself is not a sandbox.

For higher-risk multi-tenant execution, use a VM or equivalent approved isolation boundary. Containers alone are not an unconditional hostile-code isolation guarantee. Do not mount container-control sockets or privileged host devices.

Approved dependency acquisition is a separate brokered step with checksums and a frozen input manifest. Do not pass registry credentials through to arbitrary build scripts. Scrub inherited environment and repository-controlled compiler/tool selectors according to policy. Record sanitized effective settings and any fidelity loss.

Unrecorded/nondeterministic executable inputs prevent cross-run output reuse unless the environment is sufficiently constrained and identified. Capturing stdout does not capture every file/env/network input a build script might depend on.

### 16.3 Service and browser controls

- Default local bind is loopback with per-session authorization and origin checks; localhost alone is not authentication.
- Multi-user deployment requires authenticated identities, repository ACLs and tenant-scoped caches before exposure.
- Authorize snippets, symbols, graph counts, traces, downloads, autocomplete and shared links consistently.
- Render source, documentation and model output as untrusted text or sanitized content; disable raw HTML execution.
- Bound parser inputs, macro expansions, archive extraction, regex work, graph queries and layout sizes.
- Treat symlinks, source paths and uploaded artifact paths as untrusted; enforce allowed roots.
- Do not log source contents, credentials, trace payloads or user queries unnecessarily.
- Use encryption and access-controlled artifact retention in hosted mode; audit reads/exports as required by the deployment.
- Cross-tenant deduplication is disabled initially to avoid content-existence leakage and complex deletion semantics.

All security tests are defensive validation with harmless fixtures. No automatic exploration of external targets is part of this product.

## 17. Correctness and Regression Strategy

### 17.1 Independent evidence, not circular validation

Use small, hand-reviewed Rust fixtures with expected facts; compare selected compiler-derived semantics with the matching rustc toolchain. Compare against rust-analyzer navigation for adapter integration coverage, but do not call that an independent oracle when the same engine produced both results.

Compiler acceptance does not prove a complete call graph. A trace can demonstrate a missing observed edge, but cannot establish that unobserved paths do not exist. Keep these limitations in test reports.

### 17.2 Required fixture families

- Modules, reexports, aliases, visibility, multiple crate versions and path dependencies.
- Generic/inherent/trait methods, associated types, blanket impls and dynamic dispatch.
- Closures, async/await, `?`, panic strategies, drop order and unreachable syntax.
- Declarative/procedural macros, expansion failure, generated code and origin mapping.
- Host/target split, `no_std`, custom cfg, feature combinations, test-only code and broken builds.
- UTF-8 identifiers, CRLF, non-UTF-8 paths, large files and duplicate local item names.
- Source edits involving renames, moves, deletions, constants, inferred types and impl-set changes.
- FFI, assembly and domain-annotated dispatch with explicitly unknown boundaries.

### 17.3 Storage and incremental tests

Use property-based randomized edit sequences: incremental normalized facts must equal a clean re-index of the same supported inputs. Check this across dependency/configuration changes, not only body edits.

Inject worker termination, partial batches, checksum failures, disk-full conditions, expired leases, duplicate task completion and process crashes around publication boundaries. The visible state must be the old valid snapshot or the new valid snapshot, never a mixed generation.

Test reverse/forward index consistency, GC with active reader leases, schema migration rollback, snapshot pinning, concurrent updates and cache invalidation. Rebuild identical inputs under different worker schedules and compare normalized digests.

### 17.4 Query and UI tests

Check truncation/unknown semantics, authorization at every endpoint, stable pagination, cancellation, high-degree nodes, giant source windows, missing shards and stale contexts. A timeout must never convert to an empty complete answer.

Exercise all U1-U10 workflows with recorded fixture-backed results. Test inaccessible graphs through the equivalent table. Check screenshot layout at 1440x900, 1024x768 and 390x844, plus 200% browser zoom.

### 17.5 CI tiers

| Tier | Trigger | Target duration | Contents |
| --- | --- | --- | --- |
| C0 | Every local edit / agent task | <= 60 s warm | Owned crate tests, schema validation, targeted regression first |
| C1 | Every PR | <= 10 min warm | Unit/property tests, query contracts, viewer smoke, static checks |
| C2 | Merge queue/nightly | Measured budget | Full semantic fixtures, compiler adapters, clean-vs-incremental, crash/recovery |
| C3 | Performance release candidate | Dedicated hosts | L/S/O workloads, load, cold/warm baselines, recovery, usability evidence |

Measure actual compile/test time and publish it. Cold builds of the pinned semantic frontend may exceed C0/C1; report separately and use trusted build caches. Do not omit the relevant regression test just to make the duration target green.

## 18. Performance Qualification and Observability

### 18.1 Benchmark corpus

Maintain version-pinned, license-reviewed real Rust projects spanning application, library, async service, macro-heavy and embedded patterns. Use private MEC only in its authorized environment. Include generated synthetic graphs for degree, history and storage extremes, clearly labeled synthetic.

Every corpus manifest records source/artifact hashes, toolchain, configurations, macro policy, expected capabilities and unavailable dependencies. A benchmark with an unavailable compiler is a partial benchmark, not a full semantic pass.

Run baseline and candidate on the same hardware, with identical selected facts and configuration. Use isolated cold-cache hosts or disclosed cache-reset methods, not a casual second run called cold. Separate first-build dependencies from steady-state analysis. Record at least 30 repeated interactive samples per workload and enough load duration to expose tail behavior; retain raw samples.

Thirty samples suffice only for smoke comparisons, not credible p99 claims. Release tail-latency reports require at least 10,000 requests per named workload/load condition, repeated runs, and disclosed variation. Do not combine unrelated workloads into one flattering percentile.

### 18.2 Load profiles

L: one interactive reader plus active workspace edits and one background index job.

S: 20 concurrent readers, mixed symbol/search/graph/source requests, two indexing jobs, one compaction job.

O: 100 concurrent readers distributed across repositories; 20 active indexing tasks subject to measured admission limits; realistic hot/cold skew. Exercise both narrow repository queries and explicitly broad organization search. Publish offered request rate and saturation point, not just number of browser sessions.

Required adversarial-but-benign workloads: a high-fan-out trait, a function with many call sites, a huge generated file, rapid edits, a dependency/configuration change, missing shard, cold shard fetch and a single giant crate.

### 18.3 Instrumentation

Trace spans across snapshot capture, discovery, parse, macro work, resolution, fact extraction, indexing, publication, query planning, shard fetch, result merge, source fetch and browser layout.

Export queue wait, active tasks, CPU-seconds, RSS, cancellation delay, bytes/fact, disk quota, cache hit/miss, published-snapshot lag, unknowns by reason, shard fan-out, query budget exhaustion and p50/p95/p99 latency.

Keep user-visible progress based on completed units and measured stage times. Do not report "90% complete" because 90% of files were scanned when type analysis has not started. Show `discovery`, `syntax`, `semantics`, `compiler`, and `publication` separately.

### 18.4 Optimization order

1. Profile and eliminate repeated parsing, resolution, serialization and unbounded traversals.
2. Improve artifact reuse, batching, indexing and compact representation.
3. Fix invalidation granularity without weakening correctness.
4. Tune scheduler, memory limits, prefetch and cache behavior.
5. Introduce specialized adjacency storage only if it wins the benchmark gate.
6. Shard and distribute proven units of work.
7. Consider SIMD/GPU work only for a demonstrated suitable hotspot with transfer costs included.

GPU graph rendering, embedding generation or batched offline numeric analysis may be reasonable separate experiments. GPU acceleration is not a prerequisite or assumed shortcut for Rust semantics.

## 19. Repository Layout and Ownership

Proposed new repository; do not embed this product into the firmware workspace.

```text
rustscope/
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  AGENTS.md
  crates/
    model/             # typed IDs, facts, canonicalization, schema fixtures
    ingest/            # snapshot capture, build contexts, source manifests
    frontend-ra/       # all pinned rust-analyzer dependencies and translation
    store/             # catalogs, immutable shards, CAS, publication, GC
    query/             # graph/search/diff APIs and resource budgets
    analysis/          # flow adapters, metrics, local dataflow, correspondences
    evidence/          # traces, tests, annotation and artifact mapping
    scheduler/         # admission, supervised workers, jobs and cancellation
    server/            # HTTP/auth integration, generated API schema
    cli/               # local lifecycle and scriptable queries
  adapters/
    rustc/             # separate pinned toolchain workspace and lockfile
    mec/               # optional domain adapter, no firmware source copy
  web/
    src/api/           # generated types and transport wrapper
    src/source/
    src/graph/
    src/review/
    src/evidence/
    src/shell/
  fixtures/
    rust/
    graphs/
    traces/
    contracts/
  benchmarks/
    manifests/
    workloads/
    reports/
  tests/
    integration/
    recovery/
    security/
    ui/
  docs/
    adr/
    schema/
    operations/
    milestones/
  xtask/
```

Keep crate count proportional to independent ownership and toolchain boundaries. Combine small implementation modules later if separate crates add cost without value. The compiler adapter must not force the main service onto its toolchain.

Only `frontend-ra` imports upstream analyzer internals. Only storage code knows physical row/segment formats. The UI consumes generated transport types, never compiler structures. Domain adapters cannot bypass provenance, authorization or publication validation.

Engineering rules: keep the service on a pinned supported Rust toolchain, run formatter/Clippy and TypeScript checks in CI, use typed errors at process/API boundaries, and document every `unsafe` block with its invariant. Prefer existing parsers, hashing, database, graph-layout and protocol libraries over new implementations. Track dependency licenses/advisories and generate required notices. Avoid workspace-wide dependency upgrades inside feature PRs.

Configuration is versioned and validated. Resource limits, artifact retention and trust permissions are explicit settings with safe defaults; no hidden environment-variable-only production controls. Structured logs and errors must identify the failing stage without exposing private source or credentials.

Proposed CLI acceptance interface, to be implemented rather than assumed to exist:

```bash
rustscope init --workspace /path/to/project --trust read-only
rustscope index --profile firmware --level semantic
rustscope serve --listen 127.0.0.1:7878
rustscope query callers --symbol '<opaque-definition-id>' --snapshot '<id>'
rustscope diff --before '<id>' --after '<id>' --profile firmware
rustscope doctor
rustscope gc --dry-run
```

The semantic command fails with an actionable context/trust error when inputs are unavailable; it must not silently grant execution. `doctor` checks toolchain compatibility, storage integrity, context completeness and resource limits without running repository commands in read-only mode.

## 20. Parallel Agent Workstreams

### 20.1 Staffing and rules

Use eight logical workstreams below. They are not instructions to launch eight agents regardless of available slots. With four concurrent slots, run the coordinator plus three compatible lanes, then rotate as described below.

The integration lead owns shared contracts, root manifests, dependency pins and release evidence. Every other agent owns a bounded directory/task and tests. All work is read-only outside the new product repository and explicitly approved build scratch.

Contracts are frozen at each milestone, not frozen forever. An agent proposing a contract change submits the producer/consumer impact and fixture changes to the integration lead before editing shared schemas.

### 20.2 Work packages

| Lane | Ownership | First deliverables | Dependencies | Acceptance |
| --- | --- | --- | --- | --- |
| W0 Integration/contracts | `model`, root build, API generation, ADRs | IDs, fact/status envelopes, canonical fixtures, compiling skeleton, integration harness | None | Round-trip fixtures; independent producer/consumer contract tests |
| W1 Source/build ingestion | `ingest`, read-only context discovery | Frozen source manifest, Cargo/explicit context adapters, dirty overlays | W0 input schema | Same bytes/config produce same IDs; no repository execution in read-only mode |
| W2 Semantic frontend | `frontend-ra` | Pinned adapter, items, direct calls, origins, unknown reasons | W0; W1 real inputs later | Hand-reviewed semantic fixtures; no upstream IDs outside adapter |
| W3 Storage/publication | `store` | SQLite/CAS baseline, adjacency indexes, atomic snapshots, GC | W0 | Crash/publication tests; forward/reverse consistency; benchmark fixture |
| W4 Query/diff | `query`, `server` query routes | Bounded callers/callees/search, pagination, source windows, semantic diff | W0 fixture backend; W3 integration later | Contract, authorization and budget tests; no fake complete negatives |
| W5 Human viewer | `web` | Source/graph/inspector, deep links, evidence categories, change view | W0 generated API and fixture server | U1/U2/U5 path, screenshot/accessibility/layout tests |
| W6 Deep analysis/evidence | `analysis`, `evidence`, compiler/domain adapters | MIR spike, trace/test schema adapter, metrics, state annotation prototype | W0; W2/W1 for compiler mapping | Explicit phases/artifact mapping; unknowns and timing/loss tests |
| W7 Qualification/operations | `tests`, `benchmarks`, scheduler/ops initial owner | Independent fixture expectations, baseline runner, security tests, resource dashboard | W0 skeleton; runs independently of UI | Reproducible reports; incremental-vs-clean check; resource enforcement |

For W7, operational implementation and independent acceptance review should be performed by different agents/reviewers when staffing permits. W7 must not be the only reviewer of its own scheduler/security changes.

### 20.3 Task-sized issues ready for assignment

| Task | Lane | Deliverable | Must demonstrate |
| --- | --- | --- | --- |
| T00 | W0 | ADRs for frontend, IDs, evidence, storage, trust | All consumer owners sign off on one fixture |
| T01 | W0 | Buildable skeleton and generated API types | Rust/TypeScript contract test in CI |
| T02 | W1 | Source manifest and overlay capture | Edit during capture cannot produce mislabeled snapshot |
| T03 | W1 | Explicit BuildContext import | Host and firmware cfg produce separate contexts |
| T04 | W2 | Syntax item extraction | Broken source and macro invocation fixtures remain browsable |
| T05 | W2 | Resolved direct-call extraction | Call site maps to correct same-named definition across crates |
| T06 | W3 | FactReader/Writer baseline | Index survives restart; both edge directions agree |
| T07 | W3 | Atomic publication and GC | Kill publisher at each step; active snapshot stays valid |
| T08 | W4 | Bounded neighborhood/source API | Budget, cursor, unknown and auth contract tests |
| T09 | W5 | Interactive source/graph slice | Click actual edge to actual source; share/reopen link |
| T10 | W7 | Independent semantic expectations | Deliberately wrong extracted target is caught |
| T11 | W2/W1 | Macro/cfg/build-context completeness | Missing generated inputs cannot look complete |
| T12 | W6 | Named-phase MIR extraction | Branch/return/unwind/drop fixture comparison |
| T13 | W4/W6 | Definition correspondence and change review | Rename ambiguity retained; inferred-type edit invalidates correctly |
| T14 | W7/W0 | Scheduler and resource isolation | Worker memory/timeout failure does not stop browsing |
| T15 | W6 | Trace/test evidence import | Wrong executable mapping rejected; loss and clock domains visible |
| T16 | W7 | Incremental equivalence and performance runner | Clean and incremental facts agree; raw timings retained |
| T17 | W3/W4 | Large-shard and fan-out qualification | S budget achieved or bottleneck documented with profile |
| T18 | W0/W3/W7 | Distributed catalog/shard service | O snapshots survive worker loss without mixed generations |
| T19 | W5/W7 | Human comprehension study | Correctness and timing compared with baseline tools |

Where a task names two lanes, assign one driver and one reviewer before starting. Do not have both agents edit the same implementation files concurrently.

### 20.4 What can start immediately

After the first shared fixture is agreed, W1 can build manifests, W2 can extract facts, W3 can index synthetic facts, W4 can query a fixture backend, W5 can consume the fixture API, and W7 can build independent tests. W6 can investigate compiler compatibility without blocking the basic viewer.

The viewer may use fixtures during development but its milestone acceptance requires real analysis output. Fixture-only progress is reported as such.

### 20.5 Four-slot execution plan

Wave A: W0 coordinator plus W1 input capture, W2 frontend spike and W5 fixture-backed viewer. W0 supplies the minimal fixture store/query server, not the full final backend.

Wave B: W0 plus W3 durable store, W4 query integration and W7 independent tests. Keep W2 available for integration defects; pause noncritical UI polish if a slot is needed.

Wave C: rotate W6 compiler/trace work alongside W2 language coverage and W7 qualification. Use measured blockers, not fixed agent equality, to choose which lane continues.

Eight available slots allow all lanes to advance, but shared-contract changes, frontend compatibility and final integration still form a critical path. More agents do not remove those dependencies.

### 20.6 Agent handoff template

```text
Task ID / owner / worktree:
Input schema and base commit:
Owned paths:
Deliverables:
Out of scope:
Acceptance fixtures and commands:
Observed results and artifact paths:
Open assumptions / unsupported cases:
Integration dependencies:
Next blocker and proposed smallest resolution:
```

An agent is done only when its changes, targeted tests, contract fixtures and handoff are available to the integration lead. "Implemented" without a running consumer is not "integrated."

## 21. Milestones and Timelines

Estimates assume experienced Rust/frontend implementers, an integration lead, available benchmark hosts, reusable dependencies and no requirement to invent a compiler. They are planning ranges, not completion guarantees. M0 must revise them using actual adapter and corpus results.

| Milestone | Cumulative planning window | Exit criteria |
| --- | --- | --- |
| M0: contracts and feasibility | First 4-8 working hours | Target corpus/config pinned; trust mode defined; adapter loads one fixture; canonical schema agreed; baseline recorded |
| M1: working vertical slice | First 1-3 working days | Real items/direct calls persisted, queried and visualized; source links/context/unknowns correct; restart works |
| M2: credible local analyzer | End of weeks 1-2 | Multi-crate/cfg/macro fixtures; snapshot publication; incremental-vs-clean tests; L navigation targets measured |
| M3: review and deeper flow | End of weeks 3-4 | Semantic diff, named-phase MIR for supported toolchains, test/trace overlays, MEC reading tasks, security isolation |
| M4: large single-node release | End of weeks 5-8 | S qualification, recovery/retention, load/cancellation budgets, UI usability study, supported toolchain policy |
| M5: organization-scale release | Approximately weeks 12-20 | O corpus and load targets, sharded query routing, authenticated ACLs, worker-loss recovery, operational rollout |
| M6: extreme-scale extension | Re-estimate after M5 | X capacity model and real/synthetic qualification; no implied date or scale guarantee |

M4 may ship a useful local product while M5 continues. M3 compiler support applies only to explicitly supported toolchains; unsupported builds retain A0/A1. Enterprise operational requirements can extend the schedule beyond M5 even when the core graph/query engine works.

### 21.1 An 8-24 hour accelerated prototype

Reasonable stretch scope: one known Cargo configuration, a pinned compatible analyzer, items/direct calls, immutable local index, bounded calls view, source links and honest unknowns. Work from a small real repository plus fixtures. Carry all IDs/provenance through even if most deeper capabilities are absent.

Suggested sequence:

1. Hours 0-2: contracts, build skeleton, corpus and no-execution policy.
2. Hours 2-8: analyzer extraction, minimal store/API and fixture-driven UI in parallel.
3. Hours 8-16: integrate real facts, source navigation, restart and targeted semantic tests.
4. Hours 16-24: measure latency, correct mapping issues, produce a recorded end-to-end demonstration and limitation report.

This is a prototype, not billion-fact qualification, a complete Rust analyzer, full macro support or security-hardened hosted execution. If the pinned frontend cannot be integrated in that window, the deliverable is a clearly labeled syntax-only slice plus the blocking evidence, not a claim of semantic completion.

### 21.2 Critical path

```text
Fact/context/identity contract
    -> working semantic extraction
    -> correct durable publication
    -> real query/viewer integration
    -> clean-vs-incremental equivalence
    -> measured scale qualification
```

MIR extraction, trace ingestion, UI refinement and corpus construction run alongside this path. Do not let an optional LLM explainer, state-machine inference or GPU experiment block the first credible local release.

### 21.3 Milestone acceptance evidence

Each milestone report includes commit/digests, supported capabilities, unsupported cases, exact test commands, pass/fail/skip totals, raw performance samples, peak memory/disk, screenshots or demo, risks and next actions.

No milestone passes on implementation percentage alone. M2 is not complete if correctness passes but queries silently return mixed contexts. M4 is not complete if only a replicated synthetic corpus was measured.

## 22. Integration, Monitoring and Rerouting

### 22.1 Integration policy

Use isolated worktrees/branches per bounded task. The coordinator merges small PRs in dependency order. Shared contracts and lockfiles have one owner. Do not ask every agent to independently upgrade dependencies or regenerate every package file.

Run the targeted failing fixture first, then unrelated fast checks in parallel where resources permit. Keep an always-runnable integration branch. Tests and benchmark artifacts must identify the integrated commit rather than a mixture of agent branches.

Suggested PR order: contracts/skeleton; input and extraction adapters; storage publication; query/viewer vertical slice; correctness/incremental expansion; compiler/evidence integrations; single-node performance; hosted/distributed service.

### 22.2 Monitoring cadence

During the first implementation day, the coordinator checks task dependencies and integration health every 30 minutes. During sustained work, use two-hour technical checkpoints and a daily milestone report. Set timers only when implementation actually starts; this document does not launch a background swarm or monitoring service.

Track:

| Dimension | Report |
| --- | --- |
| Delivery | Planned, implemented, integrated, independently verified tasks |
| Semantic coverage | Supported fixture families and explicit gaps by context |
| Correctness | Tests passed/failed/skipped; clean-vs-incremental mismatches |
| Performance | Current tier, p95, RSS, index size, index lag, baseline comparison |
| Human utility | U1-U10 workflows demonstrated and comprehension results |
| Risk | Critical-path blocker, owner, evidence, next experiment and ETA range |

Avoid a single "90% done" number spanning a prototype and organization-scale production. If a stakeholder requires a percentage, calculate passed acceptance checks within a named milestone and show unweighted counts and release blockers alongside it.

### 22.3 Rerouting triggers

- Frontend integration blocked for half a day: assign a second reviewer to a minimal failing fixture; continue UI/store with agreed fixture data.
- Schema changes repeatedly break consumers: pause feature expansion until one contract fixture passes end to end.
- Semantic mismatch: reduce to the smallest source/configuration case before adding graph optimizations.
- Memory budget exceeded: profile active crate/dependency closure; reduce admitted workers; do not hide omissions.
- Query misses target: capture plan and shard/fact counts before changing databases.
- UI stalls: inspect payload/layout sizes and grouping before rewriting the renderer.
- More than two days without an integrated demo: redirect one feature lane to integration.
- Security isolation unavailable: disable executable analysis and ship read-only/import mode rather than bypassing the boundary.

## 23. Risk Register and Decisions

| Risk | Impact | Mitigation / decision owner |
| --- | --- | --- |
| Upstream semantic API changes | Repeated adapter breakage | Pin revision; one adapter; compatibility fixtures; W2/W0 |
| Unsupported compiler/toolchain | Missing MIR | Capability negotiation; separate adapters; preserve A0/A1; W6 |
| Incorrect build context | Plausible but wrong graph | Context manifests, cross-target fixtures, explicit unknowns; W1/W2 |
| Unsound incremental reuse | Stale answers | Clean-vs-incremental oracle, conservative invalidation; W2/W7 |
| Symbol identity drift | Wrong diff/evidence transfer | Snapshot-local IDs and reviewed correspondences; W0/W4 |
| Generic/macro explosion | Excess memory/time | Per-worker budgets, staged analysis, partial status; W2/W7 |
| One enormous crate | Aggregate sharding does not help | Separate single-crate capacity test and limits; W2 |
| Graph visual overload | Poor comprehension | Bounded neighborhood, grouping, source-first workflow; W5 |
| Unsupported path inference | False certainty | Separate graph families and evidence; W4/W6 |
| Trace mismatch or clock confusion | False diagnosis | Artifact mapping, stream order and uncertainty; W6 |
| Unbounded retained artifacts | Disk exhaustion | Quotas, category accounting, pin-aware GC; W3/W7 |
| Multi-tenant data leakage | Confidentiality failure | ACL-first queries and cache keys; no cross-tenant dedup; W0/W7 |
| Custom storage too early | Lost schedule and correctness | SQLite baseline, measured replacement gate; W3 |
| Agent integration contention | Many unmergeable branches | Ownership, frozen milestone contracts, runnable branch; W0 |
| LLM summaries overstate behavior | Misleading explanations | Source citations, explicit interpretation, opt-in; W6/W5 |

### 23.1 Decisions that must be made during M0

1. Exact rust-analyzer revision and first supported compiler/target combinations.
2. Initial corpus, actual largest crate, feature profiles and dependency availability.
3. Canonical identity/fact/evidence schemas and no-execution trust behavior.
4. First UI/editor/graph dependency pins and license compatibility.
5. Benchmark hardware, baseline commands and which L targets are realistic.
6. Single-node storage format v1 and publication/GC invariants.
7. Named integration owner and active agent concurrency budget.

### 23.2 Deferred decisions with explicit triggers

- Compact adjacency storage: decide after S profiling, not from intuition.
- Distributed deployment: begin when S limits or multi-user requirements justify it; O qualification remains mandatory for scale claims.
- WebGL renderer: decide only after bounded Canvas layouts miss interaction targets.
- LLM explanations: begin after deterministic evidence navigation passes U1/U2/U5.
- Context-sensitive interprocedural analysis: require a concrete question and precision/performance budget.
- Replacing portions of the Rust semantic frontend: require a measured bottleneck and a differential correctness corpus for that exact replacement.

## 24. Operations and Release Definition

Ship a versioned binary or container, a browser bundle, schema compatibility metadata, dependency notices and a reproducible installation path. Pin build inputs and publish checksums. Do not require users to compile the full analyzer dependency graph just to open an exported index.

Operational documentation must cover disk sizing, quotas, backup/restore, toolchain installation, artifact acquisition, snapshot migration, GC, worker failures, partial-analysis troubleshooting and safe removal of store-owned data.

Back up the catalog and referenced immutable manifests/objects consistently. Verify restore by answering pinned queries and checking evidence links, not merely starting the database. A migration publishes a new readable generation; it must not irreversibly rewrite the only copy of qualified evidence.

Offline export contains selected source/facts/evidence according to permissions, an index manifest and a viewer-compatible format. Prefer an explicit local viewer/server for active queries. Any static HTML export must disclose omitted interactive capabilities and sanitize embedded content.

### 24.1 Local release is done when

- A new user can install, open a workspace safely and complete U1/U2/U5.
- Selected build context is visible and accurate, including macro/dependency limitations.
- Facts survive restart; source links, IDs and before/after diffs are reproducible.
- No known critical clean-vs-incremental mismatches remain in the supported corpus.
- Unknown, partial, failed and complete states are distinguishable throughout the UI/API.
- The named L/S performance tier passes on recorded hardware.
- Worker crashes, cancellation and disk limits preserve browsability and index integrity.
- Accessibility, privacy, dependency licensing and defensive isolation checks pass for the shipped modes.
- Documentation states exactly which compiler, language and deployment capabilities are unsupported.

### 24.2 Organization-scale release additionally requires

- O-scale real/synthetic corpus results, with their different roles disclosed.
- Authenticated repository-level authorization on all query/artifact paths.
- Tenant quotas, bounded shard routing, admission control and predictable overload behavior.
- Worker/shard loss, restart, backup/restore and catalog recovery drills.
- No catalog/network-filesystem shortcuts that violate storage consistency assumptions.
- A supported upgrade path, on-call diagnostic information and cost/capacity measurements.

## 25. Implementation Kickoff Checklist

1. Create the new repository; do not modify the firmware repository as part of setup.
2. Read this specification and record accepted deviations as ADRs.
3. Assign W0 and select the actual concurrent lanes; create task issues T00-T19.
4. Pin tools and corpus revisions; record access/license constraints without copying secrets.
5. Implement one canonical fixture from source/context to facts to query response.
6. Make the viewer consume that exact response with source and evidence navigation.
7. Replace fixture facts with real adapter output and retain the fixture as a contract test.
8. Add persistence/restart and incomplete-analysis handling before expanding features.
9. Establish the independent semantic and clean-vs-incremental gates.
10. Measure the baseline; adjust targets transparently if necessary.
11. Ship the first useful local slice; add deeper analysis and larger tiers through measured milestones.

## 26. Final Recommendation

The valuable new product is not another Rust parser. It is a durable, inspectable model of code and evidence, a fast way to query that model, and a viewer that helps a human reason about a small relevant part of a large system.

Build the trustworthy source-to-fact-to-viewer path first. Keep compiler semantics, persistence and presentation independent. Make uncertainty visible. Demonstrate faster comprehension and bounded resource use with actual workloads before claiming large-scale or correctness advantages.
