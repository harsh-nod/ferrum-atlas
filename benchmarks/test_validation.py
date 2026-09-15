import copy
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("qualification_validate", Path(__file__).with_name("validate.py"))
validation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validation)


def smoke():
    return {"version": 1, "kind": "synthetic_indexed_facts", "qualification_status": "not_qualified", "tier_claims": [],
            "baseline": {"status": "not_run"}, "resource_limits": {"max_process_tree_rss_bytes": 1024**3, "max_new_scratch_bytes": 1024**3},
            "process": {"ok": True}, "frontend_extraction": False, "definitions": 100000, "relations": 0, "source_files": 1, "evidence": 1,
            "facts_count": 100002, "workloads": [{"name": "unit-fixture", "samples": [{"wall_ms": 1.0, "ok": True} for _ in range(30)],
                                                  "p50_ms": 1.0, "p95_ms": 1.0, "p99_ms": None}]}


class QualificationValidation(unittest.TestCase):
    def test_accepts_honest_smoke_without_qualifying_it(self):
        self.assertFalse(validation.validate_report(smoke())["qualified"])

    def test_rejects_unearned_tier_claims(self):
        for tier in ["F", "L", "S", "O", "X"]:
            report = smoke()
            report["tier_claims"] = [tier]
            with self.assertRaises(ValueError):
                validation.validate_report(report)

    def test_rejects_synthetic_as_real_frontend(self):
        report = smoke()
        report["frontend_extraction"] = True
        with self.assertRaises(ValueError):
            validation.validate_report(report)

    def test_rejects_short_samples_p99_or_favorable_summary(self):
        for change in ["short", "p99", "summary"]:
            report = smoke()
            workload = report["workloads"][0]
            if change == "short":
                workload["samples"].pop()
            elif change == "p99":
                workload["p99_ms"] = 1.0
            else:
                workload["p95_ms"] = .1
            with self.assertRaises(ValueError):
                validation.validate_report(report)

    def test_rejects_resource_overrun_and_miscount(self):
        for change in ["memory", "facts"]:
            report = copy.deepcopy(smoke())
            if change == "memory":
                report["process"]["sampled_process_tree_peak_rss_bytes"] = 3 * 1024**3
            else:
                report["facts_count"] += 1
            with self.assertRaises(ValueError):
                validation.validate_report(report)

    def test_success_requires_queries_but_failed_attempt_is_retained(self):
        report = smoke()
        report["workloads"] = []
        with self.assertRaisesRegex(ValueError, "query workloads"):
            validation.validate_report(report)
        report["process"]["ok"] = False
        self.assertEqual(validation.validate_report(report)["raw_samples"], 0)

    def test_useful_content_is_distinct_from_empty_partial_and_expected_cancellation(self):
        report = smoke()
        report["workloads"][0]["samples"] = [{"wall_ms": 1.0, "ok": True, "result": {"items": 0, "partial": True}} for _ in range(30)]
        self.assertFalse(validation.validate_report(report)["all_ordinary_responses_useful"])
        for sample in report["workloads"][0]["samples"]:
            sample["result"]["items"] = 50
        self.assertTrue(validation.validate_report(report)["all_ordinary_responses_useful"])
        cancelled = copy.deepcopy(report["workloads"][0])
        cancelled["name"] = "pre_cancelled_graph"
        for sample in cancelled["samples"]:
            sample["result"] = {"nodes": 0, "deadline": True, "partial": True}
        report["workloads"].append(cancelled)
        summary = validation.validate_report(report)
        self.assertTrue(summary["all_ordinary_responses_useful"])
        self.assertEqual(summary["deadline_samples"], 30)

    def test_empty_human_template_is_not_evidence(self):
        with self.assertRaisesRegex(ValueError, "no human study"):
            validation.validate_study([], validation.STUDY_COLUMNS)

    def test_agent_rows_are_not_human_participants(self):
        with self.assertRaisesRegex(ValueError, "not human participants"):
            validation.validate_study([{"participant_origin": "unit_fixture_agent"}], validation.STUDY_COLUMNS)


if __name__ == "__main__":
    unittest.main()
