from __future__ import annotations

from pathlib import Path
import re
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"


class CiExecutionPolicyTests(unittest.TestCase):
    def test_new_revisions_do_not_cancel_older_ci_runs(self) -> None:
        workflow = WORKFLOW.read_text()
        self.assertIsNone(re.search(r"^concurrency:", workflow, re.MULTILINE),
                          "Every revision must complete independently, including queued Chrome runs")

    def test_both_browser_jobs_require_full_runs_and_diagnostic_uploads(self) -> None:
        workflow = WORKFLOW.read_text()
        for name, engine in (("cdp-smoke", "moli"), ("cdp-smoke-chromium", "chromium")):
            with self.subTest(job=name):
                match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:|\Z)",
                                  workflow, re.MULTILINE | re.DOTALL)
                self.assertIsNotNone(match)
                job = match.group(1)
                self.assertRegex(job, rf"--browser {engine} --strict-full-suite --repeat 2")
                self.assertNotIn("continue-on-error:", job)
                self.assertIn("if-no-files-found: error", job)
