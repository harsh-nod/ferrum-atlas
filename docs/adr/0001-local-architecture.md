# 0001: Local evidence-first implementation

Status: accepted for the first local delivery.

The first integrated milestone uses Rust 1.97.1, exact rust-analyzer 0.0.349 adapters, SQLite immutable snapshots and a WAL catalog, Axum, and a React/TypeScript viewer. Only frontend-ra imports analyzer internals. Cargo manifests are parsed as data. Analyzed source never executes in read-only mode.

Canonical JSON uses ordered struct fields and BTreeMap keys, domain-separated SHA-256, and schema version 1. IDs are typed and snapshot/context-local. Source text retains exact UTF-8 and CRLF bytes; unsupported encodings are reported. Offsets are u32 (bounded source files) and timestamps use decimal strings.

Initial persistence uses indexed immutable SQLite files and source content hashes. SQLite search is the measured baseline; Tantivy, delta shards, and distributed routing require a workload justifying them. Full conservative re-analysis of changed snapshots is the correctness baseline before finer reuse.

Semantic support requires complete captured inputs for each claimed target. Unsupported sysroot/dependencies, macros, indirect calls and compiler flow remain explicit. A0 remains useful if A1 cannot be supported for a construct. A source-flow view is never labeled MIR or a complete compiler CFG.

The CLI and browser target local read-only use. Per-session authorization, loopback binding and origin checks are mandatory. Executable analysis and remote binding are unavailable until isolation and authorization have acceptance evidence.

The multi-week specification is a roadmap. Milestone reports enumerate accepted and open gates; performance targets and human studies require measurements and participants before being claimed.
