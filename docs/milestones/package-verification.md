# Local Package Verification

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
