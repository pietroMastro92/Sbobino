import hashlib
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from package_fluid_diarizer import stage_bundle, verify_bundle, seal_app, inventory


class NativeBundleTests(unittest.TestCase):
    def test_signing_failures_preserve_the_entire_original_app(self):
        for fail_at in [1, 2, 3]:
            with self.subTest(fail_at=fail_at), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                app, stage = root/'Original.app', root/'stage'
                executable = app/'Contents/MacOS/main'
                executable.parent.mkdir(parents=True)
                executable.write_bytes(b'original executable and signature')
                signature = app/'Contents/_CodeSignature/CodeResources'
                signature.parent.mkdir()
                signature.write_bytes(b'original resource seal')
                stage.mkdir()
                (stage/'helper').write_bytes(b'new runtime')
                (stage/'manifest.json').write_text(json.dumps(dict(version=1, files=inventory(stage))))
                before = {p.relative_to(app): p.read_bytes() for p in app.rglob('*') if p.is_file()}
                calls = 0

                def sign(command, **kwargs):
                    nonlocal calls
                    calls += 1
                    candidate = Path(command[-1])
                    (candidate/'Contents/MacOS/main').write_bytes(b'mutated candidate signature')
                    if calls == fail_at:
                        raise subprocess.CalledProcessError(1, command)

                with patch('subprocess.run', side_effect=sign), self.assertRaises(subprocess.CalledProcessError):
                    seal_app(app, stage, '-', root)
                self.assertEqual(before, {p.relative_to(app): p.read_bytes() for p in app.rglob('*') if p.is_file()})

    def test_manifest_hashes_final_signed_helper_and_detects_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary, models, target = root/'helper', root/'models', root/'bundle'
            binary.write_bytes(b'unsigned fixture')
            license_file = root/'SDK-LICENSE'
            license_file.write_text('SDK license fixture')
            (root/'ThirdPartyLicenses').mkdir()
            (root/'ThirdPartyLicenses/NOTICE').write_text('third party attribution fixture')
            models.mkdir()
            (models/'weights').write_bytes(b'model fixture')
            (models/'.fluidaudio-revision').write_text('model-revision\n')
            manifest = dict(fluid_audio_revision='sdk-revision', revision='model-revision',
                            files=[dict(path='weights')])

            def sign(command, **kwargs):
                if command[0] != 'codesign':
                    return
                helper = Path(command[-1])
                helper.write_bytes(helper.read_bytes()+b' signed fixture')

            with patch('subprocess.run', side_effect=sign):
                stage_bundle(binary, models, manifest, target, '-', license_file)
            result = verify_bundle(target)
            helper_entry = next(f for f in result['files'] if f['path'] == 'bin/sbobino-diarizer')
            self.assertEqual(helper_entry['sha256'], hashlib.sha256(b'unsigned fixture signed fixture').hexdigest())
            (target/'unexpected-file').write_bytes(b'preserve me')
            with self.assertRaises(ValueError):
                verify_bundle(target)
            self.assertEqual((target/'unexpected-file').read_bytes(), b'preserve me')
            (target/'unexpected-file').unlink()
            (target/'models/speaker-diarization/weights').write_bytes(b'corrupt')
            with self.assertRaises(ValueError):
                verify_bundle(target)


if __name__ == '__main__':
    unittest.main()
