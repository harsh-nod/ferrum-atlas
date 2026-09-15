# Local Operations

## Build and Open

```sh
cargo build --locked -p ferrum-atlas
cd web
npm ci
npm run build
cd ..
target/debug/atlas init --workspace /path/to/rust/project --trust read-only
target/debug/atlas index --level semantic
target/debug/atlas serve --web-dir web/dist
```

Open the URL printed by `serve`. The token is a fragment, never part of an HTTP URL or shared bookmark. The viewer removes it from browser history and retains it in session storage. API clients send `Authorization: Bearer <token>`. The service accepts loopback only and validates Host and Origin headers. Remote or multi-user hosting is not supported.

`--store PATH` selects a separate local catalog. Keep its files on a local filesystem; do not share a SQLite WAL catalog through NFS. Source snapshots contain private source, so keep stores outside public Git and use access-controlled storage. The public source tree ignores `.atlas/`.

## Configurations and Updates

```sh
atlas index --profile device --target your-target --features feature-a,feature-b --cfg custom_flag
atlas snapshots
atlas query search --text dispatch --profile device
atlas query callees --symbol definition:... --snapshot snapshot:...
atlas diff --before snapshot:... --after snapshot:...
```

Explicit BuildContext JSON is accepted through `index --context PATH`. Contexts identify captured inputs and limitations; no target values are guessed from the analysis host. Re-run `index` after editing or use `watch` for authoritative polling with a warm semantic database. Published snapshots stay immutable and old browser sessions stay pinned. Unchanged normalized facts deduplicate. Watch refreshes all extracted facts and rebuilds the database on incompatible inputs; persistent fine-grained delta shards are not implemented. See [compiler and job operations](compiler-and-jobs.md).

Source capture does not execute Cargo, rustc wrappers, build scripts or proc macros. Missing generated files, dependencies, sysroot metadata, or unknown cfg are reported. Broken source remains partially browsable. Compiler MIR requires a separately generated, exact-toolchain bundle imported against the captured snapshot. Executable sandboxed analysis remains unavailable; source flow never substitutes for compiler CFG.

## Limits and Failures

Indexing runs in a subprocess with a cleared environment, address-space/CPU/output-file limits, and a parent wall-clock timeout. Defaults are 120 seconds, 8192 MiB address space, and 256 MiB output. The Linux worker sets a parent-death signal, checks the parent identity, and applies resource limits before initializing the analyzer. Error paths kill and reap unfinished workers. The worker only reads source; this is resource containment, not a hostile executable-code sandbox. Failed workers never publish a new head. Adjust supported budgets through CLI flags.

The default store quota is 4096 MiB. Ingestion pauses at 80% existing usage; estimated publication growth is rejected at 90%. This is conservative admission accounting, not an OS disk quota. Concurrent publishers and filesystem overhead can still consume additional space. Preserve recovery headroom.

`atlas doctor` checks catalog, shards, referenced objects and content hashes. `atlas gc --dry-run --output PLAN` records an exact retention plan. `atlas gc --execute --plan PLAN` revalidates it before deletion. Live reader leases, heads, explicit pins, recent history, grace periods and observations protect retained data. Only registered owned objects are removable; source workspaces are never cleanup targets. Read [retention operations](retention.md) before changing policy.

## Backup and Restore

Stop writers, then stop the local viewer and copy the entire store, including the catalog and its WAL/SHM sidecars if present. Do not copy only the database while a writer is active. Restore into a fresh directory, run `atlas --store RESTORED doctor`, then answer the same pinned symbol/source query. The catalog and referenced immutable objects are one backup unit. No in-place schema migration is supported; retain the original store when rebuilding with a new schema.

`atlas export` defaults to a limited symbol manifest. `atlas export --format portable --snapshot ID --output DIRECTORY` includes complete static facts, captured source and observation bundles, with checksums. `atlas import --input DIRECTORY` validates all objects before publication. See [portable snapshots](portable-snapshots.md) for budgets, legacy shard restrictions and compiler-import exclusions. A consistent whole-store backup additionally retains jobs, compiler imports, pins and local configuration.

## Imported Observations

```sh
atlas import-evidence --snapshot snapshot:... --bundle observations.json --artifact path/to/executable
```

The artifact is read and hashed, never executed. The JSON must follow `ObservationBundle` in the generated API contract and match the selected source/context and actual artifact SHA-256. Any definition mapping must belong to that snapshot. Import does not independently prove who produced a trace or whether a claimed test actually ran: the producer and bundle remain user-supplied evidence. Static call facts do not change when observations are imported.

Bundles are capped at 2 MiB, 1,000 test records, 64 streams and 10,000 events. Each snapshot holds up to 50 deduplicated bundles; a per-snapshot filesystem lock serializes admission and publication. Window queries return at most 200 records. Unknown mappings may be omitted, while an explicit wrong mapping is rejected. A timeout must record its timeout cap. Device/thread identifiers, clock domains, units, sequence and loss counters survive import; independent clocks are never silently merged.

The Evidence view displays these separate observations. Back up its `observations/` directory with the rest of the store. `doctor` validates static snapshot storage; observation checksums are validated when observations are read, not by `doctor`.

## Qualification

`atlas benchmark --samples 30 --output report.json` records every latency sample and result status, the exact snapshot, configuration, coverage and basic hardware information. Explicit cold verification time is separate; the OS page cache remains uncontrolled. A deadline-partial result is not a completed empty search. This smoke measurement does not establish L/S/O targets, cold-index performance, human comprehension gains or p99 latency. [Qualification tooling](../../benchmarks/README.md) retains broader real/synthetic measurements and an unfilled human-study protocol.
