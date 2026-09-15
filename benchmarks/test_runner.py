import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
