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

State-transition inference/review and release packaging are integration work in
this checkpoint. Their final verification is recorded in the completion evidence
section below once the integrated suites pass; existence of source files alone
does not establish a shipped capability.

## Requirements Matrix

| Specification | Implemented scope | Remaining implementation or acceptance |
| --- | --- | --- |
| 3: Human workflows | Source-linked exploration, flow, change/evidence/health views and live browser regressions | Actual counterbalanced study with at least eight consenting people; domain-reviewed MEC reading tasks |
| 4, 18: Scale/performance | Frozen real-corpus and synthetic raw measurements, checked result usefulness, resource limits and percentile validation | Named L/S/O/X tiers, optimized controlled baselines, concurrent load, cost/capacity and qualified browser frame budgets |
| 5-6: Frontends/context | Captured syntax, supported rust-analyzer local semantics, explicit cfg/dependency limitations, optional exact compiler import | Full Cargo build-unit/feature unification, sysroot/dependency acquisition, generated code and proc-macro execution |
| 7-8: Facts/identity | Versioned identities, evidence categories, exact UTF-8 spans, queryable unknowns and generated transport contracts | Broader inferred-type/dispatch semantics; compatibility qualification beyond supported inputs |
| 9: Persistence | Immutable single-node shards, transactional head CAS, checksums, reader leases, retention and portable import/export | Fine-grained reusable delta shards, routed reverse postings, distributed catalog and supported schema-generation migration |
| 10: Incremental/jobs | Warm semantic database, refreshed extraction, deterministic edit sequences, bounded durable serial local jobs | Persistent sub-crate delta reuse and distributed scheduling; broad analysis jobs are not durable background jobs |
| 11-12: Queries/analysis | Scoped bounded search/source/graphs/diffs, SCC/path/metrics, selected MIR dataflow | Dependency/subsystem graph collapse, organization routing and broader supported state-machine syntax |
| 13: Viewer | Five principal views plus analysis, source/edge navigation, tables, worker layout, history and retention | Richer graph clustering, incremental local relayout, dataflow-to-source point navigation and full accessibility certification |
| 14: Change/comprehension | Structural signature/body/context delta, direct impact candidates, source metrics and pinned reading notes | Rename/move and inferred-type/effect/dependency/test deltas; nesting/token/CFG-complexity/duplication/history metrics; typed contracts and imported proof results |
| 15: Tests/traces | Exact artifact imports, eight outcomes, wide counters, independent clocks, loss and bounded anchor comparisons | Debug/address/load-map/inline-frame symbolization, richer run-input/log/payload contracts, replay integration and real MEC acceptance |
| 16: Security | No repository execution in read-only mode; loopback authorization/origin/host checks, bounded inputs, descriptor-based storage and defensive tests | Hostile executable OS isolation and dependency broker; authenticated hosted identities/ACLs, tenant quotas/encryption/audit and multi-tenant isolation |
| 17: Correctness | Independent hand-authored semantic expectations, faulty-producer mutations, graph/dataflow oracles, process crash/recovery and browser tests | Expanded language corpus, compiler-version matrix and deployment-scale failure drills |
| 19-23: Delivery | Public repository created before implementation, isolated agent worktrees, focused commits, CI and tracked task issues | Remaining roadmap issues retain their open acceptance gates |
| 24: Operations | Local limits, recovery, retention, consistent stopped-writer backup, portable restore, toolchain and trust documentation | Qualified local release criteria and organization-scale release remain unmet; packaging is not qualification |

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
validation. A review is not proof of defect absence.

[Qualification evidence](qualification-next.md) preserves preliminary failures
as well as successful cache/preparation follow-ups. All real regex relations in
that corpus remain unknown; useful search results do not establish resolved-call
accuracy. No study observations have been fabricated. The human-study validator
rejects the empty template.

The [initial implementation report](local-slice.md) is a historical checkpoint
with its original test counts. [Dependency notes](../dependencies.md) distinguish
advisory checks, distributed notices and the remaining release license review.

## Completion Evidence

Integrated final verification is pending the state-review and packaging handoffs.
Do not infer full-spec completion from a green build: external experiments and
the unimplemented matrix entries above remain required.
