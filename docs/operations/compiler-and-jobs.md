# Compiler Evidence, Jobs, and Watching

## Pinned Compiler Evidence

The optional adapter is a separate workspace with one supported compiler:
`nightly-2026-04-03`, commit `55e86c996809902e8bbad512cfb4d2c18be446d9`,
Linux x86_64. See its [producer contract](../../adapters/rustc/README.md).
It runs only with explicit `--trusted-local` approval. It is not a hostile-code
sandbox and never runs automatically when opening a snapshot or browser view.

```sh
cargo build --locked -p ferrum-atlas
target/debug/atlas --store .atlas-compiler init --workspace adapters/rustc/fixtures
target/debug/atlas --store .atlas-compiler index --target x86_64-unknown-linux-gnu --no-default-features
cd adapters/rustc
cargo build --locked
target/debug/atlas-rustc --trusted-local --root fixtures --source control_flow.rs --crate-name atlas_compiler_fixture --output /tmp/atlas-compiler.json
cd ../..
target/debug/atlas --store .atlas-compiler import-compiler --snapshot SNAPSHOT_ID --bundle /tmp/atlas-compiler.json
target/debug/atlas --store .atlas-compiler serve
```

Replace `SNAPSHOT_ID` with the index command's returned identity. Import validates
source hashes, selected context, compiler identity, canonical invocation, body
references and spans before writing evidence. No compiler is run by import.
The import adds a retention pin. The viewer's Flow selector distinguishes source
flow points from named-phase compiler basic blocks. Missing mappings are unavailable,
not guessed. Compiler identities establish consistency, not producer authenticity.

Only a single library crate, explicit cfg/features and the installed sysroot are
supported. Full Cargo build units, dependency acquisition, user build scripts,
user proc-macro artifacts and arbitrary compiler versions are not orchestrated.

## Durable Local Jobs

`atlas serve --enable-jobs` opts into indexing the workspace already registered
with `atlas init`. The service still performs only read-only indexing. Without
this flag the jobs API reports an unsupported capability. The Health view can
submit or cancel a job; no view automatically starts analysis.

An optional `--scheduler-config FILE` accepts this versioned JSON:

```json
{"version":1,"timeout_seconds":120,"memory_mib":8192,"disk_quota_mib":4096,"max_queued":16}
```

One analysis runs at a time. Queued foreground work has priority over workspace
and background work. Identical active requests coalesce; a newer request for a
profile supersedes its older generation. Publication and supersession share a
fence, and catalog publication additionally requires compare-and-swap head identity.
Contention reports unavailable instead of waiting indefinitely.

The private `jobs/journal.json` retains at most 128 jobs and 32 events per job.
Restart marks interrupted jobs failed; it does not replay them silently. The
snapshot head remains authoritative if a crash occurred around publication.
Progress streams support authenticated replay with `Last-Event-ID`. Cancellation
kills and reaps the process group; child workers also receive parent-death signals.
Local memory limits are process address-space limits, not multi-tenant RSS quotas.

## Warm Watch Sessions

```sh
target/debug/atlas watch --interval-ms 1000 --timeout 120
target/debug/atlas watch --iterations 3 --level syntax
```

The CLI owns an exclusive indexing lease and a private bounded worker protocol.
Each polling cycle performs authoritative source reconciliation. The semantic
worker retains a rust-analyzer database for a compatible context/path set and
updates changed inputs. Context or path changes rebuild that database. Facts
are still fully extracted and published in immutable shards; this is not a
fine-grained persistent delta-shard implementation.

Exact normalized fact identities avoid unchanged publication. Worker sessions
are reset after 32 successful captures, and failures preserve the previous head.
Ctrl-C cancels waiting, terminates and reaps workers, and prints the final report.
`--iterations` is useful for repeatable tests and bounded runs.

## Snapshot Retention

CLI pins, imported compiler evidence, observations and named heads retain their
snapshots. Browser reading-trail bookmarks use authenticated, context-scoped
server pins; local browser storage alone is not a retention guarantee. Removing
one bookmark releases only its scoped pin, not other bookmarks or named pins.
Pin admission is bounded to 4,096 names per local store.

Review [retention operations](retention.md) before deletion and
[portable snapshots](portable-snapshots.md) before transfer. A compiler bundle
is not an executable artifact. Portable exports currently include source facts
and observation bundles, not optional compiler imports; re-import the verified
compiler bundle into the restored snapshot separately.
