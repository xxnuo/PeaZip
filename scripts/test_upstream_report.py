import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import upstream_report


class ReportTests(unittest.TestCase):
    def test_changes_and_invalid_history(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True).strip()

            git("init", "-q")
            git("config", "user.email", "test@example.invalid")
            git("config", "user.name", "Test")
            (root / "feature.txt").write_text("old")
            git("add", ".")
            git("commit", "-qm", "baseline")
            baseline = git("rev-parse", "HEAD")
            (root / "upstream-baseline.json").write_text(json.dumps({
                "peazip": {"revision": baseline, "ref": "HEAD"}
            }))
            with patch.object(upstream_report, "ROOT", root):
                self.assertEqual(upstream_report.report()["commits"], [])
                (root / "feature.txt").write_text("new")
                git("add", "feature.txt")
                git("commit", "-qm", "changed behavior")
                result = upstream_report.report()
                self.assertEqual(result["files"], ["M\tfeature.txt"])
                self.assertEqual(len(result["commits"]), 1)
                self.assertIn("changed behavior", result["commits"][0])
                with self.assertRaises(subprocess.CalledProcessError):
                    upstream_report.report("missing-ref")
                git("checkout", "--orphan", "unrelated")
                git("commit", "-qm", "unrelated")
                with self.assertRaises(subprocess.CalledProcessError):
                    upstream_report.report()


if __name__ == "__main__":
    unittest.main()
