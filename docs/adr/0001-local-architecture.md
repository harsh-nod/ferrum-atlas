# 0001: Local evidence-first implementation

Status: accepted for the first local delivery.

The first integrated milestone uses Rust 1.97.1, exact rust-analyzer 0.0.349 adapters, SQLite immutable snapshots and a WAL catalog, Axum, and a React/TypeScript viewer. The intended dependency boundary puts analyzer internals only in frontend-ra. Cargo manifests are parsed as data. Analyzed source never executes in read-only mode.

Current boundary variance: the later source-maintainability and state-machine
analyses directly use the same pinned `ra_ap_syntax` and `ra_ap_parser` libraries
inside `atlas-analysis`. No upstream syntax types enter transport contracts, but
this does not satisfy the specification's single-adapter dependency rule.
Moving these traversals behind an owned frontend interface remains required and
is tracked in [issue 21](https://github.com/harsh-nod/ferrum-atlas/issues/21).

Canonical JSON uses ordered struct fields and BTreeMap keys, domain-separated SHA-256, and schema version 1. IDs are typed and snapshot/context-local. Source text retains exact UTF-8 and CRLF bytes; unsupported encodings are reported. Offsets are u32 (bounded source files) and timestamps use decimal strings.

Initial persistence uses indexed immutable SQLite files and source content hashes. SQLite search is the measured baseline; Tantivy, delta shards, and distributed routing require a workload justifying them. Full conservative re-analysis of changed snapshots is the correctness baseline before finer reuse.

Semantic support requires complete captured inputs for each claimed target. Unsupported sysroot/dependencies, macros, indirect calls and compiler flow remain explicit. A0 remains useful if A1 cannot be supported for a construct. A source-flow view is never labeled MIR or a complete compiler CFG.

The CLI and browser target local read-only use. Per-session authorization,
loopback binding and origin checks are mandatory. Hostile executable isolation
and remote binding remain unavailable. The later separate pinned compiler
adapter permits explicit trusted-local compilation; it is not a hostile-code
sandbox and never runs automatically from the viewer or read-only capture.

The multi-week specification is a roadmap. Milestone reports enumerate accepted and open gates; performance targets and human studies require measurements and participants before being claimed.
