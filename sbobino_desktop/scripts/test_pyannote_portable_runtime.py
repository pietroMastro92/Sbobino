#!/usr/bin/env python3
"""Tests for the shared Pyannote portable-runtime tooling.

Covers the otool parsers, the sanitizer, the strict audit, the path guard and
the packaging guard that refuses a stale runtime with host LC_RPATH before an
archive is written.

Mach-O fixtures are compiled with the system C compiler on macOS only; on
other platforms the pure-Python tests still run so CI on Linux exercises the
parsers and the path guard.
"""

import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "lib"))
import pyannote_portable_runtime as portable  # noqa: E402

SCRIPTS = pathlib.Path(__file__).resolve().parent
SELF = pathlib.Path(__file__).resolve()


class OtoolParsingTests(unittest.TestCase):
    def test_rpath_parser_reads_path_after_cmdsize(self):
        output = """
Load command 12
          cmd LC_RPATH
      cmdsize 32
         path /opt/homebrew/lib (offset 12)
Load command 13
          cmd LC_RPATH
      cmdsize 40
         path @executable_path/../lib (offset 12)
"""
        self.assertEqual(
            portable.parse_otool_rpaths(output),
            ["/opt/homebrew/lib", "@executable_path/../lib"],
        )

    def test_rpath_parser_ignores_cmdsize_without_path(self):
        output = """
          cmd LC_RPATH
      cmdsize 32
      cmdsize 32
"""
        self.assertEqual(portable.parse_otool_rpaths(output), [])

    def test_dependency_parser_extracts_load_entries(self):
        output = """/tmp/python3:
\t@rpath/libpython3.11.dylib (compatibility version 3.11.0, current version 3.11.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1356.0.0)
"""
        self.assertEqual(
            portable.parse_otool_dependencies(output),
            ["@rpath/libpython3.11.dylib", "/usr/lib/libSystem.B.dylib"],
        )

    def test_install_name_is_not_treated_as_a_dependency(self):
        output = """/tmp/libpython3.11.dylib:
\t/opt/homebrew/opt/python/lib/libpython3.11.dylib (compatibility version 3.11.0, current version 3.11.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1356.0.0)
"""
        with mock.patch.object(portable, "read_otool_dependencies") as dependencies, \
             mock.patch.object(portable, "read_install_name") as install_name:
            dependencies.return_value = portable.parse_otool_dependencies(output)
            install_name.return_value = "/opt/homebrew/opt/python/lib/libpython3.11.dylib"
            self.assertEqual(
                portable.read_linked_dependencies(pathlib.Path("/tmp/libpython3.11.dylib")),
                ["/usr/lib/libSystem.B.dylib"],
            )


class PathGuardTests(unittest.TestCase):
    def test_refuses_same_input_and_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary) / "runtime"
            root.mkdir()
            with self.assertRaises(SystemExit):
                portable.guard_paths(root, root)

    def test_refuses_output_nested_in_input(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary) / "runtime"
            root.mkdir()
            with self.assertRaises(SystemExit):
                portable.guard_paths(root, root / "payload")

    def test_refuses_personal_profile_paths(self):
        for candidate in (
            "/tmp/example/Library/Application Support/com.sbobino.desktop/runtime",
            "/tmp/example/private/pairing.enc",
            "/tmp/example/master_key_v1",
        ):
            with self.subTest(candidate=candidate), self.assertRaises(SystemExit):
                portable.refuse_personal_root(candidate)

    def test_keeps_ordinary_task_paths(self):
        self.assertEqual(
            portable.guard_paths("/tmp/example/runtime", "/tmp/example/payload"),
            {
                "input": os.path.realpath("/tmp/example/runtime"),
                "output": os.path.realpath("/tmp/example/payload"),
            },
        )


class InventoryTests(unittest.TestCase):
    def test_inventory_hashes_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            (root / "lib").mkdir()
            (root / "lib" / "a.dylib").write_bytes(b"payload")
            (root / "lib" / "link.dylib").symlink_to("a.dylib")
            inventory = portable.build_inventory(root)
            self.assertEqual(inventory["files"], 1)
            self.assertEqual(inventory["symlinks"], 1)
            self.assertEqual(inventory["bytes"], 7)
            kinds = {entry["path"]: entry["type"] for entry in inventory["inventory"]}
            self.assertEqual(kinds["lib/a.dylib"], "file")
            self.assertEqual(kinds["lib/link.dylib"], "symlink")


MACHO_TOOLING = sys.platform == "darwin" and bool(shutil.which("cc"))
NATIVE_ARCH = {"arm64": "arm64", "x86_64": "x86_64"}.get(os.uname().machine, os.uname().machine)


class MachOFixture:
    """Builds a minimal native dylib with the host toolchain."""

    def __init__(self, root):
        self.root = pathlib.Path(root)
        source = self.root / "fixture.c"
        source.write_text("int sbobino_fixture(void) { return 42; }\n")
        self.dylib = self.root / "libfixture.dylib"
        subprocess.run(
            [
                "cc",
                "-dynamiclib",
                "-o",
                str(self.dylib),
                str(source),
                "-Wl,-headerpad_max_install_names",
            ],
            check=True,
        )

    def install_runtime(self, name="runtime"):
        runtime = self.root / name
        (runtime / "lib").mkdir(parents=True)
        (runtime / "bin").mkdir(parents=True)
        shutil.copy2(self.dylib, runtime / "lib" / "libfixture.dylib")
        shutil.copy2(self.dylib, runtime / "bin" / "python3")
        (runtime / "bin" / "python3").chmod(0o755)
        return runtime

    def add_rpath(self, binary, rpath):
        subprocess.run(
            ["/usr/bin/install_name_tool", "-add_rpath", rpath, str(binary)],
            check=True,
            capture_output=True,
        )
        subprocess.run(
            ["/usr/bin/codesign", "--force", "--sign", "-", str(binary)],
            check=True,
            capture_output=True,
        )
        return binary


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class AuditAndSanitizeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.fixture = MachOFixture(self.temporary.name)

    def test_audit_flags_host_rpath_and_sanitize_repairs_it(self):
        runtime = self.fixture.install_runtime()
        binary = self.fixture.add_rpath(runtime / "lib" / "libfixture.dylib", "/opt/homebrew/lib")
        self.fixture.add_rpath(runtime / "bin" / "python3", "/opt/homebrew/lib")

        report = portable.audit_runtime(runtime, expect_arch=NATIVE_ARCH)
        self.assertEqual([item["kind"] for item in report["violations"]], ["host_rpath", "host_rpath"])

        receipt = portable.sanitize_runtime(runtime)
        self.assertEqual(len(receipt["modified"]), 2)
        self.assertEqual(
            sorted(entry["rpath"] for entry in receipt["removed_rpaths"]),
            ["/opt/homebrew/lib", "/opt/homebrew/lib"],
        )

        self.assertNotIn("/opt/homebrew/lib", portable.read_otool_rpaths(binary))
        ok, message = portable.codesign_verify(binary)
        self.assertTrue(ok, message)

        clean = portable.audit_runtime(runtime)
        self.assertEqual(clean["violations"], [])

    def test_sanitize_keeps_relative_loader_paths(self):
        runtime = self.fixture.install_runtime()
        binary = runtime / "bin" / "python3"
        self.fixture.add_rpath(binary, "@executable_path/../lib")
        self.fixture.add_rpath(binary, "/opt/homebrew/lib")

        portable.sanitize_runtime(runtime)

        self.assertEqual(portable.read_otool_rpaths(binary), ["@executable_path/../lib"])

    def test_audit_rejects_absolute_build_rpath_and_sanitize_removes_it(self):
        runtime = self.fixture.install_runtime()
        binary = runtime / "lib" / "libfixture.dylib"
        self.fixture.add_rpath(binary, "/Users/runner/miniconda3/envs/build/lib")

        report = portable.audit_runtime(runtime)
        self.assertIn("absolute_rpath", {item["kind"] for item in report["violations"]})

        receipt = portable.sanitize_runtime(runtime)

        self.assertEqual(
            [(entry["rpath"], entry["reason"]) for entry in receipt["removed_rpaths"]],
            [("/Users/runner/miniconda3/envs/build/lib", "absolute_build_path")],
        )
        self.assertEqual(portable.read_otool_rpaths(binary), [])
        self.assertEqual(portable.audit_runtime(runtime)["violations"], [])

    def test_audit_flags_arch_mismatch(self):
        runtime = self.fixture.install_runtime()
        self.fixture.add_rpath(runtime / "lib" / "libfixture.dylib", "@loader_path")
        other = "x86_64" if os.uname().machine == "arm64" else "arm64"
        report = portable.audit_runtime(runtime, expect_arch=other)
        self.assertIn("arch_mismatch", {item["kind"] for item in report["violations"]})

    def test_sanitize_is_idempotent(self):
        runtime = self.fixture.install_runtime()
        self.fixture.add_rpath(runtime / "lib" / "libfixture.dylib", "/opt/homebrew/lib")
        portable.sanitize_runtime(runtime)
        second = portable.sanitize_runtime(runtime)
        self.assertEqual(second["modified"], [])
        self.assertEqual(second["removed_rpaths"], [])


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class PackagingGuardTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.fixture = MachOFixture(self.temporary.name)

    def _package(self, source, output):
        return subprocess.run(
            [str(SCRIPTS / "package_pyannote_asset.sh"), str(source), "python", str(output)],
            capture_output=True,
            text=True,
        )

    def test_guard_rejects_stale_runtime_and_accepts_repaired_one(self):
        runtime = self.fixture.install_runtime()
        self.fixture.add_rpath(runtime / "lib" / "libfixture.dylib", "/opt/homebrew/lib")
        stale_zip = pathlib.Path(self.temporary.name) / "stale.zip"

        rejected = self._package(runtime, stale_zip)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("host_rpath", rejected.stdout + rejected.stderr)
        self.assertFalse(stale_zip.exists())

        portable.sanitize_runtime(runtime)
        accepted = self._package(runtime, stale_zip)
        self.assertEqual(accepted.returncode, 0, accepted.stdout + accepted.stderr)
        self.assertTrue(stale_zip.exists())


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class RepairScriptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.fixture = MachOFixture(self.temporary.name)

    def _run(self, source_runtime, model, output, evidence):
        return subprocess.run(
            [
                str(SCRIPTS / "sanitize_pyannote_portable_runtime.sh"),
                "--runtime",
                str(source_runtime),
                "--model",
                str(model),
                "--output",
                str(output),
                "--evidence",
                str(evidence),
            ],
            capture_output=True,
            text=True,
            env={
                **os.environ,
                "TMPDIR": str(pathlib.Path(self.temporary.name) / "tmp"),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin",
            },
        )

    def test_repair_produces_green_payload_and_leaves_source_red(self):
        root = pathlib.Path(self.temporary.name)
        (root / "tmp").mkdir()
        runtime = self.fixture.install_runtime("source-runtime")
        self.fixture.add_rpath(runtime / "bin" / "python3", "/opt/homebrew/lib")
        model = root / "model"
        model.mkdir()
        (model / "config.yaml").write_text("pipeline:\n  name: fixture\n")
        output = root / "payload" / "pyannote"
        evidence = root / "evidence"

        completed = self._run(runtime, model, output, evidence)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

        payload = output / "python"
        triples = list(payload.iterdir())
        self.assertEqual(len(triples), 1)
        staged = triples[0]
        self.assertTrue((staged / "bin" / "python3").exists())
        self.assertTrue((output / "model" / "config.yaml").exists())

        report = portable.audit_runtime(staged)
        self.assertEqual(report["violations"], [])
        source_report = portable.audit_runtime(runtime)
        self.assertIn("host_rpath", {item["kind"] for item in source_report["violations"]})

        self.assertTrue((evidence / "repair.json").exists())
        self.assertTrue((evidence / "inventory-payload.json").exists())
        self.assertTrue((evidence / "post-audit-payload.json").exists())

    def test_refuses_existing_non_empty_output(self):
        root = pathlib.Path(self.temporary.name)
        (root / "tmp").mkdir()
        runtime = self.fixture.install_runtime("source-runtime")
        model = root / "model"
        model.mkdir()
        (model / "config.yaml").write_text("pipeline:\n  name: fixture\n")
        output = root / "payload" / "pyannote"
        output.mkdir(parents=True)
        (output / "stale-file").write_text("keep me")
        evidence = root / "evidence"

        completed = self._run(runtime, model, output, evidence)
        self.assertNotEqual(completed.returncode, 0)
        self.assertEqual((output / "stale-file").read_text(), "keep me")


if __name__ == "__main__":
    unittest.main()
