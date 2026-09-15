# Dependency Inventory and Audit

Checked on 2026-09-15. This records the resolved dependency metadata and scans
performed for the local implementation, not a security or license-compliance
certification. No dependency versions were changed during this check.

## Inputs and Method

- Rust: `cargo metadata --locked --format-version 1`, using the root `Cargo.lock`.
  Direct dependencies below are the union of external dependencies of all eleven
  workspace packages, including development dependencies. Metadata contained 264
  external resolved packages; every package declared a license.
- Separate pinned compiler adapter: `adapters/rustc/Cargo.lock`, 40 external
  resolved packages, all with declared licenses. Its direct dependencies are
  `anyhow`, `rustix`, `serde`, `serde_json`, `sha2`, `tempfile`, and `ts-rs`, at
  the same versions listed below. Compiler/sysroot components are not Cargo
  lockfile dependencies and are outside the advisory scan.
- Viewer: `web/package.json` and the version 3 `web/package-lock.json`, parsed as
  JSON. The lockfile contained 89 package records, including development and
  platform-optional dependencies; every record declared a license.
- Rust license strings below are the packages' declared Cargo metadata. Viewer
  license strings are the locked package metadata. They are not an independent
  review of every distributed file or bundled component.
- Checks were repeated in the integrated main worktree. The lockfile hashes
  identify the exact inputs regardless of worktree location.

| Input | SHA-256 |
| --- | --- |
| `Cargo.lock` | `45bbad062da93ba50cc96386c8b995f0faa50e946fcfe6d37b374ff6ae7077d7` |
| `adapters/rustc/Cargo.lock` | `370c0d99c47df66ec4c38cf2ad095457b03b8335c8223253ff45e201aacd5533` |
| `web/package-lock.json` | `9f33697eaada45a9c4e9b65ba01a3ab1e714d35eafe2ce8831ae3fd12b4a9b66` |

## Direct Rust Dependencies

Workspace-owned crates use the project's `MIT OR Apache-2.0` license and are not
listed as third-party dependencies. `tempfile` is both a normal and a development
dependency; the remaining entries occur as normal dependencies of workspace
packages, including development tooling such as `xtask`.

| Package | Resolved Version | Declared License |
| --- | --- | --- |
| `anyhow` | `1.0.104` | MIT OR Apache-2.0 |
| `axum` | `0.8.4` | MIT |
| `clap` | `4.6.6` | MIT OR Apache-2.0 |
| `futures-util` | `0.3.34` | MIT OR Apache-2.0 |
| `hmac` | `0.12.1` | MIT OR Apache-2.0 |
| `petgraph` | `0.8.3` | MIT OR Apache-2.0 |
| `ra_ap_base_db` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_cfg` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_hir` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_ide_db` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_intern` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_parser` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_span` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_syntax` | `0.0.349` | MIT OR Apache-2.0 |
| `ra_ap_vfs` | `0.0.349` | MIT OR Apache-2.0 |
| `rusqlite` | `0.40.2` | MIT |
| `rustix` | `1.1.4` | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| `serde` | `1.0.229` | MIT OR Apache-2.0 |
| `serde_json` | `1.0.151` | MIT OR Apache-2.0 |
| `sha2` | `0.10.9` | MIT OR Apache-2.0 |
| `tempfile` | `3.27.0` | MIT OR Apache-2.0 |
| `tokio` | `1.47.1` | MIT |
| `toml` | `1.1.5+spec-1.1.0` | MIT OR Apache-2.0 |
| `tower` | `0.5.2` | MIT |
| `tower-http` | `0.6.11` | MIT |
| `triomphe` | `0.1.15` | MIT OR Apache-2.0 |
| `ts-rs` | `11.1.0` | MIT |

## Direct Viewer Dependencies

| Package | Resolved Version | Use | Declared License |
| --- | --- | --- | --- |
| `@codemirror/lang-rust` | `6.0.2` | Runtime | MIT |
| `@codemirror/language` | `6.12.4` | Runtime | MIT |
| `@codemirror/state` | `6.7.4` | Runtime | MIT |
| `@codemirror/view` | `6.43.11` | Runtime | MIT |
| `cytoscape` | `3.34.3` | Runtime | MIT |
| `elkjs` | `0.12.0` | Runtime | EPL-2.0 OR GPL-3.0-or-later |
| `lucide-react` | `1.46.0` | Runtime | ISC |
| `react` | `19.3.0` | Runtime | MIT |
| `react-dom` | `19.3.0` | Runtime | MIT |
| `@playwright/test` | `1.63.0` | Development | Apache-2.0 |
| `@types/cytoscape` | `3.31.0` | Development | MIT |
| `@types/node` | `26.5.1` | Development | MIT |
| `@types/react` | `19.3.0` | Development | MIT |
| `@types/react-dom` | `19.3.0` | Development | MIT |
| `@vitejs/plugin-react` | `6.1.1` | Development | MIT |
| `typescript` | `7.0.2` | Development | Apache-2.0 |
| `vite` | `8.3.0` | Development | MIT |

### Distribution Review

The project's own MIT/Apache license does not replace dependency licenses. In
particular, `elkjs` declares EPL-2.0 OR GPL-3.0-or-later, and its installed
`LICENSE.md` contains the Eclipse Public License 2.0. The transitive development
dependency `lightningcss` 1.33.0 and its platform packages declare MPL-2.0.

Before publishing compiled binaries or viewer bundles, review the actual
distribution's licenses, notices, bundled native components, and applicable
source-availability requirements. This metadata inventory is not a complete
third-party notice bundle or a determination of license compatibility.

The packaging workflow now collects actual dependency notice files and pinned
supplements. Its inventory is distinct from this declared-license summary.
[Installation and distribution gates](operations/install.md) explicitly retain
the unpassed ELK preferred-source/source-availability and Rust sysroot-linked
notice reviews. The [downloaded package check](milestones/package-verification.md)
establishes runtime/shape checks, not licensing approval.

## Advisory Scan Results

| Command | Tool and Scope | Observed Result |
| --- | --- | --- |
| `cargo audit --json` | Installed `cargo-audit` 0.22.1; 275 root lockfile packages; no target filters or ignored advisories | Exit 0; 0 matched vulnerabilities; no informational warnings |
| `cargo audit --json --file adapters/rustc/Cargo.lock` | Same audit tool; 41 compiler adapter lockfile packages | Exit 0; 0 matched vulnerabilities; no informational warnings |
| `npm audit --omit=dev --json` | npm 11.16.0 on Node 22.22.1; viewer production dependency graph | Exit 0; 0 vulnerabilities reported at every severity |
| `npm audit --json` | Same npm and Node versions; viewer graph including development dependencies | Exit 0; 0 vulnerabilities reported at every severity |

The RustSec database contained 1,246 advisories at commit
[`e2e640471715167f73e22eaf761f2e547adafeec`](https://github.com/RustSec/advisory-db/commit/e2e640471715167f73e22eaf761f2e547adafeec),
last updated `2026-09-14T18:06:06+02:00`. The npm audit responses did not expose an
advisory-database revision. Both npm responses reported 89 total dependencies;
their production/development/optional classification counts overlap and are not
added together here.

These results mean that those databases returned no matching advisories at check
time. They do not demonstrate absence of vulnerabilities, malicious packages,
unknown advisories, or application-level defects. No dedicated source audit,
reachability analysis, provenance/signature verification, complete SBOM, native
library advisory scan, or license-policy engine was run. Browser binaries, the
Rust and Node toolchains, operating-system packages, and action-internal
dependencies are outside these lockfile scans. Re-run scans after dependency
changes and before releases.

## CI Action Pins

Each action reference in `.github/workflows/ci.yml` was queried using GitHub's
repository commits API. The API returned the exact requested 40-character SHA
from the expected repository for all four entries:

| Action | Verified Commit |
| --- | --- |
| `actions/checkout` | [`11d5960a326750d5838078e36cf38b85af677262`](https://github.com/actions/checkout/commit/11d5960a326750d5838078e36cf38b85af677262) |
| `actions/setup-node` | [`49933ea5288caeca8642d1e84afbd3f7d6820020`](https://github.com/actions/setup-node/commit/49933ea5288caeca8642d1e84afbd3f7d6820020) |
| `actions/cache` | [`0057852bfaa89a56745cba8c7296529d2fc39830`](https://github.com/actions/cache/commit/0057852bfaa89a56745cba8c7296529d2fc39830) |
| `actions/upload-artifact` | [`ea165f8d65b6e75b540449e92b4886f43607fa02`](https://github.com/actions/upload-artifact/commit/ea165f8d65b6e75b540449e92b4886f43607fa02) |

Existence and exact pinning were verified; this was not a review of action source,
release recency, signing, or supply-chain trust.

## Repeat the Checks

From the repository root:

```sh
cargo metadata --locked --format-version 1
cargo audit --json
cargo audit --json --file adapters/rustc/Cargo.lock
npm --prefix web audit --omit=dev --json
npm --prefix web audit --json
sha256sum Cargo.lock adapters/rustc/Cargo.lock web/package-lock.json
```

`cargo-audit` must already be installed or separately provisioned. Metadata may
download uncached packages, but these checks do not execute an analyzed project's
Cargo commands. Ferrum Atlas's read-only indexing trust boundary is unchanged.
