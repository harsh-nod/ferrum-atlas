# Local Implementation Report

Historical checkpoint. See the [current specification inventory](specification-status.md)
and [expanded qualification evidence](qualification-next.md) for later work.

Validated on 2026-09-15 UTC against implementation commit [`2cd55d2`](https://github.com/harsh-nod/ferrum-atlas/commit/2cd55d2cf907214b2cbc30fb4563c9d0810d2482). This report documents the working local, read-only implementation. It does not declare the entire [specification](../../rust_analyzer_and_visualizer.md) complete.

The public repository was created before implementation. Separate agents implemented and reviewed the frontend adapter, storage/query engine, and browser; the primary agent integrated changes, implemented the CLI/server/evidence pipeline, and ran cross-module verification. Progress was published in incremental commits.

## Verified Results

The [hosted CI run](https://github.com/harsh-nod/ferrum-atlas/actions/runs/34928513124) passed all steps, including formatting, Clippy, Rust tests, generated-contract checks, production browser build, Chromium tests, real pilot capture, integrity checking, and a 30-sample benchmark. CI retains its browser and benchmark artifacts.

Local verification also passed:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --locked --workspace --all-targets -j2 -- -D warnings` | Passed |
| `cargo test --locked --workspace -j2` | 78 test entries passed, including two subprocess-helper entrypoints |
| `cargo test --locked --manifest-path fixtures/rust/pilot/Cargo.toml -j2` | One pilot integration test passed |
| `cargo run --locked -p xtask -- check-types` | Generated TypeScript matches Rust contracts |
| `npm --prefix web run build` | TypeScript and production build passed |
| `npm --prefix web test` | 23 Playwright tests passed |
| `git diff --check` | Passed |

The browser suite combines deterministic API fixtures with a real CLI-to-HTTP-to-browser workflow. The live test captures source, imports explicitly synthetic observations, navigates the graph, edits the fixture, creates another snapshot, and verifies restart behavior. Mock responses alone are not the end-to-end acceptance evidence.

## Coverage

| Area | Exercised behavior |
| --- | --- |
| Source capture | Exact bytes and identities, bounded traversal, no-follow descriptors, changed roots, deterministic captures, no workspace execution |
| Rust extraction | HIR-based direct calls, same-named definitions, aliases, local dependencies, methods, recursion, closures, three-state cfg, unavailable macros, indirect targets, broken source, Unicode and CRLF |
| Persistence | Immutable shards, forward/reverse indexes, compare-and-swap publication, actual abrupt process exits at publication boundaries, restart, checksum corruption, integrity reports |
| Queries | Snapshot/context authorization, signed cursor scope and expiry, bounded graph/source/diff results, cancellation, repeated-callsite evidence, incomplete impact results |
| Worker lifecycle | Resource bounds, failed-job preservation of previous snapshots, stale-parent rejection, stopped-worker cancellation and actual parent-death termination |
| HTTP | Bearer authentication, loopback/Host/Origin restrictions, typed errors, request and response bounds, static asset serving |
| Imported evidence | Exact artifact/source/context mapping, all eight outcomes, timeout caps, wrong mapping rejection, independent clocks, wide decimal counters, loss, concurrent bundle admission |
| Browser | Real canvas pixels and edge clicks, source selection, deep links, history, reading trails, stale request rejection, unavailable pins, errors distinct from empty results, review views, focus-trapped mobile drawers |

Responsive browser checks use 1440x900, 1024x768, 390x844 and 720x450 CSS-pixel viewports. The small landscape viewport is not a claim of an actual operating-system zoom test or an accessibility certification.

## Review Fixes

Cross-agent review and integration checks produced focused fixes and regression tests for:

- CodeMirror positions after CRLF normalization and non-BMP Unicode characters.
- Shared source instantiated in different crates or modules, and unavailable attribute macros.
- Evidence pagination losing distinct records under repeated call sites.
- Impact truncation and after-snapshot-only caller analysis being incorrectly interpreted as complete.
- Coverage-count overflow and duplicated limitations during diff composition.
- Missing pinned edges incorrectly falling back to a different source location.
- Change baselines accidentally selecting another repository.
- Loading and failed provenance being rendered as empty evidence.
- Unknown graph boundary nodes exceeding the total canvas budget.
- ELK worker initialization, resize framing, and narrow-screen graph orientation.
- Mobile search focus, keyboard-triggered drawer closure, and stale response handling.

Review covered the implementation, transport types, fixtures, browser tests, build configuration, CI, and operational documentation. Review and tests reduce risk; they do not establish the absence of defects. [Dependency notes](../dependencies.md) record the lockfile audits and license review limitations.

## Running-App Captures

These images were taken from the production browser build served by the real local API, not mocked endpoints. The demonstration store contains two captured revisions of the bundled pilot: the second changes `value + 1` to `value + 2`. The tracked pilot remains unchanged. Observations are prominently labeled synthetic and are not proof that any test or device workload executed. Authentication credentials and private store contents are not published.

- [Desktop source and call graph](../images/explore-desktop.png)
- [Mobile source-linked graph](../images/explore-mobile.png)
- [Before/after change review](../images/changes-desktop.png)
- [Synthetic observation display](../images/evidence-desktop.png)

All four capture workflows completed without browser page errors.

## Performance Scope

The committed [raw local smoke report](../../benchmarks/reports/local-smoke.json) contains 30 symbol-search samples for a seven-definition, seven-relation pilot on Linux x86_64 with 32 logical CPUs. The local debug build measured p50 7.522 ms and p95 10.765 ms. Cache state was uncontrolled and the first sample was retained.

This tiny fixture does not establish cold-index speed, memory scaling, large-corpus behavior, p99 latency, or the specification's L/S/O targets. The exact snapshot, context, coverage limitations, hardware summary and individual samples are retained in the report.

## Acceptance Status

The following initial task scopes have executable acceptance evidence: architecture and shared pilot (T00), build/contracts (T01), frozen source capture (T02), syntax/broken-source browsing (T04), supported local direct calls (T05), durable indexed snapshots (T06), bounded local API (T08), interactive viewer (T09), and artifact-matched evidence import (T15). These are bounded capabilities, not unrestricted language or deployment support.

The [remaining issues](https://github.com/harsh-nod/ferrum-atlas/issues) preserve the broader specification gates:

| Task | Implemented portion | Remaining gate |
| --- | --- | --- |
| T03 | Explicit imported contexts, named profiles, cfg and feature selection | Full Cargo feature unification, host/target build-unit discovery and dependency metadata |
| T07 | Atomic publication, interruption recovery, doctor, GC dry-run | Pin-aware retention qualification and deletion |
| T10 | Hand-authored semantic expectations and independent review | Broader fixture qualification and deliberate faulty-producer validation |
| T11 | Unknown cfg and missing macro/generated inputs remain visibly incomplete | Expansion support and supported-language completeness qualification |
| T12 | Explicitly unsupported compiler capability | Pinned MIR adapter and compiler CFG semantics |
| T13 | Structural body/API/configuration comparison and bounded direct impact candidates | Inferred-type changes, richer rename/move correspondence, former/transitive callers |
| T14 | Read-only worker bounds, timeout, kill/reap and parent-death handling | Separately tested OS isolation before executable analysis |
| T16 | Deterministic full reanalysis and retained raw smoke measurements | Fine-grained incremental invalidation, watcher/delta shards and clean/incremental equivalence qualification |
| T17 | Bounded local queries and pilot smoke run | Real and synthetic large single-node corpus qualification |
| T18 | No distributed implementation | Tenant ACLs, distributed catalog, worker-loss recovery and organization-scale qualification |
| T19 | No human study conducted | Representative participants, counterbalanced tasks and measured comprehension outcomes |

Source-flow points are not compiler MIR or a true compiler CFG. The browser/API are loopback-only, not a hosted multi-user service. Linux resource containment is not a sandbox for running hostile code. Evidence imports validate consistency and artifact identity, not the truth of a producer's claims. No large-scale, security, or human-comprehension certification is asserted.

See [local operations](../operations/local.md) for limits, backup/restore, trust, evidence provenance and unsupported capabilities.
