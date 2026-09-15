# Portable Snapshots

`Store::export_directory(snapshot, destination)` creates a new portable directory
containing a versioned `manifest.json`, `snapshot.json`, `facts.json`, and zero
or more artifact-matched observation JSON files. The manifest records exact file
sizes and SHA-256 checksums. Export writes and syncs a private staging directory,
then atomically renames it with no-replace semantics. An existing destination,
including an empty directory or symlink, is never overwritten.

Facts contain exact UTF-8 source bytes (including CRLF), manifests, capture
warnings, build context, definitions, relations, evidence, source flows,
diagnostics, coverage, and producer identity. Export preserves the snapshot's
identity and original publication metadata. It does not include store tokens,
configuration, scheduler state, absolute workspace roots, or executable
artifacts. Repository source and manifest contents are intentionally included
without redaction; review them before sharing an export.

Portable version 1 does not include separately imported compiler bundles or
their derived MIR CFG/dataflow results. Preserve those bundles separately and
reimport them against the restored snapshot using `atlas import-compiler`.
The importer revalidates their exact source and context identities; restoring
a portable snapshot alone must not be interpreted as restoring compiler evidence.

## Import Contract

`Store::import_directory(source, head, expected, validate_observations)` reads only
the manifest's supported fixed filenames. It rejects duplicate entries, path
traversal, symlinks, nonregular files, unknown versions, checksum mismatches,
invalid source/reference spans, and fact/snapshot identity mismatches before
publication. The ordinary compare-and-swap head contract and immutable object
collision checks apply. A completed duplicate import is idempotent.

The required validation callback receives the snapshot, complete definition-ID
set, and observation bundles before publication. The application must call its
existing observation validator there. The store additionally checks observation
schema and artifact source/context identity. Imported executable digest strings
are preserved claims from the archived observations; import does not claim to
have rehashed a binary that is not included in the export.

The result contains `snapshot`, `observations`, and a live `reader` lease. Keep
that reader alive while restoring observations through the evidence subsystem.
If evidence restoration fails after publication, report the partial restoration
and retry: immutable snapshot publication has already succeeded. Observation
restore must remain idempotent. Store import alone does not silently write
observation records or bypass the evidence subsystem's validation.

## Compatibility and Bounds

New shards include an additive fact-envelope metadata record for information
that older shards did not preserve, such as capture warnings and diagnostics.
Old shards remain readable, but complete export fails with an actionable error.
Reindex into a fresh store to create a complete export. Republishing identical
facts into an existing store does not mutate or upgrade its immutable shard.

Portable version 1 is bounded to 256 MiB serialized facts, 2 MiB snapshot metadata,
and 50 observations of at most 2 MiB each. Input reads and output serialization
enforce these bounds. Export also bounds fact table rows to one million each and
source files to 100,000, and waits at most five seconds for an observation import
lock. These local bounds do not imply billion-symbol export qualification.
