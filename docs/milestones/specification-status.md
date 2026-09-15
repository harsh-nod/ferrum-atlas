# Specification Status

This is the current implementation inventory, not a declaration that the entire
specification is complete. The original document combines local product work,
multi-week language and deployment engineering, named scale qualifications,
and experiments requiring actual human participants and domain reviewers.
Missing implementation and missing qualification are distinguished below.

## Implemented Local Workflows

The authenticated loopback application opens captured source without executing
the analyzed repository. It preserves exact source/context identities, supported
local semantic calls, explicit unknowns, immutable snapshots, bounded graph and
source queries, change review, and independently imported observations.

The extended implementation adds:

- A separately built exact-nightly compiler adapter producing typed MIR, checked
  against captured source and context before import, CFG navigation, and bounded
  whole-local reaching definitions with explicit unknown-memory effects.
- Warm rust-analyzer watch sessions, clean-versus-warm edit regressions, durable
  local jobs, cancellation, worker restart and retained progress events.
- Pin-aware retention, reviewed exact GC plans, reader leases, concurrent/crash
  recovery tests, portable static snapshots and source-independent restoration.
- SCCs, selected path/reachability analysis, metrics and independent-clock trace
  alignment with explicit first-observed-divergence semantics.
- Immutable-object verification caching and explicit cold preparation, retaining
  corruption checks and cancellation. Source reads, UTF-8 validation, checksums
  and window scanning cooperate with the same request control.
- Snapshot-retaining browser bookmarks with recoverable pin failures; desktop,
  mobile, keyboard, real canvas-pixel and actual 200% Chromium zoom tests.
- Opt-in syntactic state-transition candidates with exact guard/action spans,
  bounded unknowns and immutable digest-bound reviewer declarations. Reviews
  retain their snapshots and never upgrade candidates into proof.
- Exact source jumps from dataflow uses and reaching definitions, versioned
  lexical/AST maintainability measures, and a separately defined compiler CFG
  cyclomatic measure with explicit synthetic-exit and unreachable-block rules.
- Deterministic prerelease archive packaging, bounded validation, actual bundled
  dependency notices, packaging-time ELF inspection and a verified cloud-built
  CLI/browser artifact. Distribution review gates remain explicit.

These are bounded local capabilities. They do not establish complete Rust
semantics, executable-code isolation or organization-scale deployment.

## Requirements Matrix

| Specification | Implemented scope | Remaining implementation or acceptance |
| --- | --- | --- |
| 3: Human workflows | Source-linked exploration, flow, change/evidence/health views and live browser regressions | Actual counterbalanced study with at least eight consenting people; domain-reviewed MEC reading tasks |
| 4, 18: Scale/performance | Frozen real-corpus and synthetic raw measurements, checked result usefulness, resource limits and percentile validation | Named L/S/O/X tiers, optimized controlled baselines, concurrent load, cost/capacity and qualified browser frame budgets; exported per-stage spans and operational queue/cache/cancellation/bytes-per-fact/snapshot-lag metrics |
| 5-6: Frontends/context | Captured syntax, supported rust-analyzer local semantics, explicit cfg/dependency limitations, optional exact compiler import | Full Cargo build-unit/feature unification, sysroot/dependency acquisition, generated code and proc-macro execution |
| 7-8: Facts/identity | Versioned identities, evidence categories, exact UTF-8 spans, queryable unknowns and generated transport contracts | Broader inferred-type/dispatch semantics; compatibility qualification beyond supported inputs |
| 9: Persistence | Immutable single-node shards, transactional head CAS, checksums, reader leases, retention and portable source/fact/observation import/export | Fine-grained reusable delta shards, routed reverse postings, distributed catalog, supported schema-generation migration and portable compiler/review evidence export |
| 10: Incremental/jobs | Warm semantic database, refreshed extraction, deterministic edit sequences, bounded durable serial local jobs | Persistent sub-crate delta reuse and distributed scheduling; broad analysis jobs are not durable background jobs |
| 11-12: Queries/analysis | Scoped bounded search/source/graphs/diffs, call-graph SCC/path/metrics, selected MIR dataflow | Package/import dependency graphs and their SCCs, dependency/subsystem graph collapse, organization routing and broader supported state-machine syntax |
| 13: Viewer | Five principal views plus analysis, source/edge/dataflow-point navigation, tables, worker layout, history and retention | Richer graph clustering, incremental local relayout and full accessibility certification |
| 14: Change/comprehension | Structural signature/body/context delta, direct impact candidates, lexical tokens/noncomment lines/nesting/unsafe boundaries, selected compiler CFG complexity, state-transition review and pinned reading notes | Rename/move and inferred-type/effect/dependency/test deltas; public API-surface/reexport, structural duplication and history metrics; typed contracts and imported proof results |
| 15: Tests/traces | Exact artifact imports, eight outcomes, wide counters, independent clocks, loss and bounded anchor comparisons | Debug/address/load-map/inline-frame symbolization, richer run-input/log/payload contracts, replay integration and real MEC acceptance |
| 16: Security | No repository execution in read-only mode; loopback authorization/origin/host checks, bounded inputs, descriptor-based storage and defensive tests | Hostile executable OS isolation and dependency broker; authenticated hosted identities/ACLs, tenant quotas/encryption/audit and multi-tenant isolation |
| 17: Correctness | Independent hand-authored semantic expectations, faulty-producer mutations, graph/dataflow oracles, process crash/recovery and browser tests | Expanded language corpus, compiler-version matrix and deployment-scale failure drills |
| 19-23: Delivery | Public repository created before implementation, isolated agent worktrees, focused commits, CI and tracked task issues | Source metric/state-machine traversals still directly import the pinned analyzer parser/syntax crates outside frontend-ra; restoring the prescribed adapter dependency boundary and remaining roadmap acceptance gates stay open |
| 24: Operations | Local limits, recovery, retention, consistent stopped-writer backup, portable restore, toolchain and trust documentation | Qualified local release criteria and organization-scale release remain unmet; portable restore omits compiler imports and state reviews, which require whole-store backup; packaging is not qualification |

Optional interprocedural dataflow and optional LLM explanations are not silently
presented as implemented. No hosted endpoint exposes captured source. Trusted-local
compiler execution is explicit and is not an executable-code sandbox.

## Review And Evidence

Cross-agent review covers the Rust crates, separate compiler adapter, browser,
fixtures, generated contracts, CI/release scripts and operational documentation.
The primary agent integrates and reruns cross-module tests. Review fixes include
snapshot-readiness eviction, withheld-dataflow counts, source cancellation,
descriptor-relative evidence I/O, private bounded locks, strict compiler
invocations and cfg/context matching, retained bookmark failures, and journal
validation. The final review also corrected hidden valid no-exit CFG records,
compiler-load cancellation, retained raw-buffer memory and archive-extension
headers that could be processed before size validation. A review is not proof
of defect absence.

[Qualification evidence](qualification-next.md) preserves preliminary failures
as well as successful cache/preparation follow-ups. All real regex relations in
that corpus remain unknown; useful search results do not establish resolved-call
accuracy. No study observations have been fabricated. The human-study validator
rejects the empty template.

The [initial implementation report](local-slice.md) is a historical checkpoint
with its original test counts. [Dependency notes](../dependencies.md) distinguish
advisory checks, distributed notices and the remaining release license review.

## Completion Evidence

The application and packaging integration at
`1e19f972c1aedf85ad76999c50784ab5f9688b49` passed
[cloud CI](https://github.com/harsh-nod/ferrum-atlas/actions/runs/34956569046),
including the exact-nightly compiler job. Local verification was repeated after
the compiler-evidence cancellation fixes and final viewer changes:

- Workspace Rust tests: 278 passed; two adapter-dependent tests were initially
  ignored and then explicitly invoked against the pinned adapter, both passing.
- Separate compiler adapter: 18 tests passed in the pinned-compiler CI job.
- Browser: 91 tests passed with the real pinned compiler enabled, including
  actual capture/import/HTTP/metrics/review/restart, desktop/mobile, source-byte
  selection, nonblank canvas checks, cancellation races and 200% Chromium zoom.
- Packaging: 31 Python regressions passed after the documentation follow-up,
  including compatibility checks for older candidates. Benchmark/study validation: 14 tests
  passed; all five checked benchmark reports validated without qualifying a tier.
- Workspace Clippy with warnings denied, formatting, generated TypeScript
  verification and the production viewer build passed. A refreshed root lockfile
  audit matched zero advisories; dependency scope and remaining caveats are in
  the linked dependency inventory.

The [package evidence](package-verification.md) identifies exact downloaded
archive hashes, source commits and actual CLI/browser checks. Later source
commits are never implicitly covered by an older archive hash. Full local
application checks also measured exact fixture source counts and inspected
[desktop source metrics](../images/source-metrics-desktop.png),
[mobile source metrics](../images/source-metrics-mobile.png), and
[compiler complexity](../images/compiler-complexity-desktop.png).

Do not infer full-spec completion from these results: the engineering gaps and
external experiments in the matrix remain required. No scale tier, human study,
hostile-code sandbox, hosted release or license-compliance certification is claimed.
