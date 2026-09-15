# Qualification: Measured Next Gates

Status on 2026-09-15: reproducible smoke tooling, 300 initial query/profile samples,
and 240 committed-build cache follow-up samples recorded. **No F/L/S/O/X tier
passes, speedup multiplier, semantic accuracy, or human
comprehension improvement are established.** Partial responses are not successful
empty searches. A successful indexing process is not a useful-query pass.

## Corpus and Provenance

The reviewed [public corpus manifest](../../benchmarks/manifests/public-corpus.json)
pins [rust-lang/regex 1.11.1](https://github.com/rust-lang/regex/tree/9870c06e6c772daaad7ab612faab29130753e41c)
to `9870c06e6c772daaad7ab612faab29130753e41c`. Root/member manifests declare
MIT OR Apache-2.0. The [MIT](https://github.com/rust-lang/regex/blob/9870c06e6c772daaad7ab612faab29130753e41c/LICENSE-MIT),
[Apache-2.0](https://github.com/rust-lang/regex/blob/9870c06e6c772daaad7ab612faab29130753e41c/LICENSE-APACHE),
and separate [Unicode data notice](https://github.com/rust-lang/regex/blob/9870c06e6c772daaad7ab612faab29130753e41c/regex-syntax/src/unicode_tables/LICENSE-UNICODE)
were inspected and checksummed. No source is redistributed in this project, and
no external Cargo/build scripts/proc macros/tests or repository tools were run.

The checkout contains 124,986 non-generated physical Rust lines plus 33,933 known
generated Unicode-table lines in 222 files. There is no replicated LOC, one
repository/configuration/revision, one query client, and no dependency source
outside the checkout. Crate-attributed LOC and macro expansion ratio remain
unmeasured; the runner labels these omissions instead of inventing values.

The synthetic workload directly publishes 20,000 definitions, 80,000 relations,
one evidence record, and one source file: **100,002 actual indexed records**.
Its high-degree hub/ring includes 825 deterministic unknown targets. This is not
a semantic extraction benchmark, and placeholder source bodies are not evidence
for the generated graph topology.

Both initial executable SHA-256 values are retained. Their exact build-source
revisions are unverified because a shared development target was in use. The
regex executable was not frozen before that preliminary run; the synthetic
executable was copied and verified before launch. Future runner invocations
freeze executables and distinguish harness commit from build-source provenance.
A clean, pinned build repeat is still required before qualification claims.

## Actual Results

Raw reports: [regex semantic](../../benchmarks/reports/regex-semantic-20260915.json)
and [synthetic 100k](../../benchmarks/reports/synthetic-100k-20260915.json).
Each listed workload has 30 samples; p50/p95 use nearest rank and include the
first sample and all failures. p99 is deliberately absent.

| Measure | Regex, CLI end-to-end | Synthetic, in-process |
| --- | ---: | ---: |
| Actual records | 54,558 | 100,002 |
| Definitions / relations | 8,361 / 20,668 | 20,000 / 80,000 |
| Unknown relations | 20,668 (100%) | 825 (1.03%) |
| Index/publication wall time | 130.97 s index | 15.86 s publication |
| Entire measured process CPU | 80.29 s user + 27.87 s system, index only | 136.84 s user + 2.90 s system, publication and queries |
| Sampled process-tree peak RSS | 179.44 MiB | 138.70 MiB |
| Kernel max single-process RSS | 178.19 MiB | 137.45 MiB |
| Persisted store bytes | 79,147,308 | 58,739,808 |
| Search p50 / p95 | 320.46 / 417.40 ms | 253.47 / 256.32 ms |
| One-hop p50 / p95 | 325.78 / 343.78 ms | 254.44 / 258.88 ms |

**All 60 regex query samples and all 90 ordinary synthetic search/graph samples
returned empty deadline-partial results.** All 30 synthetic source-window samples
failed with `budget_exhausted`. These latencies measure deadline handling, not
completed useful content. Regex's unknowns are 17,618 `cfg_unknown` and 3,050
`macro_unavailable`; indexing it does not demonstrate resolved-call precision.

Additional synthetic profiles:

| Workload | p50 | p95 | Outcome |
| --- | ---: | ---: | --- |
| Two-hop graph | 254.58 ms | 257.05 ms | 30/30 empty deadline-partial |
| Source window | 252.51 ms | 256.94 ms | 30/30 budget errors |
| Pre-cancelled graph | 1.83 ms | 2.70 ms | 30/30 explicitly cancelled/partial |
| Reader open and checksum | 2,787.78 ms | 3,398.53 ms | 30/30 completed |
| Already-open indexed search | 1.62 ms | 2.09 ms | 30/30 returned 50 records |
| Already-open adjacency | 7.73 ms | 10.50 ms | 30/30 returned 501 bounded rows |

The missing-shard injection returned `unavailable_shard`, and restoring the
owned fixture shard preserved full integrity. No memory, disk, or outer wall
budget was exceeded. Scratch data at the end of measurement was 154,730,177 bytes,
including corpus and both stores; no global page-cache dropping occurred.

## Measured Bottleneck

These debug-build timings strongly indicate repeated whole-shard verification
dominates the 250 ms interactive deadline: opening a 58.3 MB synthetic shard took
seconds while querying its already-open indexes took milliseconds. This is a
profile-supported hypothesis, not an optimized-build performance guarantee.
The cache follow-up below tests a bounded immutable-object verification cache
keyed by file identity and change metadata while preserving cancellation and
corruption checks. Optimized-build and realistic concurrent-load repeats remain
necessary; shorter empty partial responses would not constitute a useful result.

The host was WSL2 Linux 6.6.87.2 on an AMD Ryzen AI Max+ Pro 395, 16 physical / 32
logical CPUs and 94.07 GiB RAM. Workloads were restricted to logical CPUs 0 and 1,
1.5 GiB per-process address space, 1,900 MiB sampled process-tree RSS, and 4 GiB
scratch data. The host ran unrelated development/test work and had an uncontrolled
page cache. No hardware-cost, storage-throughput, controlled cold/warm, HTTP-only,
concurrent-load, or baseline comparison claims follow from this run.

## Reproduction and Gates

Commands and resource details are in [benchmarks/README.md](../../benchmarks/README.md).
The manifest and standalone Cargo lock make future runs reproducible from a clean
candidate commit. `validate.py report` recomputes smoke percentiles, checks raw
sample counts/resources/category counts, and rejects unearned tier/p99 claims.
Fourteen automated Python tests cover rejection logic, useful versus empty
responses, and real runner deadline,
RSS/CPU collection, executable freezing, and symlink-safe disk accounting.

Still required: clean optimized-build repeats; useful-content success after the
identified bottleneck is fixed; equivalent pinned baseline; realistic configuration
and dependency variety; independent LOC/fact scaling to L/S/O; sustained concurrent
index/query/compaction loads; complete failure injection and repeated cache
conditions; browser/session timing; and resource/hardware-cost accounting.

The [human-study protocol](../../benchmarks/study/protocol.md) and header-only CSV
define U1-U10, matched variants, at least eight real consenting participants,
counterbalanced tool order, correctness and timing. No participants were recruited
and no results were synthesized. The CSV validator rejects the empty template;
independent consent/session auditing remains necessary even for a valid CSV.

## Committed-Build Cache Follow-Up

The [unchanged synthetic workload rerun](../../benchmarks/reports/synthetic-100k-cached-20260915.json)
was built from clean commit `33be68174d82d39a9418509bbcbfa57326575219`, with
the committed standalone lockfile and a separate target directory. The runner
froze and verified the executable before launch. The fact digest, 100,002 record
count, 825 unknowns, and 58,739,808 store bytes exactly match the preliminary run.

All 120 ordinary search/graph/source samples returned useful bounded content;
none reached its request deadline or failed. All 30 deliberately pre-cancelled
samples remained explicit empty deadline-partial responses. Pagination/window
truncation is retained and is not counted as a failure when useful content exists.

| Workload | p50 | p95 | Identical result in all 30 samples |
| --- | ---: | ---: | --- |
| Search | 9.75 ms | 12.68 ms | 50 items, paginated |
| High-degree graph | 74.49 ms | 103.52 ms | 200 nodes / 222 edges, node-budget partial |
| Two-hop graph | 4.25 ms | 5.97 ms | 3 nodes / 8 edges, not truncated |
| Source window | 52.34 ms | 70.73 ms | 1,900 bytes, bounded window |
| Pre-cancelled graph | 1.49 ms | 3.85 ms | Explicitly cancelled, no graph contents |
| Warm verified reader open | 1.34 ms | 1.69 ms | Cache hit with identity checks |
| Already-open indexed search | 1.79 ms | 3.37 ms | 50 items |
| Already-open adjacency | 8.70 ms | 13.27 ms | 501 bounded profiling rows |

Publication, including cache seeding, took 19.17 s; the complete measured process
took 28.70 s, 26.35 user + 1.56 system CPU seconds, and 138.73 MiB sampled peak
process-tree RSS. Missing-shard injection still returned `unavailable_shard`, and
restoration passed integrity checking. Total benchmark scratch, including the
isolated build target, remained below 436 MB, well under the 4 GiB cap.

The cache holds at most 256 completed verifications, no file contents. It checks
expected hash, device/inode, size, nanosecond mtime/ctime, and mode; changed files
are rehashed. New cold interactive requests still obey cancellation and can
return budget-partial until explicit preparation completes. Publication seeds
the same-process cache. A separate CLI process does not inherit it. The local
filesystem's change metadata is trusted; this is not protection against a hostile
privileged filesystem. Shard verification is bounded to 512 MiB objects.

All 40 store tests and Clippy with warnings denied passed, including eight new
cache/corruption/cancellation/identity/symlink regressions. The earlier binary's
source provenance and shared-host conditions prevent a controlled speedup claim.
This follow-up demonstrates useful warm responses for this generated workload,
not a tier, real-corpus semantic, optimized-build, or sustained-load qualification.

## Real Corpus Prepared-Query Follow-Up

The [fresh regex index and cold CLI samples](../../benchmarks/reports/regex-cached-20260915.json)
and [separate prepared-process samples](../../benchmarks/reports/regex-cached-warm-20260915.json)
use the same reviewed public commit and read-only configuration as the initial
real run. The CLI was built with Rust 1.97.1, default debug profile, from clean
commit `18fc4a989c6edf5b92b16d84f6648397ca92578d`; frozen SHA-256 is
`7b8dc7b52af8108abe224af018210493602d410090e7220652139fea709031ba`.
The root build used a shared development target, not a hermetic rebuild. The
runner independently verified and copied the frozen executable before each run.

The new index contains 54,558 records: 222 source files, 8,361 definitions,
20,668 relations, 20,668 evidence records, and 4,639 source-flow records. All
20,668 relations remain unresolved: 17,618 `cfg_unknown` and 3,050
`macro_unavailable`. Useful source/symbol/unknown-frontier responses are not
evidence of resolved-call precision or complete semantic analysis.

| Workload | p50 | p95 | Result in all 30 samples |
| --- | ---: | ---: | --- |
| Fresh-process CLI prefix `parse` | 4,293.33 ms | 5,034.63 ms | 50 items, paginated |
| Fresh-process CLI one-hop graph | 4,529.20 ms | 5,923.26 ms | 1 node / 97 edges, not truncated |
| Prepared-process empty-prefix search | 28.53 ms | 38.35 ms | 50 items, paginated |

All 90 query samples returned useful content with no failed or deadline samples.
The first two rows include cold checksum preparation and CLI startup on every
sample. The third row is a different search workload measured inside one process,
after a separately recorded 4,545.09 ms preparation. Its entire process took
5,508.03 ms, 5.27 user + 0.18 system CPU seconds, and 23.5 MiB sampled peak RSS.
These rows are not interchangeable HTTP latency or controlled speedup evidence.

Indexing took 165.12 s, 116.55 user + 38.71 system CPU seconds, and 190.82 MiB
sampled peak process-tree RSS. The fresh store occupied 79,278,380 bytes and its
scratch directory, including frozen executable, occupied 202,551,681 bytes. No
resource or outer wall budget was exceeded. The largest real Rust file was
266,293 bytes (`regex-syntax/src/unicode_tables/property_bool.rs`); this corpus
does not establish bounded cancellation for multi-megabyte individual sources.
The synthetic source-window follow-up above likewise measures only a 1,900-byte
returned window, not arbitrary giant-source responsiveness.

Reproduction uses the committed runner's `real` mode with the pinned corpus,
fresh scratch, frozen CLI and 30 samples, followed by `warm` using the same scratch,
CLI and new index report. The warm report links the exact index-report SHA-256.
No external repository build scripts or tools were executed. Optimized builds,
controlled cache states, resolved-call workloads, concurrent service load,
equivalent baselines and real participant sessions remain unqualified gates.
