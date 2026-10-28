import unittest
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run_fluid_speaker_replay import merge_turns


class StreamingReplayTests(unittest.TestCase):
    def test_adjacent_frames_merge_without_joining_gaps_or_overlapping_speakers(self):
        turns, last = [], {}
        for speaker, start, end in [('S1', 0, .1), ('S2', 0, .1), ('S1', .1, .2),
                                    ('S2', .2, .3), ('S1', .4, .5)]:
            merge_turns(turns, last, [dict(speaker_id=speaker, start_seconds=start, end_seconds=end)])
        self.assertEqual([(t['speaker_id'], t['start_seconds'], t['end_seconds']) for t in turns],
                         [('S1', 0, .2), ('S2', 0, .1), ('S2', .2, .3), ('S1', .4, .5)])

    def test_invalid_frame_is_rejected(self):
        for start, end in ((-1, 1), (0, 0), (0, float('nan'))):
            with self.assertRaises(ValueError):
                merge_turns([], {}, [dict(speaker_id='S1', start_seconds=start, end_seconds=end)])


if __name__ == '__main__':
    unittest.main()
