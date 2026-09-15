# Qualification: Measured Next Gates

Status on 2026-09-15: reproducible smoke tooling and 300 actual query/profile
samples recorded. **No F/L/S/O/X tier passes, speedup, semantic accuracy, or human
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
Next work should validate a bounded immutable-object verification cache keyed by
file identity and change metadata, preserve cancellation/corruption checks, then
repeat identical workloads. An optimization is not a win until queries return
the expected bounded contents, not just a faster empty partial response.

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
Twelve automated Python tests cover rejection logic and real runner deadline,
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
