#!/usr/bin/env python3
"""Native macOS regression for launcher execution and updater signature integrity."""

import os
import pathlib
import plistlib
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = pathlib.Path(__file__).with_name("prepare_macos_bundle_scripts.py")


class MacOSBundleScriptsTests(unittest.TestCase):
    def test_signed_updater_roundtrip_preserves_shell_launcher(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            app = root / "Sbobino.app"
            executables = app / "Contents" / "MacOS"
            executables.mkdir(parents=True)
            shutil.copyfile("/usr/bin/true", executables / "main")
            (executables / "main").chmod(0o755)
            with (app / "Contents" / "Info.plist").open("wb") as handle:
                plistlib.dump({"CFBundleIdentifier": "com.sbobino.signature-test", "CFBundleExecutable": "main", "CFBundlePackageType": "APPL"}, handle)
            launcher = executables / "launcher"
            launcher.write_text('#!/bin/sh\nprintf "launcher:%s\\n" "$1"\n')
            launcher.chmod(0o755)
            for _ in range(2):
                subprocess.run(["python3", str(SCRIPT), str(app)], check=True)
            self.assertTrue(launcher.is_symlink())
            subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
            archive = root / "updater.tar.gz"
            subprocess.run(["tar", "-czf", str(archive), "-C", str(root), app.name], env=os.environ | {"COPYFILE_DISABLE": "1"}, check=True)
            extracted = root / "extracted"
            extracted.mkdir()
            subprocess.run(["tar", "-xzf", str(archive), "-C", str(extracted)], check=True)
            installed = extracted / app.name
            subprocess.run(["codesign", "--verify", "--deep", "--strict", str(installed)], check=True)
            result = subprocess.run([str(installed / "Contents/MacOS/launcher"), "ok"], text=True, capture_output=True, check=True)
            self.assertEqual(result.stdout, "launcher:ok\n")


if __name__ == "__main__":
    unittest.main()
