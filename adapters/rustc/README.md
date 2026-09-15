# Pinned Compiler Adapter

This optional, standalone adapter extracts machine-readable compiler facts through
`rustc_driver` and `rustc_middle`. It does not parse diagnostic text or human MIR
dumps. It is a separate Cargo workspace and does not change the main service's
toolchain or read-only indexing behavior.

## Supported Producer

| Setting | Pinned value |
| --- | --- |
| Toolchain | `nightly-2026-04-03` |
| Rust release | `1.96.0-nightly` |
| Compiler commit | `55e86c996809902e8bbad512cfb4d2c18be446d9` |
| Compiler date | `2026-04-02` |
| Host and target | `x86_64-unknown-linux-gnu` |
| LLVM | `22.1.2` |
| MIR query and phase | `optimized_mir`, `Runtime(Optimized)` |
| Interchange phase name | `runtime_optimized` |
| MIR optimization level | `0` |
| Code optimization level | `0` |
| Overflow checks | Enabled |
| Panic strategy | Explicit `unwind` or `abort` |
| Crate kind | One library crate |

The build script rejects every other compiler commit. The pinned rustup file
requests `rustc-dev`, `llvm-tools`, `rust-src`, Clippy, and rustfmt. Cargo dependencies
have a separate committed lockfile. The executable embeds the selected sysroot
and a runtime linker path for its compiler libraries; it is not a portable binary
independent of that installed toolchain.

The selected phase is after runtime lowering and drop elaboration, even with
`mir-opt-level=0`. It is neither borrow-checker MIR nor a source-level CFG. Types
are included as display-only strings; consumers must not parse them as a machine
contract. Statement and terminator kinds, local accesses, edges, and unwind
actions are translated from the matching compiler's typed APIs.

## Build and Run

Run these commands from this directory so rustup selects this workspace's
toolchain, not the main workspace's toolchain:

```sh
rustup show
cargo build --locked -j2
cargo test --locked -j2
cargo clippy --locked --all-targets -j2 -- -D warnings
target/debug/atlas-rustc --trusted-local \
  --root fixtures --source control_flow.rs \
  --crate-name fixture --panic unwind --output /tmp/atlas-mir.json
```

The output must not already exist. `--root` identifies the allowed source tree;
`--source` is its crate root. `--edition` accepts 2015, 2018, 2021, or 2024 and
defaults to 2021. Repeated `--cfg` arguments record explicit selections, including
quoted feature predicates when supplied by the caller. There is no arbitrary
rustc-argument passthrough and no implicit Cargo configuration discovery.

Compilation stops after analysis and MIR extraction. No object, library, or
executable artifact is produced. A JSON file is published with an atomic,
no-clobber rename only after successful extraction and serialization. Compiler
errors remain JSON diagnostics on stderr and do not publish an empty success.

## Trust Boundary

`--trusted-local` is mandatory. This is an explicitly trusted compiler invocation,
not an OS sandbox and not a service endpoint for hostile source. All inherited
environment variables are removed before rustc starts threads; `env!` cannot
read inherited secrets. The invocation uses the embedded toolchain rather than
repository-specified wrappers, Cargo configuration, or toolchain selectors.

The adapter does not orchestrate Cargo, dependency acquisition, build scripts,
or user-supplied procedural macro artifacts. Rustc can still perform compile-time
evaluation and use libraries from the installed sysroot, including its compiler
support crates. This is not a claim that compilation executes no code. Toolchain
integrity and OS isolation are the responsibility of the trusted caller.

The file loader checks canonical source paths against `--root`, rejects non-UTF-8
paths, and caches the first bytes actually read for repeated reads. Source files,
modules, and `include!`/`include_str!`/`include_bytes!` inputs read by that loader
are hashed. There is no promise of hostile-filesystem race resistance: use a
frozen read-only input mount when that guarantee is required. Missing or
outside-root source inputs fail compilation.

Limits are 4 MiB per input file, 64 MiB total captured bytes, 4,096 captured files,
2,000 selected bodies, 20,000 blocks, 200,000 statements, 100,000 locals, and
64 MiB serialized output. Exceeding a limit fails the bundle instead of labeling
truncated facts complete. These are adapter input/output limits, not a bound on
rustc's internal query memory, compilation time, or CPU consumption. A caller
requiring resource containment must supply tested OS limits and cancellation.

## Interchange

`crates/model/src/compiler.rs` is the canonical serde and TypeScript transport
contract. The adapter includes that exact source module to keep the toolchain
workspace independent without duplicating the schema. Structs reject unknown
JSON fields. `CompilerBundle.schema_version` is `1`.

- `compiler` records the exact compiler and adapter identity.
- `inputs` records sorted captured file paths, raw-byte SHA-256 hashes and byte
  lengths, crate root/name, edition, target, panic strategy, the exact rustc
  argument vector, empty-environment policy, and `trusted_local` mode.
- `inputs.compiled_artifact` is null because extraction does not produce an
  executable. Compiler input identity must not be confused with executable or
  trace provenance.
- `inputs.manifest_hash` is lowercase SHA-256 of the compact serde JSON tuple
  `[compiler, phase, inputs]` with `inputs.manifest_hash` set to the empty string.
- Each `body_id` is `mir:` followed by lowercase SHA-256 of the compact JSON tuple
  `[inputs.manifest_hash, def_path]`. It is local to these compiler inputs, not a
  source `DefinitionId` or persistent cross-configuration identity.
- `span.status=exact` identifies a captured path and half-open, zero-based byte
  offsets into the original bytes. Rustc's original-position mapping restores
  CRLF offsets. A definition span covers rustc's definition/header span, not
  necessarily its whole body. Macro expansion, virtual, cross-file, generated,
  and external spans are explicitly unavailable.
- Blocks, locals, and source scopes use zero-based body-local indexes. Statement
  indexes are block-local. Scope records preserve parents and inlined item paths.
- A successor's kind distinguishes normal flow, switch values, otherwise,
  cleanup unwind, resume, coroutine drop, and imaginary edges. Switch values
  are unsigned MIR bit patterns serialized as decimal strings, including u128.
- `unwind` distinguishes continuation outside this body, unreachable unwind,
  process termination, and cleanup blocks. An unwind action without a block is
  not silently converted to an ordinary edge. Consumers deriving graph metrics
  must define their own virtual-exit convention explicitly.

### Local Effects

These are local accesses extracted with rustc's MIR visitor, not an advertised
reaching-definitions solver. `defs` contains whole-local assignments only.
Projected writes and writes through pointers do not define the pointer/base
local. `uses` is deliberately conservative and includes address-taking and
projection bases. Moves and storage-live/storage-dead events are separate.

Calls record their destination in `normal_return_defs`, never in unconditional
`defs`: the destination is not initialized on an unwind edge. A caller may return
an unknown value. Drop, unknown calls, indirect calls, borrows, pointer aliasing,
partial writes, inline assembly, intrinsics, retagging, and thread-local state
remain explicit unknown effects. A dataflow consumer must propagate these
effects conservatively instead of treating an empty whole-local `defs` list as
proof that no memory changed.

`FunctionDefinition` call targets identify compiler items. They do not assert a
fully monomorphized target or a selected generic-trait implementation. Pointer
and other indirect targets remain `Indirect`. External bodies, promoted
constants, static initializers, generated shims, and every monomorphized instance
are not enumerated. Coroutine lowering may remove original yield points; this
adapter does not promise a source-level async suspension graph.

## Qualification

On 2026-09-15, the pinned Linux toolchain passed 5 adapter unit tests and 11
compiler-process integration tests. The final measured run took 8.17 seconds of
warm recompilation and 0.47 seconds of test execution; this is not a cold-build
benchmark. Clippy was run for every adapter target with warnings denied.

The fixture is hand-written, not derived from a compiler dump. Expectations cover
a two-arm branch joining a return, copy-to-return-place def/use, reverse-order
destruction on normal and callback-unwind paths, termination on a second panic
during cleanup, and disappearance of cleanup paths under `panic=abort` while
normal drops remain. Further regressions cover pointer effects, indirect calls,
assertions, cfg selection, source mapping, macro uncertainty, deterministic
identity, output preservation, source limits, and environment scrubbing.

This is a small fixture-qualified adapter for the exact producer above, not
qualification of arbitrary Rust workspaces, all nightly features, all targets,
hostile builds, large corpora, or cross-toolchain interchange. The main product
must separately validate imported bundles against an immutable snapshot and
selected BuildContext before presenting them as attached compiler evidence.

Primary references: [external rustc drivers](https://rustc-dev-guide.rust-lang.org/rustc-driver/external-rustc-drivers.html),
[MIR overview](https://rustc-dev-guide.rust-lang.org/mir/index.html), and the pinned
[MIR syntax definitions](https://github.com/rust-lang/rust/blob/55e86c996809902e8bbad512cfb4d2c18be446d9/compiler/rustc_middle/src/mir/syntax.rs).
