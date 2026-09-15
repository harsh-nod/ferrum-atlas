#!/usr/bin/env python3
"""Bounded local measurements. Never runs commands from a corpus checkout."""
import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import signal
import shutil
import sqlite3
import subprocess
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parent
MAX_DISK = 4 * 1024**3
MAX_RSS = 1900 * 1024**2
MAX_AS = 1536 * 1024**2


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def freeze_binary(binary, scratch):
    """Pin an executable copy so concurrent developer rebuilds cannot change samples."""
    binary = Path(binary).resolve()
    identity = digest(binary)
    destination = Path(scratch) / f"binary-{identity}"
    if destination.exists():
        if digest(destination) != identity:
            raise ValueError("frozen executable was modified")
    else:
        shutil.copyfile(binary, destination)
        destination.chmod(0o700)
    if digest(destination) != identity or digest(binary) != identity:
        raise ValueError("executable changed while freezing; rerun with a stable binary")
    return destination


def disk_bytes(root):
    total = 0
    for directory, folders, files in os.walk(root, followlinks=False):
        folders[:] = [name for name in folders if not Path(directory, name).is_symlink()]
        for name in files:
            path = Path(directory, name)
            try:
                if not path.is_symlink():
                    total += path.stat().st_size
            except FileNotFoundError:
                pass
    return total


def process_tree_rss(pid):
    pending, seen, total = [pid], set(), 0
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        try:
            status = Path(f"/proc/{current}/status").read_text()
            total += next((int(line.split()[1]) * 1024 for line in status.splitlines() if line.startswith("VmRSS:")), 0)
            for task in Path(f"/proc/{current}/task").iterdir():
                pending.extend(int(value) for value in (task / "children").read_text().split())
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            pass
    return total


def hardware():
    cpu = Path("/proc/cpuinfo").read_text()
    memory = dict(line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
    identities = set()
    for block in cpu.split("\n\n"):
        fields = dict(line.split(":", 1) for line in block.splitlines() if ":" in line)
        fields = {k.strip(): v.strip() for k, v in fields.items()}
        if "core id" in fields:
            identities.add((fields.get("physical id"), fields["core id"]))
    return {"os": platform.system(), "kernel": platform.release(), "architecture": platform.machine(),
            "cpu_model": next(line.split(":", 1)[1].strip() for line in cpu.splitlines() if line.startswith("model name")),
            "logical_cpus": os.cpu_count(), "physical_cores": len(identities),
            "available_memory_bytes_at_start": int(memory["MemAvailable"].split()[0]) * 1024,
            "total_memory_bytes": int(memory["MemTotal"].split()[0]) * 1024,
            "cpu_affinity_for_workload": sorted(os.sched_getaffinity(0))[:2]}


def bounded_process(command, scratch, timeout=600):
    scratch = Path(scratch)
    before = disk_bytes(scratch)
    if before > MAX_DISK:
        raise ValueError("scratch disk budget exceeded before launch")
    affinity = sorted(os.sched_getaffinity(0))[:2]

    def limits():
        resource.setrlimit(resource.RLIMIT_AS, (MAX_AS, MAX_AS))
        resource.setrlimit(resource.RLIMIT_FSIZE, (512 * 1024**2, 512 * 1024**2))
        resource.setrlimit(resource.RLIMIT_CPU, (900, 901))
        os.sched_setaffinity(0, affinity)

    with tempfile.TemporaryDirectory(prefix="measure-", dir=scratch) as private:
        private = Path(private)
        timing = private / "timing.json"
        fmt = '{"user_cpu_s":%U,"system_cpu_s":%S,"max_single_process_rss_kib":%M,"fs_inputs":%I,"fs_outputs":%O,"exit_code":%x}'
        with (private / "stdout").open("w+b") as output, (private / "stderr").open("w+b") as errors:
            started = time.perf_counter()
            proc = subprocess.Popen(["/usr/bin/time", "-q", "-o", str(timing), "-f", fmt, "--", *map(str, command)],
                                    stdout=output, stderr=errors, start_new_session=True, preexec_fn=limits)
            maximum, failure, last_disk_check = 0, None, 0.0
            while proc.poll() is None:
                now = time.perf_counter()
                maximum = max(maximum, process_tree_rss(proc.pid))
                if now - started > timeout:
                    failure = "wall_deadline"
                elif maximum > MAX_RSS:
                    failure = "process_tree_rss_limit"
                elif output.tell() > 16 * 1024**2 or errors.tell() > 2 * 1024**2:
                    failure = "output_limit"
                if now - last_disk_check > 0.5:
                    if disk_bytes(scratch) > MAX_DISK:
                        failure = "scratch_disk_limit"
                    last_disk_check = now
                if failure:
                    os.killpg(proc.pid, signal.SIGKILL)
                    break
                time.sleep(0.01)
            code = proc.wait()
            elapsed = (time.perf_counter() - started) * 1000
            output.seek(0)
            errors.seek(0)
            stdout = output.read(16 * 1024**2).decode(errors="replace")
            stderr = errors.read(65536).decode(errors="replace").replace(str(scratch), "$SCRATCH")
        timing_text = timing.read_text().strip() if timing.exists() else ""
        # SIGKILL can stop GNU time before it writes counters; retain the failed sample.
        timing_data = json.loads(timing_text) if timing_text else {}
    record = {"wall_ms": elapsed, "ok": code == 0 and not failure, "exit_code": code,
              "failure_budget": failure, "sampled_process_tree_peak_rss_bytes": maximum,
              "disk_bytes_after": disk_bytes(scratch), "kernel_counters_complete": bool(timing_data), **timing_data}
    if stderr:
        record["stderr"] = stderr
    return record, stdout


def base_report(kind, binary):
    return {"version": 1, "kind": kind, "recorded_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "qualification_status": "not_qualified", "tier_claims": [],
            "hardware": hardware(), "binary_sha256": digest(binary),
            "harness_base_commit": subprocess.check_output(["git", "-C", str(ROOT.parent), "rev-parse", "HEAD"], text=True).strip(),
            "binary_source_commit": None,
            "binary_provenance": "Executable SHA-256 pins bytes; build-source revision is not inferred from the harness worktree.",
            "resource_limits": {"max_process_tree_rss_bytes": MAX_RSS, "max_address_space_per_process_bytes": MAX_AS,
                                "max_new_scratch_bytes": MAX_DISK, "cpu_affinity_count": 2, "rss_poll_ms": 10},
            "cache_policy": "Uncontrolled shared-host page cache; no cache dropping; first sample retained.",
            "baseline": {"status": "not_run", "reason": "No equivalent pinned baseline configured; no speedup claim."},
            "limitations": ["Single run on a shared host, two assigned logical CPUs; not a reference-tier machine/load profile.",
                            "Thirty samples support smoke percentiles only; p99 and L/S/O/X qualification are not claimed.",
                            "RSS tree is sampled; GNU time additionally records kernel peak of the largest single process."]}


def summarize(name, samples, measurement):
    times = sorted(sample["wall_ms"] for sample in samples)
    return {"name": name, "measurement": measurement, "samples": samples,
            "p50_ms": times[math.ceil(len(times) * .5) - 1], "p95_ms": times[math.ceil(len(times) * .95) - 1], "p99_ms": None}


def real(args):
    scratch, corpus, binary = args.scratch.resolve(), args.corpus.resolve(), args.atlas.resolve()
    binary = freeze_binary(binary, scratch)
    manifest = json.loads((ROOT / "manifests/public-corpus.json").read_text())["repositories"][0]
    revision = subprocess.check_output(["git", "-C", str(corpus), "rev-parse", "HEAD"], text=True).strip()
    if revision != manifest["commit"]:
        raise ValueError("corpus commit differs from reviewed pin")
    dirty = subprocess.check_output(["git", "-C", str(corpus), "status", "--porcelain", "--untracked-files=all"], text=True)
    if dirty:
        raise ValueError("reviewed corpus checkout must be clean, including untracked files")
    for license_file in manifest["license_files"]:
        if digest(corpus / license_file["path"]) != license_file["sha256"]:
            raise ValueError("corpus license notice checksum differs")
    if digest(corpus / "Cargo.toml") != manifest["manifest_sha256"]:
        raise ValueError("corpus manifest checksum differs")
    tracked = subprocess.check_output(["git", "-C", str(corpus), "ls-files", "-z"]).split(b"\0")
    loc, generated_loc, rust_files = 0, 0, 0
    members = []
    for name in tracked:
        if name == b"Cargo.toml" or name.endswith(b"/Cargo.toml"):
            document = tomllib.loads((corpus / os.fsdecode(name)).read_text())
            if "package" in document:
                members.append({"manifest": os.fsdecode(name), "name": document["package"]["name"],
                                "declared_direct_dependencies": sum(len(document.get(key, {})) for key in ("dependencies", "dev-dependencies", "build-dependencies"))})
        if name.endswith(b".rs"):
            relative = os.fsdecode(name)
            path = corpus / relative
            if path.is_symlink():
                continue
            lines = len(path.read_bytes().splitlines())
            rust_files += 1
            if any(fragment in relative for fragment in manifest["generated_path_fragments"]):
                generated_loc += lines
            else:
                loc += lines
    report = base_report("real_public_corpus", binary)
    report["corpus"] = {"id": manifest["id"], "url": manifest["url"], "commit": revision,
                        "unique_non_generated_rust_loc": loc, "generated_rust_loc": generated_loc,
                        "rust_files": rust_files, "replicated_loc": 0, "configuration_count": 1,
                        "repositories": 1, "retained_revisions": 1, "concurrent_query_clients": 1,
                        "package_manifests": members,
                        "largest_single_crate_loc": "not measured; shared module files prevent unambiguous manifest attribution",
                        "macro_expansion_ratio": "not measured; expansions unavailable in this read-only profile"}
    store = scratch / f"real-{args.level}-store"
    if store.exists():
        raise ValueError("benchmark store must be fresh")
    init, _ = bounded_process([binary, "--store", store, "init", "--workspace", corpus], scratch)
    if not init["ok"]:
        raise ValueError(init)
    progress = scratch / f"real-{args.level}-progress.json"
    index, output = bounded_process([binary, "--store", store, "index", "--level", args.level, "--timeout", "600",
                                     "--memory-mib", "1536", "--disk-quota-mib", "3072", "--progress-file", progress], scratch, 630)
    index["frontend_level"] = args.level
    index["last_recorded_stage"] = json.loads(progress.read_text()) if progress.exists() else None
    report["index"] = index
    report["workloads"] = []
    if not index["ok"]:
        return report
    snapshot = json.loads(output)
    report["snapshot"] = snapshot
    report["store_bytes"] = disk_bytes(store)
    catalog = sqlite3.connect(f"file:{store / 'catalog.sqlite'}?mode=ro", uri=True)
    shard_hash = catalog.execute("SELECT shard_hash FROM snapshots WHERE id=?", (snapshot["id"],)).fetchone()[0]
    shard = sqlite3.connect(f"file:{store / 'shards' / shard_hash}?mode=ro", uri=True)
    counts = {table: shard.execute(f"SELECT count(*) FROM {table}").fetchone()[0] for table in ("files", "definitions", "relations", "evidence", "flows")}
    report["fact_counts"] = counts
    report["facts_count"] = sum(counts.values())
    report["unknown_relations"] = shard.execute("SELECT count(*) FROM relations WHERE target_id IS NULL").fetchone()[0]
    report["unknowns_by_reason"] = dict(shard.execute("SELECT json_extract(payload,'$.target.reason'),count(*) FROM relations WHERE target_id IS NULL GROUP BY 1"))
    row = shard.execute("SELECT source_id,count(*) FROM relations GROUP BY source_id ORDER BY count(*) DESC LIMIT 1").fetchone()
    hub = row[0] if row else shard.execute("SELECT id FROM definitions ORDER BY id LIMIT 1").fetchone()[0]
    report["largest_callsite_fanout"] = row[1] if row else 0
    for name, command in [("symbol_prefix", ["query", "search", "--text", "parse"]),
                          ("one_hop_high_fanout", ["query", "both", "--symbol", hub, "--depth", "1"])]:
        samples = []
        for _ in range(args.samples):
            measured, output = bounded_process([binary, "--store", store, *command, "--snapshot", snapshot["id"]], scratch, 15)
            if measured["ok"]:
                result = json.loads(output)
                measured["result"] = {"items": len(result.get("items", [])), "nodes": len(result.get("nodes", [])),
                                      "edges": len(result.get("edges", [])), "partial": result.get("page", {}).get("truncated", False),
                                      "deadline": result.get("work", {}).get("deadline_reached", False)}
            samples.append(measured)
        report["workloads"].append(summarize(name, samples, "CLI process end-to-end; includes startup, not server-only latency"))
    return report


def synthetic(args):
    binary = freeze_binary(args.binary, args.scratch)
    report = base_report("synthetic_indexed_facts", binary)
    measured, output = bounded_process([binary, args.scratch.resolve() / "synthetic-store", str(args.samples)], args.scratch, 900)
    report["process"] = measured
    report["workloads"] = []
    if measured["ok"]:
        report.update(json.loads(output))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["real", "synthetic"])
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--atlas", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--level", choices=["syntax", "semantic"], default="semantic")
    parser.add_argument("--samples", type=int, default=30)
    args = parser.parse_args()
    if not 30 <= args.samples <= 1000:
        parser.error("samples must be 30..1000; no p99 claim")
    args.scratch.mkdir(parents=True, exist_ok=True)
    if ROOT.parent in args.scratch.resolve().parents or args.scratch.resolve() == ROOT.parent:
        parser.error("scratch must be outside the public repository")
    report = real(args) if args.mode == "real" else synthetic(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    print(json.dumps({"report": str(args.output), "kind": report["kind"], "facts_count": report.get("facts_count"),
                      "successful_process": report.get("index", report.get("process", {})).get("ok"), "qualification_status": report["qualification_status"]}))


if __name__ == "__main__":
    main()
