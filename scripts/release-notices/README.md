# Supplemental Upstream Notices

These are unchanged notice texts downloaded from the immutable source URLs in
`../release-notices.json` on 2026-09-15. SHA-256 values were checked against the
raw upstream responses. No downloaded code was executed. The packager never
downloads notices at release time: it verifies the checked-in bytes, records
their exact URL/digest/mapping basis in the dependency inventory, and uses them
only for specifically enumerated package versions that lack distributed text.

The rust-analyzer, perf-event and ts-rs revisions come from published Cargo VCS
metadata. The `ra-ap-rustc` revision is named in those packages' descriptions.
Rolldown's v1.2.8 tag was resolved to the recorded immutable source commit.
`perf-event-open-sys` now shares the perf-event source repository; its older
repository metadata does not expose that revision at the old URL.

The legacy la-arena and line-index crates lack exact VCS provenance. They declare
the repository's MIT/Apache alternatives; their supplemental mapping explicitly
records that repository-root notices at the selected rust-analyzer revision are
being used, not an assertion of exact legacy source identity. The analyzer's VCS
metadata also records a dirty publishing tree. These limitations remain visible.

This closes missing-text acquisition, not independent licensing approval.
Maintainers must still review source-availability obligations (including the
EPL/GPL alternative declared by elkjs), native/toolchain/bundled components,
copyright attribution, modifications and distribution terms before publishing.
No source offer, authenticity guarantee, legal determination or authorization
to publish follows from passing the packaging validator. The workflow creates
only draft prereleases and cannot promote them to public releases.
