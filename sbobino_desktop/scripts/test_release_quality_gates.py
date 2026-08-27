#!/usr/bin/env python3

import importlib.util
import json
import pathlib
import shutil
import tempfile
import unittest


MODULE_PATH = pathlib.Path(__file__).with_name("run_release_quality_gates.py")
SPEC = importlib.util.spec_from_file_location("run_release_quality_gates", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReleaseQualityGateTests(unittest.TestCase):
    def test_pinned_redistributable_fixtures_pass_both_gates(self):
        with tempfile.TemporaryDirectory() as directory:
            asr, live = MODULE.run(MODULE.DEFAULT_FIXTURE_DIR, pathlib.Path(directory))
            self.assertEqual(asr["status"], "passed")
            self.assertEqual(live["status"], "passed")
            self.assertEqual(asr["evidence_class"], MODULE.SYNTHETIC_EVIDENCE_CLASS)
            self.assertEqual(live["evidence_class"], MODULE.SYNTHETIC_EVIDENCE_CLASS)
            self.assertFalse(asr["real_engine"])
            self.assertFalse(live["real_harness"])
            self.assertEqual(asr["fixture_source"], "repo-redistributable")
            self.assertEqual(live["fixture_source"], "repo-redistributable")
            self.assertEqual(asr["fixture_id"], live["fixture_id"])

    def test_public_json_proof_asset_contract_is_exact_and_allows_non_json(self):
        names = set(MODULE.PUBLIC_JSON_PROOF_ASSETS)
        names.update({"release-notes.md", "Sbobino_2.0.29_aarch64.dmg", "Sbobino_2.0.29_windows_x86_64-setup.exe"})
        MODULE.validate_public_json_proof_assets(names)

        with self.assertRaisesRegex(ValueError, "unexpected=.*extra-proof.json"):
            MODULE.validate_public_json_proof_assets(names | {"extra-proof.json"})

        with self.assertRaisesRegex(ValueError, "missing=.*distribution-readiness-proof.json"):
            MODULE.validate_public_json_proof_assets(names - {"distribution-readiness-proof.json"})

    def test_real_hosted_report_rejects_synthetic_fixture_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            asr, _ = MODULE.run(MODULE.DEFAULT_FIXTURE_DIR, pathlib.Path(directory))
        with self.assertRaisesRegex(ValueError, "not hosted packaged-engine"):
            MODULE.validate_real_hosted_quality_report(asr, "ASR reference")

    def test_real_hosted_report_contract_accepts_packaged_engine_metadata(self):
        report = {
            "schema_version": 1,
            "status": "passed",
            "evidence_class": MODULE.REAL_HOSTED_EVIDENCE_CLASS,
            "real_engine": True,
            "real_harness": True,
            "runner": "github-hosted macos-15-intel",
            "engine": "parakeet",
            "harness": "release-intel-asr-live-harness",
            "input_audio_sha256": "a" * 64,
            "runtime_artifact_sha256": {"speech-runtime": "b" * 64},
            "failures": [],
            "metrics": {"wer": 0.1},
        }
        MODULE.validate_real_hosted_quality_report(report, "ASR reference")

    def test_fixture_hash_change_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture_dir = pathlib.Path(directory) / "fixtures"
            shutil.copytree(MODULE.DEFAULT_FIXTURE_DIR, fixture_dir)
            manifest_path = fixture_dir / "release_quality_manifest.json"
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            manifest["sha256"]["release_live_latency.json"] = "0" * 64
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                MODULE.run(fixture_dir, pathlib.Path(directory) / "reports")


if __name__ == "__main__":
    unittest.main()
