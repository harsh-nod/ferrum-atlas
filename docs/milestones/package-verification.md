# Local Package Verification

## Preview 3

On 2026-09-15, [workflow 34956575490](https://github.com/harsh-nod/ferrum-atlas/actions/runs/34956575490)
built Linux x86_64 `0.1.0-preview.3` from source commit
`1e19f972c1aedf85ad76999c50784ab5f9688b49`. Both the package workflow and
[source CI](https://github.com/harsh-nod/ferrum-atlas/actions/runs/34956569046)
succeeded. No tag, GitHub release or draft was created. This is a time-limited
Actions review artifact, not a qualified release.

Archive: `ferrum-atlas-0.1.0-preview.3-x86_64-unknown-linux-gnu.tar.gz`

SHA-256: `f6fdfd51fc4c31abd24df0d843266be67541d0bd7a94b18ac866623141112e04`

Verification of this exact candidate:

- The cloud workflow ran 31 packaging regressions, validated the archive, checked
  its sidecar digest, extracted it and passed the live capture/HTTP/browser/edit/
  restart regression against the bundled CLI and viewer before uploading it.
  That cloud package check omitted the optional compiler adapter, which is not
  bundled. The separate source CI pinned-compiler job passed.
- Local validation of the downloaded archive passed for 501 payload files;
  `sha256sum --check` independently passed before fresh-directory extraction.
- The archive includes the source/CFG metrics guide and fixes the three broken
  offline references from preview 2 with explicitly online, commit-pinned links.
- The installed binary returned `atlas 0.1.0`, and installed `doctor` verified
  both demonstration snapshots with no errors.
- The local live regression passed using this installed binary, this bundled
  viewer and the separately built pinned compiler. Actual compiler import,
  reaching definitions, source/CFG measurements, exact source navigation,
  observations, state-review fixture records and restart all passed. Its graph
  canvas was nonblank and no browser page errors were reported.
- No development web server was started for installed-package checks. The
  regression stopped its own temporary application server and removed its
  temporary fixture and store. The separate development demo remains separate.

The workflow emitted GitHub's deprecation annotation for pinned actions declaring
Node 20; GitHub ran those actions on Node 24 successfully. This is not future
action-runtime qualification. Remaining license/distribution, scale, isolation
and human-study gates are unchanged. Later documentation-only corrections are
not covered by this archive hash.

## Preview 2

On 2026-09-15, [workflow 34955364397](https://github.com/harsh-nod/ferrum-atlas/actions/runs/34955364397)
built the Linux x86_64 `0.1.0-preview.2` candidate from source commit
`606b519f0812208bb311cc468e9cb539ea5e0f39`. The workflow succeeded. No tag,
GitHub release or draft was created. This remains a time-limited Actions review
artifact, not a qualified release.

Archive: `ferrum-atlas-0.1.0-preview.2-x86_64-unknown-linux-gnu.tar.gz`

SHA-256: `17ca8dbaf9b652996587c50227e552be06dd2ec46fabbb93bda1530eaff0ddde`

Verification of the downloaded artifact:

- Streaming structural validation passed for 500 payload files, followed by an
  independent `sha256sum --check`. Extraction used a fresh ignored directory.
- The packaged binary reported `atlas 0.1.0`; packaged `doctor` verified both
  captured demonstration snapshots with no errors.
- The complete live browser regression passed using the packaged binary and its
  packaged viewer. It captured a fresh fixture, imported actual pinned-compiler
  MIR, checked authorization and context mismatches, rendered nonblank graph
  pixels, selected source through graph/dataflow links, computed source and CFG
  metrics, retained and released bookmarks, recorded fixture state reviews,
  imported observations, edited the fixture and exercised server restart.
- The test also checked browser errors and stopped its temporary server. Its
  fixture declarations are automated test data, not human-study observations.
- Independent `readelf` inspection matched the manifest: the GNU loader,
  GLIBC 2.39, GCC 4.2.0, four declared libraries and no RPATH/RUNPATH.

The same regression can test a verified, extracted package from a source
checkout with browser test dependencies installed:

```sh
cd web
ATLAS_TEST_BINARY=/absolute/install/path/bin/atlas \
ATLAS_TEST_WEB_DIR=/absolute/install/path/share/ferrum-atlas/web \
ATLAS_RUSTC=/absolute/path/to/pinned/atlas-rustc \
ATLAS_WEB_TEST_PORT=4183 npx playwright test tests/live.spec.ts
```

Omitting `ATLAS_RUSTC` skips compiler integration, so that run must not be
reported as compiler-to-browser verification. The adapter is a separate trusted
build and is not included in this package. Later presentation, test-harness and
documentation fixes are not covered by this archive hash. Review found three
offline documentation references whose targets were not bundled in preview 2;
this is a known documentation defect in the immutable candidate.

## Preview 1

On 2026-09-15, [workflow 34952777116](https://github.com/harsh-nod/ferrum-atlas/actions/runs/34952777116)
built the Linux x86_64 `0.1.0-preview.1` candidate from source commit
`e0bf7a13e582cef735cdabec5d3e609747e19131`. The workflow succeeded. No tag,
GitHub release or draft was created; the downloadable Actions artifact is a
time-limited review candidate, not a qualified release.

Archive: `ferrum-atlas-0.1.0-preview.1-x86_64-unknown-linux-gnu.tar.gz`

SHA-256: `91150eb8dffa8b8c9f4c7859728881447111977c11da1848903f7e59fde46338`

Local verification of the downloaded artifact, not the development binary:

- Streaming archive validation passed for 500 payload files, followed by the
  independent `sha256sum --check` sidecar check.
- Extracted only after validation into a fresh, ignored installation directory.
- Packaged `atlas --version` returned `atlas 0.1.0`; the candidate suffix is in
  `RELEASE.json`, while the binary reports the Cargo base version.
- Packaged `doctor` verified both captured demonstration snapshots with no errors.
- The packaged CLI served the packaged browser on loopback. Playwright selected
  the captured function, rendered its graph, opened exact compiler MIR and ran
  reaching definitions to a fixed point. The canvas check counted 42,532
  nonblank pixels and the browser reported zero page errors.
- Stopped the temporary package-test server after verification. The separate
  development demo is not an installed release.

The demonstration uses public test-fixture source, exact compiler imports and
two snapshots. It establishes that this package runs these local workflows, not
scale, complete Rust semantics, operating-system isolation, accessibility
certification or results from human participants. Later commits are not covered
by this exact archive hash.

The [installation guide](../operations/install.md) records runtime requirements
and remaining distribution review gates. Archive checksums and package shape
validation do not establish authenticity or complete license compliance.

## Inspected Viewer States

These images were captured separately from the actual integrated local app,
using the same nonprivate fixture and imported compiler evidence:

![Compiler flow and reaching definitions](../images/compiler-dataflow-desktop.png)

![Mobile source flow and state-transition selection](../images/state-candidates-mobile.png)

The automated mobile state-review regression additionally exercises the result
table, acceptance/rejection controls and source navigation; the image is only a
visible selection checkpoint, not a substitute for that test.
