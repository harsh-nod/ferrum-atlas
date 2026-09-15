# Bounded Qualification Evidence

These tools collect smoke evidence, not F/L/S/O/X qualification. Real source,
generated source, and directly generated indexed facts are separate categories.
Reports retain every sample, unknown frontier, failure, and resource measurement.
Thirty samples are insufficient for a credible p99; this schema forbids that claim.

## Public Corpus

The reviewed manifest pins rust-lang/regex 1.11.1 to a full commit SHA and hashes
its MIT, Apache-2.0, and generated Unicode notices. The checkout is not committed
here. Review notices again if changing the pin. No Cargo commands, build scripts,
proc macros, tests, submodules, or tools from the checkout are executed.

Use a new scratch directory outside this public repository:

```sh
export SCRATCH=/tmp/ferrum-atlas-qualification
mkdir -p "$SCRATCH"
env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_CONFIG_NOSYSTEM=1 \
  git -c core.hooksPath=/dev/null -c core.fsmonitor=false clone \
  --depth 1 --single-branch --branch 1.11.1 --no-recurse-submodules \
  https://github.com/rust-lang/regex.git "$SCRATCH/regex"
git -C "$SCRATCH/regex" rev-parse HEAD
```

The result must be `9870c06e6c772daaad7ab612faab29130753e41c`. The runner
independently rejects a different commit, modified/untracked checkout, manifest,
or license checksum. Rust LOC means physical lines, not statements. The configured
`unicode_tables` path rule separates known generated tables; other generated code
is not automatically detectable. Dependency source outside the checkout is absent.

## Run

Prerequisites: Linux `/proc`, Python 3.11+, GNU time, Git, and the project's pinned
Rust toolchain. Build only Ferrum Atlas and its standalone harness, not the corpus:

```sh
cargo build --locked -p ferrum-atlas -j 1
CARGO_TARGET_DIR="$SCRATCH/harness-target" \
  cargo build --locked --manifest-path benchmarks/stress/Cargo.toml -j 1
python3 benchmarks/run.py real --scratch "$SCRATCH" --corpus "$SCRATCH/regex" \
  --atlas target/debug/atlas --samples 30 --output "$SCRATCH/real-report.json"
python3 benchmarks/run.py synthetic --scratch "$SCRATCH" \
  --binary "$SCRATCH/harness-target/debug/atlas-index-stress" --samples 30 \
  --output "$SCRATCH/synthetic-report.json"
python3 benchmarks/validate.py report "$SCRATCH/real-report.json"
python3 benchmarks/validate.py report "$SCRATCH/synthetic-report.json"
python3 -m unittest discover -s benchmarks -p 'test_*.py'
```

Use a new scratch directory for each run. Existing measurement stores and report
files are never overwritten. Executables are copied and SHA-256 verified before
new runs so a concurrent developer build cannot change the measured executable.
Record the actual clean build commit independently; a digest alone does not prove
source provenance. The committed preliminary regex report predates executable
freezing and explicitly retains this provenance limitation.

The runner pins work to two available logical CPUs, limits each process to
1.5 GiB address space and 900 CPU seconds, samples the aggregate process tree at
10 ms, stops at 1,900 MiB observed RSS, and caps scratch data at 4 GiB. Outputs,
single files, wall time, and index-worker memory/disk budgets are bounded too.
Compiler setup is separate from workload measurements; build with one job and
budget its target directory independently. No global cache dropping is performed.
The shared-host page cache is uncontrolled, and first samples remain in results.
GNU time records CPU seconds, kernel maximum single-process RSS, and filesystem
input/output operation counters (not byte throughput). RSS sampling can miss short
peaks; no process-tree hard-memory cgroup claim is made.

The real workload measures CLI startup plus query execution. The synthetic
workload invokes the real immutable store and query engine in process, separately
timing publication, reader checksum/open, already-open search, and adjacency.
The generated 100,002 records are not frontend-extracted semantic claims about
the placeholder source. High-degree truncation, cancellation, and missing shards
are exercised, with no user store modified. Results are not directly comparable
between CLI and in-process measurements.

## Gates Not Satisfied

The validator intentionally returns `qualified: false`. There is no equivalent
baseline, controlled cold/warm cache pair, sustained concurrent-load run, repeated
reference-tier machine run, distributed qualification, or human result. The human
study protocol and empty CSV are under `study/`; the empty template must fail
`validate.py study`. Do not fill it with generated participants or agent answers.

See [observed results](../docs/milestones/qualification-next.md) for the actual
run, limitations, measured bottleneck, and remaining gates.
