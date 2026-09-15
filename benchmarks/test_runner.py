import importlib.util
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("qualification_run", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class BoundedRunnerTests(unittest.TestCase):
    def test_executable_copy_is_stable_and_modification_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path(directory, "original")
            original.write_bytes(b"reviewed executable fixture")
            frozen = runner.freeze_binary(original, directory)
            self.assertEqual(runner.digest(original), runner.digest(frozen))
            frozen.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "modified"):
                runner.freeze_binary(original, directory)

    def test_disk_walk_does_not_follow_symlinks(self):
        with tempfile.TemporaryDirectory() as directory, tempfile.TemporaryDirectory() as outside:
            Path(outside, "other").write_bytes(b"outside")
            Path(directory, "link").symlink_to(outside, target_is_directory=True)
            Path(directory, "own").write_bytes(b"abc")
            self.assertEqual(runner.disk_bytes(directory), 3)

    def test_success_captures_real_process_cpu_and_rss(self):
        with tempfile.TemporaryDirectory() as directory:
            record, output = runner.bounded_process([sys.executable, "-c", "print('measured')"], directory)
            self.assertTrue(record["ok"])
            self.assertEqual(output, "measured\n")
            self.assertGreater(record["max_single_process_rss_kib"], 0)
            self.assertIn("user_cpu_s", record)

    def test_deadline_stops_process_without_erasing_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            record, _ = runner.bounded_process([sys.executable, "-c", "import time; time.sleep(5)"], directory, timeout=.05)
            self.assertFalse(record["ok"])
            self.assertEqual(record["failure_budget"], "wall_deadline")
            self.assertLess(record["wall_ms"], 2000)

    def test_warm_measurement_checks_binary_and_preserves_separate_preparation(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory, "binary")
            binary.write_bytes(b"unit fixture, not executed")
            snapshot = {"id": "snapshot:unit", "fact_digest": "facts:unit"}
            previous = {"kind": "real_public_corpus", "index": {"ok": True}, "binary_sha256": runner.digest(binary),
                        "snapshot": snapshot, "corpus": {}, "facts_count": 2, "fact_counts": {},
                        "unknown_relations": 0, "unknowns_by_reason": {}, "store_bytes": 1}
            index_report = Path(directory, "index.json")
            index_report.write_text(json.dumps(previous))
            args = SimpleNamespace(atlas=binary, scratch=Path(directory), index_report=index_report, level="semantic", samples=30)
            native = {"snapshot": snapshot, "samples_ms": [2.0] * 30,
                      "sample_results": [{"ok": True, "items": 50, "deadline_reached": False}] * 30,
                      "preparation_ms": 1234.0}
            with patch.object(runner, "bounded_process", return_value=({"ok": True}, json.dumps(native))):
                result = runner.warm(args)
            self.assertEqual(result["preparation_ms"], 1234.0)
            self.assertEqual(result["workloads"][0]["p95_ms"], 2.0)
            self.assertEqual(len(result["workloads"][0]["samples"]), 30)
            previous["binary_sha256"] = "0" * 64
            index_report.write_text(json.dumps(previous))
            with self.assertRaisesRegex(ValueError, "identical executable"):
                runner.warm(args)


if __name__ == "__main__":
    unittest.main()
