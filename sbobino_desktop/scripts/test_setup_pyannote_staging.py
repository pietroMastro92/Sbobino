#!/usr/bin/env python3
"""Contract tests for the Pyannote staging directory.

Regression for the 2.0.37 hosted candidates (runs 43628701223 / 43632133897):
the release workflow exported ``TMPDIR="$RUNNER_TEMP"`` but
``setup_bundled_pyannote.sh`` staged with a bare ``mktemp -d``.  On macOS (BSD
``mktemp``) that form behaves as ``mktemp -d -t tmp`` and ignores ``TMPDIR``, so
the tree landed in ``/var/folders/.../T/tmp.XXXXXXXX`` and
``sanitize_runtime`` refused it after resolving to ``/private/var/...``.

``--stage-dir-probe`` resolves, creates and removes the real staging directory
without downloading anything, so these tests exercise the shipped code path
instead of a copy of the idiom.
"""

import os
import pathlib
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "lib"))
import pyannote_portable_runtime as portable  # noqa: E402

SCRIPTS = pathlib.Path(__file__).resolve().parent
SETUP = SCRIPTS / "setup_bundled_pyannote.sh"
PERSONAL_MARKERS = ("/Library/Application Support/", "/private/")


def non_personal_staging_base():
    """First writable candidate root that the shipped guard actually accepts.

    ``tempfile.gettempdir()`` resolves to ``/private/var/folders/...`` on macOS
    and the generator refuses that lockbox alias by design, so the tests pick a
    base where the probe is allowed to stage.  ``SBOBINO_STAGING_TEST_ROOT``
    points the suite at a real build volume when one is available.
    """
    candidates = []
    override = os.environ.get("SBOBINO_STAGING_TEST_ROOT")
    if override:
        candidates.append(pathlib.Path(override))
    candidates.append(pathlib.Path(tempfile.gettempdir()))
    candidates.append(pathlib.Path.home() / ".cache" / "sbobino-portable-tests")
    for candidate in candidates:
        try:
            candidate.mkdir(parents=True, exist_ok=True)
        except OSError:
            continue
        resolved = candidate.resolve()
        try:
            portable.refuse_personal_root(resolved)
        except SystemExit:
            continue
        return resolved
    raise unittest.SkipTest("no non-personal staging root is available")


def random_letters(count=10):
    alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
    return "".join(secrets.choice(alphabet) for _ in range(count))


def run_probe(tmpdir, args=(), **environment_overrides):
    environment = dict(os.environ)
    for name, value in environment_overrides.items():
        if value is None:
            environment.pop(name, None)
        else:
            environment[name] = value
    environment["TMPDIR"] = str(tmpdir)
    completed = subprocess.run(
        ["bash", str(SETUP), "--stage-dir-probe", *args],
        capture_output=True,
        text=True,
        env=environment,
        timeout=300,
    )
    values = {}
    for line in completed.stdout.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            values[key] = value
    return completed, values


class StagingDirectoryTests(unittest.TestCase):
    def setUp(self):
        base = non_personal_staging_base()
        directory = tempfile.mkdtemp(dir=str(base), prefix="staging test ")
        self.addCleanup(shutil.rmtree, directory, True)
        self.parent = pathlib.Path(directory)

    def test_probe_honors_tmpdir_without_spaces(self):
        requested = self.parent / "plain-tmp"
        requested.mkdir()
        completed, values = run_probe(requested)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(values["stage_parent"], str(requested.resolve()))
        self.assertTrue(
            values["stage_dir"].startswith(str(requested.resolve()) + "/"),
            values,
        )

    def test_probe_honors_tmpdir_with_spaces(self):
        requested = self.parent / "tmp with spaces" / "nested"
        requested.mkdir(parents=True)
        completed, values = run_probe(requested)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(values["stage_parent"], str(requested.resolve()))
        self.assertTrue(values["stage_dir"].startswith(str(requested.resolve()) + "/"))
        self.assertIn("tmp with spaces", values["stage_dir"])

    def test_probe_does_not_fall_back_to_the_system_temporary_directory(self):
        requested = self.parent / "explicit-tmp"
        requested.mkdir()
        completed, values = run_probe(requested)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        stage_dir = pathlib.Path(values["stage_dir"])
        self.assertEqual(stage_dir.parent, requested.resolve())
        for marker in PERSONAL_MARKERS:
            self.assertNotIn(marker, str(stage_dir.resolve()))

    @unittest.skipUnless(sys.platform == "darwin", "macOS BSD mktemp behavior")
    def test_macos_mktemp_without_a_template_ignores_tmpdir(self):
        # Documents why the template is mandatory: this is the exact no-op that
        # made `export TMPDIR=$RUNNER_TEMP` useless for the 2.0.37 candidates.
        requested = self.parent / "documented-bsd-behavior"
        requested.mkdir()
        environment = dict(os.environ, TMPDIR=str(requested))
        completed = subprocess.run(
            ["mktemp", "-d"], capture_output=True, text=True, env=environment
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        created = pathlib.Path(completed.stdout.strip())
        self.addCleanup(shutil.rmtree, created, True)
        self.assertFalse(
            str(created).startswith(str(requested.resolve())),
            "BSD mktemp honored TMPDIR without a template; the regression test "
            "premise changed and the staging helper should be re-measured",
        )

    def test_probe_cleans_up_the_stage_directory(self):
        requested = self.parent / "cleanup-tmp"
        requested.mkdir()
        completed, values = run_probe(requested)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertFalse(pathlib.Path(values["stage_dir"]).exists())
        self.assertTrue(requested.is_dir())
        self.assertEqual([entry.name for entry in requested.iterdir()], [])

    def test_probe_refuses_a_personal_staging_parent(self):
        requested = pathlib.Path.home() / "Library" / "Application Support" / (
            "sbobino-staging-probe-" + random_letters()
        )
        self.addCleanup(shutil.rmtree, requested, True)
        completed, values = run_probe(requested)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("personal/lockbox", completed.stderr)
        self.assertEqual(values, {})
        self.assertFalse(requested.exists(), "the probe created a personal staging root")

    def test_probe_refuses_a_relative_tmpdir(self):
        completed, _ = run_probe("relative-tmp")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("absolute path", completed.stderr)


class ResourceRootContractTests(unittest.TestCase):
    """The assembled resource root can be redirected for local verification.

    The default stays ``apps/desktop/src-tauri/resources/pyannote`` (tracked app
    input); a local full hydration must be able to assemble and verify the real
    payload somewhere else without touching it.  The redirect is vetted with the
    same personal-path guard as the staging parent, before anything is created.
    """

    def setUp(self):
        base = non_personal_staging_base()
        directory = tempfile.mkdtemp(dir=str(base), prefix="resource root test ")
        self.addCleanup(shutil.rmtree, directory, True)
        self.parent = pathlib.Path(directory)
        self.tmpdir = self.parent / "tmp"
        self.tmpdir.mkdir()

    def test_default_resource_root_is_the_tracked_app_input(self):
        completed, values = run_probe(self.tmpdir)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            pathlib.Path(values["resource_root"]).parts[-6:],
            (
                "sbobino_desktop",
                "apps",
                "desktop",
                "src-tauri",
                "resources",
                "pyannote",
            ),
        )

    def test_explicit_resource_root_is_used_and_not_created_by_the_probe(self):
        requested = self.parent / "hydrated output with space" / "pyannote"
        completed, values = run_probe(self.tmpdir, ["--resource-root", str(requested)])
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(values["resource_root"], str(requested))
        self.assertFalse(
            requested.exists(), "the stage probe must not create the resource root"
        )

    def test_relative_resource_root_is_refused(self):
        completed, values = run_probe(self.tmpdir, ["--resource-root", "relative/root"])
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("absolute path", completed.stderr)
        self.assertNotIn("resource_root", values)

    def test_personal_resource_root_is_refused(self):
        requested = pathlib.Path.home() / "Library" / "Application Support" / (
            "sbobino-resource-root-" + random_letters()
        )
        self.addCleanup(shutil.rmtree, requested, True)
        completed, values = run_probe(self.tmpdir, ["--resource-root", str(requested)])
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("personal/lockbox", completed.stderr)
        self.assertFalse(requested.exists())

    def test_runtime_directory_removal_is_guarded(self):
        source = SETUP.read_text(encoding="utf-8")
        self.assertIn("refusing to remove runtime directory", source)
        # The destructive step must go through the guard, never straight rm -rf.
        self.assertNotIn('rm -rf "$RUNTIME_DIR"', source)
        self.assertIn('replace_runtime_dir "$RUNTIME_DIR" "$RESOURCE_ROOT"', source)


class StagingScriptContractTests(unittest.TestCase):
    def setUp(self):
        self.source = SETUP.read_text(encoding="utf-8")
        self.lines = [
            (number, line)
            for number, line in enumerate(self.source.splitlines(), start=1)
            if "mktemp" in line
            and not line.strip().startswith("#")
            and ("$(" in line or line.strip().startswith("mktemp"))
        ]

    def test_every_mktemp_uses_an_explicit_template(self):
        self.assertTrue(self.lines)
        for number, line in self.lines:
            self.assertIn(
                "XXXXXXXXXX",
                line,
                "mktemp call without an explicit template on line {}: {}".format(
                    number, line.strip()
                ),
            )

    def test_no_mktemp_dash_t_idiom(self):
        # `-t` also ignores TMPDIR on macOS; only the template form is portable.
        for number, line in self.lines:
            self.assertNotIn(" -t ", " " + line.strip() + " ")

    def test_cleanup_vets_the_stage_directory_before_removing_it(self):
        self.assertIn("refusing to remove unsafe stage directory", self.source)
        self.assertIn(
            'if [[ -z "${STAGE_PARENT:-}" || "$dir" != "$STAGE_PARENT"/tmp.* ]]',
            self.source,
        )

    def test_personal_path_guard_is_unchanged(self):
        library = (SCRIPTS / "lib" / "pyannote_portable_runtime.py").read_text(encoding="utf-8")
        for marker in ('"/Library/Application Support/"', '"/private/"', '"pairing.enc"', '"master_key_v1"'):
            self.assertIn(marker, library)


if __name__ == "__main__":
    unittest.main()
