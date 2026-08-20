#!/usr/bin/env python3

import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW = ROOT.parent / ".github" / "workflows" / "release.yml"
PROMOTION = ROOT / "scripts" / "promote_candidate_release.sh"


class ReleaseWorkflowContractTests(unittest.TestCase):
    def test_synthetic_quality_job_is_contract_only(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("release-quality-contract-tests:", workflow)
        self.assertIn("run_release_quality_gates.py", workflow)
        self.assertIn("Synthetic ASR/live contract tests (not release evidence)", workflow)
        self.assertIn("Upload contract-test reports (never folded into proof)", workflow)
        self.assertNotIn("name: release-quality-contract-tests\n          path: validation-assets", workflow)
        self.assertIn("hosted-packaged-engine", workflow)
        self.assertIn("real_engine", workflow)
        self.assertIn("real_harness", workflow)
        self.assertIn("candidate remains unpromotable", workflow)
        self.assertNotIn("poliglot", workflow.lower())

    def test_workflow_validates_exact_public_json_set_without_requiring_json_only_release(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("Require exactly the public seven JSON proof assets", workflow)
        self.assertIn("--validate-public-proof-assets validation-assets", workflow)
        self.assertIn("release-quality-contract-tests.result", workflow)
        self.assertIn("release-notes.md", (ROOT / "docs" / "release-notes" / "v2.0.28.md").read_text(encoding="utf-8"))

    def test_promotion_requires_both_nested_quality_reports(self):
        promotion = PROMOTION.read_text(encoding="utf-8")
        self.assertIn('validate_quality_gate("asr_reference", "ASR reference")', promotion)
        self.assertIn('validate_quality_gate("live_latency", "live-latency")', promotion)
        self.assertIn("expected_json_assets", promotion)
        self.assertIn("unexpected_json", promotion)
        self.assertIn('pathlib.PurePosixPath(name).suffix.lower() == ".json"', promotion)
        self.assertIn('evidence_class") != "hosted-packaged-engine"', promotion)
        self.assertIn('real_engine") is not True', promotion)
        for threshold in ("0.35", "0.25", "256.0"):
            self.assertIn(threshold, promotion)
        self.assertIn('release-notes.md', promotion)


if __name__ == "__main__":
    unittest.main()
