# Install a Local Prerelease

This packaging path produces **unqualified prereleases**, not a qualified L/S
release or a hosted/distributed service. No release or tag is published by the
packaging script. A maintainer must review dependency notices, test evidence,
limitations and draft assets before making a prerelease public.

## Supported Machine

- Linux x86_64 with glibc 2.39 or newer and mounted `/proc`.
- A local filesystem for the store, not a shared NFS SQLite catalog.
- A current browser. Rust, Cargo, Node and npm are not needed to browse or import
  an exported index, or to perform read-only source indexing with the binary.
- The default indexing worker reserves an 8192 MiB address-space budget. Disk
  admission defaults to 4096 MiB per store and preserves recovery headroom.
  These are controls, not measured hardware or performance-tier guarantees.

The archive is built on Ubuntu 24.04. Other architectures, musl, Windows, macOS,
remote serving and multi-user hosting are not included. The compiler MIR driver
and its separately pinned nightly toolchain are not bundled.

## Acquire and Verify

Use a reviewed version from the project's GitHub Releases page. Prereleases do
not use GitHub's `latest` download alias. The example version below is a naming
example, not a claim that an asset has been published.

```sh
VERSION=0.1.0-alpha.1
ARCHIVE="ferrum-atlas-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
BASE="https://github.com/harsh-nod/ferrum-atlas/releases/download/v${VERSION}"
curl --fail --location --proto '=https' --tlsv1.2 --output "$ARCHIVE" "$BASE/$ARCHIVE"
curl --fail --location --proto '=https' --tlsv1.2 --output "$ARCHIVE.sha256" "$BASE/$ARCHIVE.sha256"
sha256sum --check "$ARCHIVE.sha256"
```

Check the repository, tag, review notes and exact source commit as well as the
digest. A checksum proves byte integrity relative to the downloaded checksum,
not authenticity, absence of vulnerabilities, or qualification. These artifacts
are not independently signed or attested. Do not run an untrusted binary.

Maintainers and users with this source checkout can additionally validate the
archive without extracting or executing it:

```sh
python3 scripts/release.py validate "$ARCHIVE"
```

The validator rejects unexpected files, duplicate/traversing paths, links,
special files, missing components, mismatched hashes, incompatible metadata,
incorrect modes and oversized archives. It does not inspect machine-code
behavior or establish complete license compliance.

## Install and Open

Install each version side by side; no administrator privileges are necessary.
Only extract an archive after verification. Keep the application directory and
private stores separate.

```sh
INSTALL_ROOT="$HOME/.local/opt/ferrum-atlas"
RELEASE="$INSTALL_ROOT/ferrum-atlas-${VERSION}-x86_64-unknown-linux-gnu"
mkdir -p "$INSTALL_ROOT"
test ! -e "$RELEASE"
tar --extract --gzip --file "$ARCHIVE" --directory "$INSTALL_ROOT" --no-same-owner --no-same-permissions
ATLAS="$RELEASE/bin/atlas"
STORE="$HOME/.local/share/ferrum-atlas/example"
"$ATLAS" --version
"$ATLAS" --store "$STORE" init --workspace /absolute/path/to/project --trust read-only
"$ATLAS" --store "$STORE" index --level semantic --memory-mib 8192 --disk-quota-mib 4096
"$ATLAS" --store "$STORE" doctor
"$ATLAS" --store "$STORE" serve --web-dir "$RELEASE/share/ferrum-atlas/web"
```

Open the loopback URL printed by `serve`. Its initial token is in the URL
fragment; the viewer removes the fragment and keeps credentials in session
storage. Do not log, publish or share that token. Use the same release's browser
assets and binary. `serve` must not be exposed through a public reverse proxy.

To open a reviewed portable export instead of indexing source:

```sh
"$ATLAS" --store "$STORE" import --input /absolute/path/to/portable-export --profile default
"$ATLAS" --store "$STORE" doctor
"$ATLAS" --store "$STORE" snapshots
"$ATLAS" --store "$STORE" serve --web-dir "$RELEASE/share/ferrum-atlas/web"
```

No analyzed-project build command runs during read-only capture. Missing macros,
dependencies, generated inputs or target cfg remain explicit unknowns. Partial
analysis is not evidence that a call or behavior does not exist. Compiler MIR
requires the separate trusted adapter workflow, an exact supported compiler and
an artifact-matched import; source flow is not a substitute.

## Upgrade and Restore

`share/ferrum-atlas/schema-compatibility.json` records the model, store,
portable-export and compiler-bundle schemas supported by this release.
`RELEASE.json` records the source commit, pinned input hashes, target and file
digests. Unknown store/schema versions are rejected; no in-place schema migration
or irreversible upgrade of the only copy is supported.

Before replacing a version, stop `watch`, indexing jobs and the viewer. Back up
the entire store, including immutable objects, observations, compiler imports,
catalog and any WAL/SHM sidecars. Do not copy only a live SQLite database. Keep
the prior application and backup unchanged; test a restored copy with the new
binary. Run `doctor`, answer the same pinned source/call queries, and verify
observation/compiler evidence links before accepting the restoration.

Portable exports preserve selected source and facts, so review them before
sharing. They do not replace a whole-store backup: configuration, jobs, pins and
compiler-import retention differ. Old shards missing export metadata may require
a fresh index. Rebuilding into a new store is the fallback for incompatible
schemas, not overwriting qualified evidence. See [portable snapshots](portable-snapshots.md).

Disk admission pauses around 80% existing usage or 90% estimated post-publication
usage. It is not a filesystem quota. Increase the explicit budget only after
checking free space, or inspect a retention plan before executing it. Worker
timeouts or crashes retain the last valid head. Investigate partial coverage and
recorded failure reasons before retrying; do not delete the store to clear a
transient worker error. See [local operations](local.md),
[compiler and jobs](compiler-and-jobs.md), and [retention](retention.md).
Opt-in state-machine annotations and their limitations are documented in
[state machines](state-machines.md).

## Remove Safely

Stop processes using this installation. Remove only the exact version directory
under your chosen application installation root. Stores and source workspaces
are separate and are not removed with the binary. For data cleanup, review
`gc --dry-run --output PLAN`, then use `gc --execute --plan PLAN` against the
intended store. Unknown files, credentials and source workspaces are not GC
targets. Keep backups and pinned evidence until their retention decision is
explicit. Never run recursive cleanup against a source workspace or an inferred
store path.

## Rebuild the Package

Maintainers need Python 3.11+, the repository's `rust-toolchain.toml` pin
(currently 1.97.1), Node 22.22.1, and npm 11.16.0. Build only this trusted project,
from a reviewed clean commit on Ubuntu 24.04. `npm ci` and Cargo build scripts
execute dependency/build code; they are not part of read-only analyzed-project
indexing. Lockfiles and notices require review when dependencies change.

```sh
VERSION=0.1.0-alpha.1
test -z "$(git status --porcelain)"
rustup show
node --version
npm --version
npm --prefix web ci
cargo test --locked -p atlas-model --test release_compatibility
cargo run --locked -p xtask -- check-types
cargo build --locked --release --target x86_64-unknown-linux-gnu -p ferrum-atlas -j2
npm --prefix web run build
mkdir -p release-inputs release-output
cargo metadata --locked --format-version 1 --filter-platform x86_64-unknown-linux-gnu > release-inputs/cargo-metadata.json
python3 -m unittest discover -s scripts/tests -p 'test_release.py'
python3 scripts/release.py package --repo . --version "$VERSION" \
  --commit "$(git rev-parse HEAD)" --epoch "$(git show -s --format=%ct HEAD)" \
  --binary target/x86_64-unknown-linux-gnu/release/atlas --web-dist web/dist \
  --cargo-metadata release-inputs/cargo-metadata.json --npm-root web --output release-output
python3 scripts/release.py validate "release-output/ferrum-atlas-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
```

Only explicit binaries, `web/dist/index.html`, allowlisted `web/dist/assets`
files, operation documents, compatibility metadata, project licenses and
dependency notices enter the archive. Source maps, stores, tokens, source
exports, arbitrary public files and symlinks are rejected or never traversed.
Packages that omit license text use only the explicitly versioned supplements in
`scripts/release-notices.json`, verified against checked-in immutable upstream
notice bytes. Their exact source URLs, hashes and provenance limitations are
carried into the inventory. Missing unreviewed notices still block packaging;
supplementation is not approval of source-offer or distribution obligations.
Packaging never runs the binary. Repeating packaging with identical payloads,
commit, version and epoch yields identical tar/gzip bytes; independently
bit-reproducible Rust/linker builds have not been demonstrated.

The release workflow runs only for explicit dispatch or version tags. It uploads
versioned workflow artifacts. Creating a GitHub draft additionally requires an
existing matching tag; `--verify-tag` forbids implicit tag creation. Drafts are
always marked prerelease and not latest. The workflow never promotes a draft or
makes a qualified, hosted, or distributed release claim. Review the packaged
dependency inventory and notice texts, including source-availability obligations
and bundled/native/toolchain components, before public distribution.
