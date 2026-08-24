#!/usr/bin/env python3

import json
import os
import pathlib
import stat
import subprocess
import sys
import tempfile
import unittest
import wave

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from run_whisper_live_replay import (
    backlog_threshold_overshoot,
    final_runtime_summary,
    finalized_transcript,
    first_voiced_frame,
    live_command_profile,
)


class FinalizedTranscriptTests(unittest.TestCase):
    def test_live_command_profile_matches_cpu_and_auto_runtime_windows(self):
        self.assertEqual(live_command_profile("cpu", 4), (4, 1920))
        self.assertEqual(live_command_profile("auto", 12), (8, 3200))
        self.assertEqual(live_command_profile("cpu", None), (1, 1920))

    def test_first_voiced_frame_excludes_leading_silence_from_preview_latency(self):
        samples = [0] * 640 + [2000] * 320
        self.assertEqual(first_voiced_frame(samples, 16000), 640)

    def test_preview_redraws_are_not_counted_as_final_duplicates(self):
        stdout = (
            "[Start speaking]\n"
            "\x1b[2K old por\r"
            "\x1b[2K old portrait\r"
            "\x1b[2K old portrait.\n"
        )
        self.assertEqual(finalized_transcript(stdout), "old portrait.")

    def test_distinct_final_lines_are_preserved(self):
        stdout = "first final\nsecond final\nsecond final\n"
        self.assertEqual(finalized_transcript(stdout), "first final\nsecond final")

    def test_final_runtime_summary_uses_authoritative_dropped_counter(self):
        stderr = (
            "SBOBINO_WHISPER_LIVE_METRIC captured_seconds=2.000 "
            "processed_seconds=0.320 backlog_seconds=1.680 inference_ms=50 "
            "dropped_samples=0\n"
            "SBOBINO_WHISPER_LIVE_METRICS captured_seconds=2.010 "
            "processed_seconds=0.320 backlog_seconds=1.690 dropped_samples=17\n"
        )
        self.assertEqual(final_runtime_summary(stderr, 16000), (32160, 17))

    def test_final_runtime_summary_requires_terminal_summary(self):
        self.assertIsNone(final_runtime_summary("SBOBINO_WHISPER_LIVE_METRIC", 16000))

    def test_backlog_reaction_is_measured_from_threshold_crossing(self):
        stderr = (
            "SBOBINO_WHISPER_LIVE_BACKLOG exceeded captured=32240 "
            "inferred=160 buffered=32080 dropped=0\n"
        )
        self.assertAlmostEqual(backlog_threshold_overshoot(stderr, 16000), 0.005)

    def test_missing_terminal_summary_makes_end_to_end_report_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run_dir = root / "run"
            run_dir.mkdir()
            audio = root / "audio.wav"
            fixture = root / "fixture.wav"
            for path in (audio, fixture):
                with wave.open(str(path), "wb") as handle:
                    handle.setnchannels(1)
                    handle.setsampwidth(2)
                    handle.setframerate(16000)
                    handle.writeframes(b"\x00\x00" * 320)
            model = root / "model.bin"
            model.write_bytes(b"fake")
            report = root / "report.json"

            if os.name == "nt":
                binary = root / "fake-whisper.cmd"
                binary.write_text(
                    "@echo off\r\n"
                    "copy /Y \"%SBOBINO_WHISPER_REPLAY_WAV%\" captured.wav >NUL\r\n"
                    "echo [Start speaking]\r\n"
                    "echo old portrait\r\n"
                    "echo SBOBINO_WHISPER_LIVE_METRIC captured_seconds=0.020 processed_seconds=0.020 backlog_seconds=0.000 inference_ms=1.000 dropped_samples=0 1>&2\r\n",
                    encoding="utf-8",
                )
            else:
                binary = root / "fake-whisper"
                binary.write_text(
                    "#!/bin/sh\n"
                    "cp \"$SBOBINO_WHISPER_REPLAY_WAV\" captured.wav\n"
                    "printf '[Start speaking]\\nold portrait\\n'\n"
                    "printf 'SBOBINO_WHISPER_LIVE_METRIC captured_seconds=0.020 processed_seconds=0.020 backlog_seconds=0.000 inference_ms=1.000 dropped_samples=0\\n' 1>&2\n",
                    encoding="utf-8",
                )
                binary.chmod(binary.stat().st_mode | stat.S_IXUSR)

            completed = subprocess.run(
                [
                    sys.executable,
                    str(pathlib.Path(__file__).with_name("run_whisper_live_replay.py")),
                    "--binary",
                    str(binary),
                    "--model",
                    str(model),
                    "--audio",
                    str(audio),
                    "--fixture",
                    str(fixture),
                    "--report",
                    str(report),
                    "--run-dir",
                    str(run_dir),
                    "--device",
                    "cpu",
                    "--platform",
                    "test",
                ],
                check=False,
            )
            payload = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(completed.returncode, 1)
            self.assertEqual(payload["status"], "failed")
            self.assertEqual(payload["dropped_samples"], -1)
            self.assertIn("terminal runtime summary is missing", payload["failures"])


if __name__ == "__main__":
    unittest.main()
