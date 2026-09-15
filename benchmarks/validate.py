#!/usr/bin/env python3
"""Validate smoke evidence and real-human study completeness, never invent it."""
import argparse
import csv
import json
import math
from pathlib import Path
import re


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def validate_report(report):
    require(report.get("version") == 1, "unsupported report version")
    require(report.get("kind") in {"real_public_corpus", "synthetic_indexed_facts"}, "unknown corpus category")
    require(report.get("qualification_status") == "not_qualified" and report.get("tier_claims") == [],
            "this smoke schema cannot support F/L/S/O/X qualification claims")
    require(report.get("baseline", {}).get("status") == "not_run", "this schema cannot support baseline speedup claims")
    limits = report.get("resource_limits", {})
    require(0 < limits.get("max_process_tree_rss_bytes", 0) <= 2 * 1024**3, "RSS limit exceeds approved budget")
    require(0 < limits.get("max_new_scratch_bytes", 0) <= 4 * 1024**3, "disk limit exceeds approved budget")
    if report["kind"] == "real_public_corpus":
        corpus = report.get("corpus", {})
        require(bool(re.fullmatch(r"[0-9a-f]{40}", corpus.get("commit", ""))), "real corpus needs a pinned commit")
        require(corpus.get("replicated_loc") == 0 and number(corpus.get("unique_non_generated_rust_loc")),
                "real unique LOC must be explicit and cannot include replication")
    else:
        if report.get("process", {}).get("ok"):
            require(report.get("frontend_extraction") is False, "synthetic facts cannot claim frontend extraction")
            expected = sum(report.get(key, 0) for key in ("definitions", "relations", "evidence", "source_files", "flows"))
            require(report.get("facts_count") == expected, "synthetic fact count must match actual record categories")
    measurements = [report[key] for key in ("index", "process") if key in report]
    require(bool(measurements), "report must contain an actual index/process measurement")
    if any(measurement.get("ok") for measurement in measurements):
        require(bool(report.get("workloads")), "successful indexing must include measured query workloads")
    total_samples, failed_samples, partial_samples, deadline_samples = 0, 0, 0, 0
    ordinary_samples, useful_samples = 0, 0
    for workload in report.get("workloads", []):
        samples = workload.get("samples", [])
        require(len(samples) >= 30, "each measured workload requires 30 raw samples")
        require(workload.get("p99_ms") is None, "30-sample smoke reports cannot claim p99")
        require(all(number(sample.get("wall_ms")) and type(sample.get("ok")) is bool for sample in samples),
                "invalid raw duration or outcome")
        times = sorted(sample["wall_ms"] for sample in samples)
        for key, percentile in (("p50_ms", .5), ("p95_ms", .95)):
            expected = times[math.ceil(len(times) * percentile) - 1]
            require(number(workload.get(key)) and abs(workload[key] - expected) < 1e-6, "percentile does not match raw samples")
        measurements.extend(samples)
        total_samples += len(samples)
        failed_samples += sum(not sample["ok"] for sample in samples)
        partial_samples += sum(bool(sample.get("result", {}).get("partial")) for sample in samples)
        deadline_samples += sum(bool(sample.get("result", {}).get("deadline")) for sample in samples)
        if not workload.get("name", "").startswith(("profile_", "pre_cancelled_")):
            ordinary_samples += len(samples)
            for sample in samples:
                result = sample.get("result", {})
                useful_samples += int(sample["ok"] and not result.get("deadline", False)
                                      and any(result.get(key, 0) > 0 for key in ("items", "nodes", "bytes")))
    for measurement in measurements:
        require(measurement.get("sampled_process_tree_peak_rss_bytes", 0) <= 2 * 1024**3,
                "observed process tree exceeded approved memory budget")
        require(measurement.get("max_single_process_rss_kib", 0) * 1024 <= 2 * 1024**3,
                "kernel-recorded process peak exceeded approved memory budget")
        require(measurement.get("disk_bytes_after", 0) <= 4 * 1024**3, "observed disk exceeded approved budget")
    require(report.get("store_bytes", 0) <= 4 * 1024**3, "index exceeded disk budget")
    return {"valid": True, "qualified": False, "kind": report["kind"], "raw_samples": total_samples,
            "failed_samples": failed_samples, "partial_samples": partial_samples, "deadline_samples": deadline_samples,
            "ordinary_response_samples": ordinary_samples, "useful_response_samples": useful_samples,
            "all_ordinary_responses_useful": ordinary_samples > 0 and useful_samples == ordinary_samples,
            "facts_count": report.get("facts_count"), "index_succeeded": report.get("index", report.get("process", {})).get("ok")}


STUDY_COLUMNS = ["participant_id", "participant_origin", "consent_recorded", "session_id", "order",
                 "condition", "task_id", "variant", "duration_seconds", "correct", "confidence",
                 "navigation_actions", "notes"]


def validate_study(rows, columns):
    require(columns == STUDY_COLUMNS, "study CSV columns differ from the registered schema")
    require(bool(rows), "no human study has been conducted; empty templates are not results")
    people, keys = {}, set()
    for row in rows:
        require(row["participant_origin"] == "real_human", "synthetic/agent observations are not human participants")
        require(row["consent_recorded"] == "yes", "consent must be recorded outside the public report")
        participant = row["participant_id"]
        require(bool(re.fullmatch(r"[A-Za-z0-9_-]{2,32}", participant)), "use pseudonymous participant identifiers")
        require(row["order"] in {"atlas_first", "baseline_first"}, "invalid counterbalance order")
        require(row["condition"] in {"atlas", "baseline"}, "invalid tool condition")
        require(row["task_id"] in {f"U{i}" for i in range(1, 11)}, "invalid task family")
        require(row["variant"] in {"A", "B"}, "invalid matched-task variant")
        expected = "A" if (row["condition"] == "atlas") == (row["order"] == "atlas_first") else "B"
        require(row["variant"] == expected, "variant/order assignment is not counterbalanced")
        require(row["correct"] in {"0", "1"}, "correctness must use the predeclared answer rubric")
        try:
            elapsed, confidence, actions = float(row["duration_seconds"]), int(row["confidence"]), int(row["navigation_actions"])
        except ValueError as error:
            raise ValueError("invalid study numeric value") from error
        require(math.isfinite(elapsed) and elapsed > 0 and 1 <= confidence <= 5 and actions >= 0, "study numeric value out of range")
        key = (participant, row["task_id"], row["condition"])
        require(key not in keys, "duplicate participant/task/condition")
        keys.add(key)
        previous = people.setdefault(participant, (row["order"], row["session_id"]))
        require(previous == (row["order"], row["session_id"]) and bool(row["session_id"]), "inconsistent participant session/order")
    require(len(people) >= 8, "at least eight independently verified real participants are required")
    orders = [value[0] for value in people.values()]
    require(orders.count("atlas_first") >= 4 and orders.count("baseline_first") >= 4 and abs(orders.count("atlas_first") - orders.count("baseline_first")) <= 1,
            "tool order groups must be balanced, with at least four participants each")
    for participant in people:
        require(sum(key[0] == participant for key in keys) == 20, "each participant needs ten matched tasks in both conditions")
    return {"complete_schema": True, "participant_count": len(people), "observations": len(rows),
            "human_authenticity": "requires independent consent/session audit; CSV validation cannot establish authenticity"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=["report", "study"])
    parser.add_argument("path", type=Path)
    args = parser.parse_args()
    if args.kind == "report":
        result = validate_report(json.loads(args.path.read_text()))
    else:
        with args.path.open(newline="") as stream:
            reader = csv.DictReader(stream)
            result = validate_study(list(reader), reader.fieldnames)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
