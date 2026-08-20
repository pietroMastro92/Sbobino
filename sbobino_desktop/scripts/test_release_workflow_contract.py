#!/usr/bin/env python3

import pathlib
import os
import subprocess
import tempfile
import unittest
import wave


ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW = ROOT.parent / ".github" / "workflows" / "release.yml"
PROMOTION = ROOT / "scripts" / "promote_candidate_release.sh"
INTEL_VALIDATION = ROOT.parent / ".github" / "workflows" / "intel-runtime-validation.yml"
INTEL_SMOKE = ROOT / "scripts" / "release_intel_pyannote_parakeet_smoke.sh"
ARM_LIVE_SMOKE = ROOT / "scripts" / "release_macos_parakeet_live_smoke.sh"


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

    def test_real_quality_evidence_comes_from_distinct_packaged_engine_jobs(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        intel = INTEL_SMOKE.read_text(encoding="utf-8")
        live = ARM_LIVE_SMOKE.read_text(encoding="utf-8")
        self.assertIn("release_intel_pyannote_parakeet_smoke.sh", workflow)
        self.assertIn("release_macos_parakeet_live_smoke.sh", workflow)
        self.assertIn('(\"asr_reference\", smoke.get(\"asr_reference\"))', workflow)
        self.assertIn('(\"live_latency\", live)', workflow)
        self.assertIn("evaluate_asr_reference.py", intel)
        self.assertIn("SBOBINO_ASR_TIMELINE_OUTPUT", intel)
        self.assertIn("review_status", intel)
        self.assertIn("parakeet_realtime_c_api_streams_real_wav", live)
        self.assertIn("SBOBINO_PARAKEET_LIVE_REALTIME=1", live)
        self.assertIn("PARAKEET_LIVE_FEED_SAMPLES", (ROOT / "apps" / "desktop" / "src-tauri" / "src" / "parakeet_realtime.rs").read_text(encoding="utf-8"))
        self.assertNotIn("poliglot", intel.lower() + live.lower())
        self.assertIn('FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"', intel)
        self.assertIn('FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"', live)
        self.assertNotIn("need_cmd ffmpeg", intel)
        self.assertNotIn("command in gh ditto ffmpeg", live)

    def test_pyannote_abi_scan_includes_the_packaged_runtime_lib_directory(self):
        readiness = (ROOT / "scripts" / "distribution_readiness.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn('root / "lib" / "embedded-dylibs",\n            root / "lib",', readiness)

    def test_intel_validation_uses_only_standard_free_runner_and_real_worker(self):
        workflow = INTEL_VALIDATION.read_text(encoding="utf-8")
        self.assertIn("runs-on: macos-15-intel", workflow)
        self.assertNotIn("-large", workflow)
        self.assertNotIn("larger", workflow.lower())
        self.assertIn("package_macos_runtime_asset.sh", workflow)
        self.assertIn("smoke_parakeet_real.sh", workflow)
        self.assertIn("SBOBINO_PARAKEET_FORCE_CPU=1", workflow)
        self.assertIn("SBOBINO_PARAKEET_REQUIRE_WORKER_RSS_MONITOR=1", workflow)
        self.assertIn("SBOBINO_PARAKEET_SMOKE_MODE=service", workflow)

    def test_intel_release_smoke_transcribes_the_long_fixture_only_once(self):
        intel = INTEL_SMOKE.read_text(encoding="utf-8")
        harness = (ROOT / "scripts" / "smoke_parakeet_real.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn("SBOBINO_PARAKEET_SMOKE_MODE=service", intel)
        self.assertIn('SMOKE_MODE=${SBOBINO_PARAKEET_SMOKE_MODE:-both}', harness)
        self.assertIn('if [[ "$SMOKE_MODE" == "service" || "$SMOKE_MODE" == "both" ]]', harness)
        self.assertNotIn("need_cmd ffmpeg", harness)
        audio_helpers = (ROOT / "scripts" / "lib" / "asr_samples.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn("import wave", audio_helpers)
        self.assertIn("audio.getframerate() != 16000", audio_helpers)

    def test_normalized_wav_fast_path_needs_no_host_ffmpeg_or_ffprobe(self):
        helper = ROOT / "scripts" / "lib" / "asr_samples.sh"
        with tempfile.TemporaryDirectory() as temp_dir:
            source = pathlib.Path(temp_dir) / "normalized.wav"
            unused_output = pathlib.Path(temp_dir) / "unused.wav"
            with wave.open(str(source), "wb") as audio:
                audio.setnchannels(1)
                audio.setsampwidth(2)
                audio.setframerate(16000)
                audio.writeframes(b"\0\0" * 16000)
            env = os.environ.copy()
            env["PATH"] = "/usr/bin:/bin"
            result = subprocess.run(
                [
                    "/bin/bash",
                    "-c",
                    'source "$1"; asr_prepare_wav "$2" "$3"; '
                    '[[ "$ASR_NORMALIZED_WAV" == "$2" ]]; '
                    '[[ "$(asr_audio_duration_seconds "$2")" == "1.000" ]]',
                    "_",
                    str(helper),
                    str(source),
                    str(unused_output),
                ],
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(unused_output.exists())

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
