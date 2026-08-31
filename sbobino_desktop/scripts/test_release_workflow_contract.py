#!/usr/bin/env python3

import json
import pathlib
import os
import subprocess
import tempfile
import unittest
import wave
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKFLOW = ROOT.parent / ".github" / "workflows" / "release.yml"
PROMOTION = ROOT / "scripts" / "promote_candidate_release.sh"
INTEL_VALIDATION = ROOT.parent / ".github" / "workflows" / "intel-runtime-validation.yml"
ARM_VALIDATION = ROOT.parent / ".github" / "workflows" / "arm-runtime-validation.yml"
INTEL_SMOKE = ROOT / "scripts" / "release_intel_pyannote_parakeet_smoke.sh"
ARM_LIVE_SMOKE = ROOT / "scripts" / "release_macos_whisper_live_smoke.sh"
WINDOWS_LIVE_SMOKE = ROOT / "scripts" / "release_windows_whisper_live_smoke.ps1"
WINDOWS_RUNTIME_PACKAGER = ROOT / "scripts" / "package_windows_runtime_asset.ps1"
WINDOWS_PYANNOTE_PACKAGER = ROOT / "scripts" / "package_windows_pyannote_runtime.ps1"
PROVISIONING = ROOT / "apps" / "desktop" / "src-tauri" / "src" / "commands" / "provisioning.rs"
DISPATCHER = ROOT / "scripts" / "dispatch_release_candidate.sh"


class ReleaseWorkflowContractTests(unittest.TestCase):
    def test_candidate_validation_is_bound_to_the_requested_tag_revision(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        dispatcher = DISPATCHER.read_text(encoding="utf-8")
        upload_job = workflow.split("  upload-validation-assets:", 1)[1]
        self.assertIn("ref: ${{ inputs.tag_name }}", upload_job)
        self.assertIn("fetch-depth: 0", upload_job)
        self.assertIn('--ref "$TAG_NAME"', dispatcher)

    def test_release_readiness_metadata_binds_candidate_revision_and_repository(self):
        with tempfile.TemporaryDirectory() as temporary:
            output_dir = pathlib.Path(temporary)
            version = "9.8.7"
            required_assets = [
                f"Sbobino_{version}_aarch64.dmg",
                f"Sbobino_{version}_x86_64.dmg",
                f"Sbobino_{version}_aarch64.app.tar.gz",
                f"Sbobino_{version}_aarch64.app.tar.gz.sig",
                f"Sbobino_{version}_x86_64.app.tar.gz",
                f"Sbobino_{version}_x86_64.app.tar.gz.sig",
                f"Sbobino_{version}_windows_x86_64-setup.exe",
                f"Sbobino_{version}_windows_x86_64.nsis.zip",
                f"Sbobino_{version}_windows_x86_64.nsis.zip.sig",
                "latest.json",
                "setup-manifest.json",
                "runtime-manifest.json",
                "speech-runtime-macos-aarch64.zip",
                "speech-runtime-macos-x86_64.zip",
                "speech-runtime-windows-x86_64.zip",
                "pyannote-manifest.json",
                "pyannote-runtime-macos-aarch64.zip",
                "pyannote-runtime-macos-x86_64.zip",
                "pyannote-runtime-windows-x86_64.zip",
                "pyannote-model-community-1.zip",
                "release-notes.md",
            ]
            zip_members = {
                "speech-runtime-macos-aarch64.zip": [
                    "runtime/bin/ffmpeg",
                    "runtime/bin/whisper-cli",
                    "runtime/bin/whisper-stream",
                    "runtime/bin/parakeet-cli",
                    "runtime/bin/parakeet-batch-json",
                ],
                "speech-runtime-macos-x86_64.zip": [
                    "runtime/bin/ffmpeg",
                    "runtime/bin/whisper-cli",
                    "runtime/bin/whisper-stream",
                    "runtime/bin/parakeet-cli",
                    "runtime/bin/parakeet-batch-json",
                ],
                "speech-runtime-windows-x86_64.zip": [
                    "runtime/bin/ffmpeg.exe",
                    "runtime/bin/whisper-cli.exe",
                    "runtime/bin/whisper-stream.exe",
                    "runtime/bin/parakeet-cli.exe",
                    "runtime/bin/parakeet-batch-json.exe",
                ],
                "pyannote-runtime-macos-aarch64.zip": ["python/bin/python3"],
                "pyannote-runtime-macos-x86_64.zip": ["python/bin/python3"],
                "pyannote-runtime-windows-x86_64.zip": ["python/python.exe"],
                "pyannote-model-community-1.zip": ["model/config.yaml"],
            }
            for name in required_assets:
                path = output_dir / name
                if name in zip_members:
                    with zipfile.ZipFile(path, "w") as archive:
                        for member in zip_members[name]:
                            archive.writestr(member, b"fixture")
                elif name == "latest.json":
                    path.write_text(json.dumps({"version": version}), encoding="utf-8")
                elif name == "setup-manifest.json":
                    path.write_text(
                        json.dumps({"app_version": version, "release_tag": f"v{version}"}),
                        encoding="utf-8",
                    )
                elif name in {"runtime-manifest.json", "pyannote-manifest.json"}:
                    path.write_text(json.dumps({"app_version": version}), encoding="utf-8")
                elif name == "release-notes.md":
                    path.write_text(
                        "## Sbobino 9.8.7\n\n"
                        "### Fixes\n\n- fixture fix\n\n"
                        "### New and improved\n\n- fixture improvement\n\n"
                        "### Compatibility\n\n- fixture compatibility\n\n"
                        "### Known issues\n\n- fixture issue\n\n"
                        "### Refs\n\n- v9.8.6..v9.8.7\n",
                        encoding="utf-8",
                    )
                else:
                    path.write_bytes(b"fixture")
            subprocess.run(
                [
                    "python3",
                    str(ROOT / "scripts" / "generate_release_candidate_metadata.py"),
                    temporary,
                    version,
                    "--commit-sha",
                    "a" * 40,
                    "--repo-slug",
                    "owner/repo",
                ],
                check=True,
            )
            proof = json.loads(
                (pathlib.Path(temporary) / "release-readiness-proof.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(proof["commit_sha"], "a" * 40)
            self.assertEqual(proof["repo_slug"], "owner/repo")

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
        self.assertIn("Stage the candidate readiness proof", workflow)
        self.assertIn('--pattern "release-readiness-proof.json"', workflow)
        self.assertIn("release-quality-contract-tests.result", workflow)
        self.assertIn("release-notes.md", (ROOT / "docs" / "release-notes" / "v2.0.29.md").read_text(encoding="utf-8"))

    def test_real_quality_evidence_comes_from_distinct_packaged_engine_jobs(self):
        workflow = WORKFLOW.read_text(encoding="utf-8")
        intel = INTEL_SMOKE.read_text(encoding="utf-8")
        live = ARM_LIVE_SMOKE.read_text(encoding="utf-8")
        live_runner = (ROOT / "scripts" / "run_whisper_live_replay.py").read_text(
            encoding="utf-8"
        )
        self.assertIn("release_intel_pyannote_parakeet_smoke.sh", workflow)
        self.assertIn("release_macos_whisper_live_smoke.sh", workflow)
        self.assertIn('(\"asr_reference\", smoke.get(\"asr_reference\"))', workflow)
        self.assertIn('arm_distribution["live_latency"] = arm_live', workflow)
        self.assertIn('distribution["live_latency"] = intel_live', workflow)
        self.assertIn("release_windows_whisper_live_smoke.ps1", workflow)
        self.assertIn("evaluate_asr_reference.py", intel)
        self.assertIn("SBOBINO_ASR_TIMELINE_OUTPUT", intel)
        self.assertIn("review_status", intel)
        self.assertIn("SBOBINO_WHISPER_REPLAY_WAV", live)
        self.assertIn("SBOBINO_WHISPER_LIVE_METRIC", live)
        self.assertIn("run_whisper_live_replay.py", live)
        self.assertIn("--step", live_runner)
        self.assertIn("--length", live_runner)
        self.assertIn("--save-audio", live_runner)
        self.assertNotIn("release_macos_parakeet_live_smoke.sh", workflow)
        self.assertNotIn("poliglot", intel.lower() + live.lower())
        self.assertIn('FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"', intel)
        self.assertIn('SBOBINO_WHISPER_FFMPEG="$FFMPEG_BIN"', intel)
        self.assertIn('export PATH="$SPEECH_ROOT/bin:$PATH"', intel)
        self.assertIn('optional_env("SBOBINO_WHISPER_FFMPEG")', (ROOT / "crates" / "infrastructure" / "tests" / "parakeet_real_service_smoke_tests.rs").read_text(encoding="utf-8"))
        self.assertIn('FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"', live)
        self.assertNotIn("need_cmd ffmpeg", intel)
        self.assertNotIn("command in gh ditto ffmpeg", live)

    def test_whisper_live_smokes_consume_the_shared_pinned_model_manifest(self):
        live = ARM_LIVE_SMOKE.read_text(encoding="utf-8")
        windows = WINDOWS_LIVE_SMOKE.read_text(encoding="utf-8")
        for smoke in (live, windows):
            self.assertIn("whisper_live_model.json", smoke)
            self.assertIn("immutable URL", smoke)
        self.assertIn("coreml_encoder", live)
        self.assertIn("SBOBINO_WHISPER_EXPECT_COREML", live)
        self.assertNotIn("resolve/5359861c739e955e79d9a303bcbc70fb988958b1", live)
        self.assertNotIn("60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe", live)
        self.assertNotIn("resolve/5359861c739e955e79d9a303bcbc70fb988958b1", windows)
        self.assertNotIn("60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe", windows)

    def test_windows_cpu_live_proves_fail_fast_incompatibility_without_weakening_other_platforms(self):
        windows = WINDOWS_LIVE_SMOKE.read_text(encoding="utf-8")
        release_workflow = WORKFLOW.read_text(encoding="utf-8")
        windows_workflow = (ROOT.parent / ".github" / "workflows" / "windows-port.yml").read_text(
            encoding="utf-8"
        )
        promotion = PROMOTION.read_text(encoding="utf-8")
        for workflow in (release_workflow, windows_workflow):
            self.assertIn("-ExpectPreflightRejection", workflow)
        for contract in (
            "SBOBINO_WHISPER_LIVE_PREFLIGHT",
            "--expect-preflight-rejection",
            "preflight-rejected-incompatible-cpu",
            "realtime_capable",
            "requested_duration_seconds",
            "captured_duration_seconds",
            "commit_sha",
            "repo_slug",
        ):
            self.assertIn(contract, windows)
        self.assertIn('-RuntimeZip (Join-Path $staging "speech-runtime-windows-x86_64.zip")', release_workflow)
        self.assertIn('max(float(inference_ms), float(max_ms)) <= float(budget_ms)', promotion)
        self.assertIn('float(samples) < 3', promotion)
        self.assertIn('validate_preflight_rejection(windows_live, "Windows live-latency")', promotion)
        self.assertIn('arm_profile.get("coreml_loaded") is not True', promotion)
        self.assertIn('arm_runtime_hashes.get("whisper_coreml_encoder", "")', promotion)
        self.assertIn('float(requested_duration) < 900', promotion)
        self.assertIn('float(captured_duration) != 0.0', promotion)
        self.assertIn('arm_live_mode not in {"realtime", "preflight-rejected-incompatible-cpu"}', promotion)
        self.assertIn('intel_live_mode not in {"realtime", "preflight-rejected-incompatible-cpu"}', promotion)
        self.assertIn('windows_live_mode not in {"realtime", "preflight-rejected-incompatible-cpu"}', promotion)
        self.assertIn('recovery.get("live_mode") != "backlog-recovery"', promotion)
        self.assertIn('recovery.get("backlog_recovery_expected") is not True', promotion)
        self.assertIn("TAG_COMMIT_SHA", promotion)
        self.assertIn("validate_revision", promotion)

        macos = ARM_LIVE_SMOKE.read_text(encoding="utf-8")
        self.assertIn('"live_mode": raw.get("live_mode")', macos)
        self.assertIn('"realtime_capable": raw.get("live_mode") == "realtime"', macos)
        self.assertIn('"commit_sha": sys.argv[12]', macos)
        self.assertIn('"repo_slug": sys.argv[13]', macos)

    def test_pyannote_abi_scan_includes_the_packaged_runtime_lib_directory(self):
        readiness = (ROOT / "scripts" / "distribution_readiness.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn('root / "lib" / "embedded-dylibs",\n            root / "lib",', readiness)

    def test_windows_packagers_do_not_depend_on_deleted_release_assets(self):
        for script in (WINDOWS_RUNTIME_PACKAGER, WINDOWS_PYANNOTE_PACKAGER):
            contents = script.read_text(encoding="utf-8")
            self.assertIn("SBOBINO_FFMPEG_RUNTIME_URL", contents)
            self.assertIn("SBOBINO_FFMPEG_RUNTIME_SHA256", contents)
            self.assertNotIn("pietroMastro92/Sbobino/releases/download", contents)
            self.assertNotIn("v2.0.25", contents)
            self.assertIn("BtbN/FFmpeg-Builds/releases/download/latest", contents)
            self.assertIn("ca56a31e81af11c8e4c77249039d3a61833ec37bfbc5308da91ba1b00b43a00f", contents)
            if script.name == "package_windows_pyannote_runtime.ps1":
                self.assertIn('must contain exactly one ffmpeg.exe', contents)

    def test_candidate_publish_binds_readiness_proof_to_tag_commit(self):
        publish = (ROOT / "scripts" / "publish_candidate_release.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn('git rev-parse "$TAG^{commit}"', publish)
        self.assertIn('"$TAG_COMMIT_SHA"', publish)
        self.assertIn('proof["commit_sha"].strip().lower() != tag_commit_sha', publish)

    def test_windows_pyannote_receipt_scans_embedded_python_dll_directory(self):
        contents = PROVISIONING.read_text(encoding="utf-8")
        self.assertIn(
            'roots.append(pathlib.Path(sys.executable).resolve().parent / "DLLs")',
            contents,
        )
        self.assertIn(
            '_dll_directory_handle = os.add_dll_directory(str(_dll_directory))',
            contents,
        )

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
        self.assertIn("SBOBINO_WHISPER_LIVE_DEVICE=auto", workflow)
        self.assertIn("SBOBINO_WHISPER_EXPECT_PREFLIGHT_REJECTION=1", workflow)
        self.assertNotIn("SBOBINO_WHISPER_LIVE_DEVICE=cpu", workflow)

        release_workflow = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("DEVICE=auto", release_workflow)
        self.assertNotIn("DEVICE=cpu", release_workflow)

    def test_arm_validation_uses_standard_runner_and_shared_metal_buffers(self):
        workflow = ARM_VALIDATION.read_text(encoding="utf-8")
        self.assertIn("runs-on: macos-15", workflow)
        self.assertNotIn("-large", workflow)
        self.assertNotIn("larger", workflow.lower())
        self.assertIn("release_macos_whisper_live_smoke.sh", workflow)
        self.assertIn("SBOBINO_WHISPER_LIVE_ARCH=arm64", workflow)
        self.assertIn("SBOBINO_WHISPER_ALLOW_PREFLIGHT_REJECTION=1", workflow)
        self.assertIn("arm-whisper-live-smoke-proof.json", workflow)
        self.assertNotIn("GGML_METAL_SHARED_BUFFERS_ENABLE", workflow)

    def test_intel_release_smoke_transcribes_the_long_fixture_only_once(self):
        intel = INTEL_SMOKE.read_text(encoding="utf-8")
        harness = (ROOT / "scripts" / "smoke_parakeet_real.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn("SBOBINO_PARAKEET_SMOKE_MODE=service", intel)
        self.assertIn('SMOKE_MODE=${SBOBINO_PARAKEET_SMOKE_MODE:-both}', harness)
        self.assertIn('if [[ "$SMOKE_MODE" == "service" || "$SMOKE_MODE" == "both" ]]', harness)
        self.assertIn('pwd -P)/parakeet-batch-json', harness)
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
        self.assertIn('"live_latency", "Intel live-latency", intel_distribution', promotion)
        self.assertIn('"live_latency", "Windows live-latency", windows_distribution', promotion)
        self.assertIn("expected_json_assets", promotion)
        self.assertIn("unexpected_json", promotion)
        self.assertIn('pathlib.PurePosixPath(name).suffix.lower() == ".json"', promotion)
        self.assertIn('evidence_class") != "hosted-packaged-engine"', promotion)
        self.assertIn('real_engine") is not True', promotion)
        self.assertIn("validate_live_recovery", promotion)
        self.assertIn('recovery.get("captured_audio_frames")', promotion)
        self.assertIn('recovery.get("saved_audio_frames")', promotion)
        self.assertIn('recovery.get("dropped_samples", -1)', promotion)
        self.assertIn('recovery.get("backlog_reaction_seconds")', promotion)
        self.assertIn('recovery.get("backlog_reaction_budget_seconds")', promotion)
        self.assertIn("reaction_budget != 0.25", promotion)
        self.assertIn("validate_live_duration", promotion)
        for manifest in ("latest.json", "setup-manifest.json", "runtime-manifest.json", "pyannote-manifest.json"):
            self.assertIn(manifest, promotion)
        self.assertIn("ConvertTo-Json -Depth 10", WORKFLOW.read_text(encoding="utf-8"))
        for threshold in ("0.35", "0.25", "256.0"):
            self.assertIn(threshold, promotion)
        self.assertIn('release-notes.md', promotion)


if __name__ == "__main__":
    unittest.main()
