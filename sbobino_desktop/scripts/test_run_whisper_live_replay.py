#!/usr/bin/env python3

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from run_whisper_live_replay import (
    backlog_threshold_overshoot,
    final_runtime_summary,
    finalized_transcript,
)


class FinalizedTranscriptTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
