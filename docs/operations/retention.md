# Snapshot Retention

The store provides `pin`, `unpin`, `pins`, `gc_plan`, and `gc_execute` APIs. A
`RetentionPolicy` retains every named head and pin, the configured number of
recent generations per repository and build context, snapshots younger than
the grace period, explicitly supplied additional roots, and snapshots with an
observation scope directory. The default is three recent generations and a
24-hour grace period. Pin names are unique; setting an existing name atomically
moves that pin. Removing a pin does not remove the snapshot immediately.

`gc_plan` returns an exact, serializable candidate plan: retained snapshots and
their reasons, snapshots to remove, owned objects, category byte totals, and a
fingerprint. `gc_execute` recomputes the plan under an exclusive lease and
rejects changes caused by publication, new pins, observation roots, grace-period
expiry, or object replacement. Repeating an already completed plan returns its
recorded report without deleting anything else. `gc_dry_run` never deletes data.

## Reader Protection

Every `SnapshotReader` owns a shared operating-system file lock until dropped.
Publishers hold the same shared lock from before validation through catalog
commit. Collection requires an exclusive, nonblocking lock, so it reports busy
while any reader, publisher, or collector is active. Process termination releases
locks automatically, without guessing lease expiry or reclaiming a live slow
reader. This intentionally prevents collection of unrelated snapshots too.

Observation importers must keep a snapshot reader alive through the observation
write, or pin the snapshot before import. Arbitrary external observation writers
are not protected. Older binaries that do not acquire store leases must not run
concurrently against a store with deletion enabled.

## Ownership and Recovery

Only registered immutable source and shard objects can be deleted. Registration
records device, inode, size, and creation time; replaced files cause collection
to fail closed. Existing published references are registered on first open with
the new implementation. Unknown files, incomplete staging files, observations,
configuration, credentials, scheduler state, and other directory contents are
never deleted. A crash between object creation and registration can therefore
leave an unregistered object for manual inspection rather than unsafe cleanup.

Directory-relative operations reject symlink object directories. Object links
are inspected without following them, and deletion is confined to the opened
source/shard directory. No recursive removal is used. The store assumes its
owner does not maliciously replace regular files during the final stat/unlink
interval; filesystem ownership is not a sandbox against the same OS user.

Collection first commits snapshot removal and a recovery record in SQLite, then
unlinks unreferenced objects, syncing the containing directory before removing
ownership records. If interrupted, retained snapshots remain readable. Create
and execute a new plan to finish orphan cleanup; already absent objects are
accounted for with zero bytes. Republished/shared objects are recomputed as live
and cannot be removed by recovery. No unlinks run automatically at startup.

Planning currently rejects catalogs above 100,000 snapshots, 128 MiB of snapshot
metadata, or 1,000,000 registered objects. It validates referenced shard checksums
and materializes the bounded plan. These are local operational bounds, not
billion-symbol scale or latency qualification.
