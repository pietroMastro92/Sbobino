import hashlib
import io
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from prepare_fluid_diarizer_models import install, SDK_REVISION, REQUIRED_PATHS
from smoke_fluid_diarizer_native import validate_output


class ModelInstallationTests(unittest.TestCase):
    def test_native_smoke_rejects_wrong_backend_empty_result_and_invalid_timestamps(self):
        valid = dict(version=1, backend='fluid_offline', speakers=[
            dict(speaker_id='S1', start_seconds=0, end_seconds=1),
            dict(speaker_id='S2', start_seconds=1, end_seconds=2),
        ])
        validate_output(valid, 2)
        for output in (dict(valid, backend='pyannote'), dict(valid, speakers=[])):
            with self.assertRaises(ValueError):
                validate_output(output, 2)
        valid['speakers'][1]['end_seconds'] = 2.1
        with self.assertRaises(ValueError):
            validate_output(valid, 2)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = dict(repository='FluidInference/speaker-diarization-coreml', revision='a'*40,
                             fluid_audio_revision=SDK_REVISION,
                             installed_repository_folder='speaker-diarization',
                             files=[dict(path=path, bytes=3, sha256=hashlib.sha256(b'abc').hexdigest())
                                    for path in sorted(REQUIRED_PATHS)])

    def populate_cache(self):
        destination = self.root/'speaker-diarization'
        for entry in self.manifest['files']:
            path = destination/entry['path']
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'abc')
        return destination

    def test_install_and_existing_cache_require_no_second_download(self):
        with patch('urllib.request.urlopen', side_effect=lambda *a, **k: io.BytesIO(b'abc')) as download:
            destination = install(self.root, self.manifest)
            self.assertEqual(download.call_count, len(REQUIRED_PATHS))
        self.assertEqual((destination/'.fluidaudio-revision').read_text(), 'a'*40+'\n')
        with patch('urllib.request.urlopen', side_effect=AssertionError('unexpected network')):
            self.assertEqual(install(self.root, self.manifest), destination)
            (destination/'.fluidaudio-revision').unlink()
            install(self.root, self.manifest)
        self.assertEqual((destination/'.fluidaudio-revision').read_text(), 'a'*40+'\n')

    def test_bad_download_never_publishes_partial_models(self):
        for content in (b'ab', b'abcd', b'xxx'):
            with patch('urllib.request.urlopen', return_value=io.BytesIO(content)):
                with self.assertRaises(ValueError):
                    install(self.root, self.manifest)
            self.assertEqual(list(self.root.iterdir()), [])

    def test_corrupt_existing_models_are_preserved_without_revision_marker(self):
        destination = self.populate_cache()
        corrupted = destination/'Embedding.mlmodelc/coremldata.bin'
        corrupted.write_bytes(b'xxx')
        with self.assertRaises(ValueError):
            install(self.root, self.manifest)
        self.assertEqual(corrupted.read_bytes(), b'xxx')
        self.assertFalse((destination/'.fluidaudio-revision').exists())

    def test_wrong_revision_and_unsafe_paths_are_rejected(self):
        with patch('urllib.request.urlopen', side_effect=lambda *a, **k: io.BytesIO(b'abc')):
            destination = install(self.root, self.manifest)
        marker = destination/'.fluidaudio-revision'
        marker.write_text('b'*40)
        with self.assertRaises(ValueError):
            install(self.root, self.manifest)
        self.assertEqual(marker.read_text(), 'b'*40)
        self.manifest['files'][0]['path'] = '../escape'
        with self.assertRaises(ValueError):
            install(self.root, self.manifest)
        self.assertFalse((self.root.parent/'escape').exists())

    def test_missing_artifacts_sdk_mismatch_and_partial_bundle_never_get_adopted(self):
        destination = self.populate_cache()
        partial = destination/'Embedding.mlmodelc/weight.bin.partial'
        partial.write_bytes(b'interrupted')
        with self.assertRaises(ValueError):
            install(self.root, self.manifest)
        self.assertTrue(partial.exists())
        self.assertFalse((destination/'.fluidaudio-revision').exists())
        partial.unlink()
        for manifest in (dict(self.manifest, fluid_audio_revision='wrong'),
                         dict(self.manifest, files=self.manifest['files'][:-1])):
            with self.assertRaises(ValueError):
                install(self.root, manifest)
        self.assertFalse((destination/'.fluidaudio-revision').exists())


if __name__ == '__main__':
    unittest.main()
