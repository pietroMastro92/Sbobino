import tempfile
import unittest
from pathlib import Path
import wave
from benchmark_transcription_service import descendants, repeat_audio, process_memory, run
import json
import os
import sys

class BenchmarkContracts(unittest.TestCase):
    def test_descendants_excludes_unrelated_and_includes_nested_child(self):
        rows=[(8,7,100),(9,8,200),(7,1,300),(20,1,900),(21,20,1000)]
        self.assertEqual(descendants(rows,7),{7,8,9})

    def test_repeat_preserves_all_pcm_frames(self):
        with tempfile.TemporaryDirectory() as directory:
            source,target=Path(directory)/'in.wav',Path(directory)/'out.wav'
            with wave.open(str(source),'wb') as audio:
                audio.setparams((1,2,16000,0,'NONE','not compressed'))
                audio.writeframes(b'\x10\x00'*1600)
            self.assertAlmostEqual(repeat_audio(source,target,12),1.2)
            with wave.open(str(target),'rb') as audio:
                self.assertEqual(audio.getnframes(),19200)
                self.assertEqual(audio.readframes(19200),b'\x10\x00'*19200)

    def test_timeout_preserves_measurement_and_log(self):
        with tempfile.TemporaryDirectory() as directory:
            script,log=Path(directory)/'sleep.py',Path(directory)/'run.log'
            script.write_text('import time\ntime.sleep(10)\n')
            with self.assertRaises(TimeoutError):
                run(Path(sys.executable),str(script),log,os.environ.copy(),0.15)
            record=json.loads(log.with_suffix('.measurement.json').read_text())
            self.assertEqual(record['status'],'failed')
            self.assertIn('exceeded',record['error'])
            self.assertTrue(log.exists())

    def test_failed_process_is_not_reported_passed(self):
        with tempfile.TemporaryDirectory() as directory:
            script,log=Path(directory)/'fail.py',Path(directory)/'run.log'
            script.write_text('raise SystemExit(7)\n')
            with self.assertRaises(AssertionError):
                run(Path(sys.executable),str(script),log,os.environ.copy(),5)
            record=json.loads(log.with_suffix('.measurement.json').read_text())
            self.assertEqual(record['exit_code'],7)
            self.assertEqual(record['status'],'failed')

    def test_native_snapshot_contains_current_process(self):
        import os
        rows=process_memory()
        self.assertTrue(any(pid==os.getpid() and rss is not None and rss>0 for pid,_,rss in rows))

if __name__=='__main__': unittest.main()
