#!/usr/bin/env python3
"""Tests for the shared Pyannote portable-runtime tooling.

Covers the otool parsers, the sanitizer, the strict audit, the path guard and
the packaging guard that refuses a stale runtime with host LC_RPATH before an
archive is written.

Mach-O fixtures are compiled with the system C compiler on macOS only; on
other platforms the pure-Python tests still run so CI on Linux exercises the
parsers and the path guard.
"""

import ctypes
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
             mock.patch.object(portable, "read_install_names") as install_names:
            dependencies.return_value = portable.parse_otool_dependencies(output)
            install_names.return_value = ["/opt/homebrew/opt/python/lib/libpython3.11.dylib"]
            self.assertEqual(
                portable.read_linked_dependencies(pathlib.Path("/tmp/libpython3.11.dylib")),
                ["/usr/lib/libSystem.B.dylib"],
            )

    def test_dependency_path_with_spaces_is_not_truncated(self):
        output = """\
/payload/bin/python3:
\t/Volumes/My Disk/build/payload/lib/libpython3.11.dylib (compatibility version 3.11.0, current version 3.11.0)
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1356.0.0)
"""
        self.assertEqual(
            portable.parse_otool_dependencies(output),
            [
                "/Volumes/My Disk/build/payload/lib/libpython3.11.dylib",
                "/usr/lib/libSystem.B.dylib",
            ],
        )

    def test_universal_binary_architecture_headers_are_not_dependencies(self):
        # Real `otool -L` output for a universal Mach-O: the per-slice header
        # repeats the absolute path of the *file*.  The pre-November-2026
        # parser skipped only the first line, so it reported that header as an
        # absolute dependency equal to the file itself.  Measured on the
        # 2.0.36 payload: 11 of 449 Mach-O files are universal and produced 11
        # phantom dependencies that were then wrongly blamed on the build root.
        output = """\
/payload/lib/md.cpython-311-darwin.so (architecture x86_64):
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1345.120.2)
/payload/lib/md.cpython-311-darwin.so (architecture arm64):
\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1345.120.2)
"""
        self.assertEqual(
            portable.parse_otool_dependencies(output),
            ["/usr/lib/libSystem.B.dylib"],
        )

    def test_install_name_parser_skips_universal_arch_headers(self):
        with mock.patch.object(portable, "run_command") as run:
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="""\
/payload/lib/leaf.so (architecture x86_64):
bazel-out/osx-x86_64/bin/leaf.so
/payload/lib/leaf.so (architecture arm64):
bazel-out/osx-aarch_64/bin/leaf.so
""",
            )
            self.assertEqual(
                portable.read_install_names(pathlib.Path("/payload/lib/leaf.so")),
                ["bazel-out/osx-x86_64/bin/leaf.so", "bazel-out/osx-aarch_64/bin/leaf.so"],
            )
            self.assertEqual(
                portable.read_install_name(pathlib.Path("/payload/lib/leaf.so")),
                "bazel-out/osx-x86_64/bin/leaf.so",
            )

    def test_install_name_without_id_dylib_is_none(self):
        with mock.patch.object(portable, "run_command") as run:
            run.return_value = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="/payload/lib/md.cpython-311-darwin.so (architecture arm64):\n",
            )
            self.assertEqual(
                portable.read_install_names(pathlib.Path("/payload/lib/md.so")), []
            )
            self.assertIsNone(portable.read_install_name(pathlib.Path("/payload/lib/md.so")))


class ReferenceClassificationTests(unittest.TestCase):
    """The audit must anchor detection on the payload layout, not on prefixes."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sbobino classify ")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name) / "payload"
        for relative in ("lib/libinner.dylib", "nested/lib/libinner.dylib"):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b"fixture")
        self.index = portable.payload_file_index(self.root)

    def test_relative_reference_is_not_absolute(self):
        self.assertEqual(
            portable.classify_absolute_reference("@rpath/libinner.dylib", self.root, self.index),
            {"kind": "not_absolute"},
        )

    def test_reference_inside_the_root_is_inside(self):
        classified = portable.classify_absolute_reference(
            str(self.root / "lib" / "libinner.dylib"), self.root, self.index
        )
        self.assertEqual(classified["kind"], "inside")
        self.assertEqual(classified["relative"], "lib/libinner.dylib")

    def test_foreign_root_with_matching_layout_is_a_suffix_match(self):
        classified = portable.classify_absolute_reference(
            "/some/other/build/payload/lib/libinner.dylib", self.root, self.index
        )
        self.assertEqual(classified["kind"], "suffix")
        self.assertEqual(classified["relative"], "lib/libinner.dylib")

    def test_two_matching_layouts_are_ambiguous(self):
        # Two different payload paths are a tail of the same reference: never
        # pick one of them.
        classified = portable.classify_absolute_reference(
            "/somewhere/nested/lib/libinner.dylib", self.root, self.index
        )
        self.assertEqual(classified["kind"], "ambiguous")
        self.assertEqual(
            classified["candidates"],
            ["lib/libinner.dylib", "nested/lib/libinner.dylib"],
        )

    def test_bare_basename_never_matches_the_payload(self):
        # A single-component tail is not enough evidence to claim a payload
        # file: that would be a basename guess.
        classified = portable.classify_absolute_reference(
            "/opt/llvm-openmp/libinner.dylib", self.root, self.index
        )
        self.assertEqual(classified["kind"], "external")

    def test_dead_absolute_path_is_external(self):
        self.assertEqual(
            portable.classify_absolute_reference(
                "/DLC/pillow/.dylibs/libpng16.16.dylib", self.root, self.index
            ),
            {"kind": "external"},
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


def non_personal_test_directory(prefix="sbobino portable "):
    """Create a temporary directory the sanitizer is allowed to operate on.

    ``tempfile.gettempdir()`` resolves to ``/private/var/folders/...`` on macOS,
    and ``refuse_personal_root`` rejects that lockbox alias on purpose.  The
    tests therefore pick the first candidate root that the *shipped* guard
    accepts, so the suite stays honest on macOS instead of failing on a path it
    was never allowed to touch.  ``SBOBINO_RELOCATION_TEST_ROOT`` lets the
    release review point the tree at a real build volume.
    """
    candidates = []
    override = os.environ.get("SBOBINO_RELOCATION_TEST_ROOT")
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
        return tempfile.mkdtemp(dir=str(resolved), prefix=prefix)
    raise unittest.SkipTest("no non-personal test directory is available")


class SanitizerTestDirectoryMixin:
    def make_test_directory(self):
        directory = pathlib.Path(non_personal_test_directory())
        self.addCleanup(shutil.rmtree, directory, True)
        return directory


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
                "-arch", NATIVE_ARCH,
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
class AuditAndSanitizeTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    def setUp(self):
        self.directory = self.make_test_directory()
        self.fixture = MachOFixture(self.directory)

    def test_audit_flags_host_rpath_and_sanitize_repairs_it(self):
        runtime = self.fixture.install_runtime()
        binary = self.fixture.add_rpath(runtime / "lib" / "libfixture.dylib", "/opt/homebrew/lib")
        self.fixture.add_rpath(runtime / "bin" / "python3", "/opt/homebrew/lib")

        report = portable.audit_runtime(runtime, expect_arch=NATIVE_ARCH)
        # The fixture is linked with the host output path as LC_ID_DYLIB, which
        # is the same defect class as a wheel built in a scratch directory: an
        # absolute install name that cannot resolve on a user machine.
        # `personal_path_reference` is expected as well whenever the test root
        # itself lives under a marker path (for example /Users), so it is not
        # part of the equality: it is reported independently of the fix.
        kinds = [item["kind"] for item in report["violations"]]
        self.assertEqual(kinds.count("host_rpath"), 2, kinds)
        self.assertEqual(kinds.count("non_portable_install_name"), 2, kinds)

        receipt = portable.sanitize_runtime(runtime)
        self.assertEqual(len(receipt["modified"]), 2)
        self.assertEqual(len(receipt["repaired_install_names"]), 2)
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

    def test_system_install_name_is_preserved_and_not_flagged(self):
        runtime = self.fixture.install_runtime()
        binaries = [runtime / "lib" / "libfixture.dylib", runtime / "bin" / "python3"]
        for binary in binaries:
            portable.run_command(
                [portable.INSTALL_NAME_TOOL, "-id", "/usr/lib/libfixture.dylib", binary]
            )
            portable.run_command([portable.CODESIGN, "--force", "--sign", "-", binary])

        report = portable.audit_runtime(runtime)
        self.assertEqual(report["violations"], [])
        receipt = portable.sanitize_runtime(runtime)
        self.assertEqual(receipt["repaired_install_names"], [])
        for binary in binaries:
            self.assertEqual(portable.read_install_names(binary), ["/usr/lib/libfixture.dylib"])


class RelocatablePayloadFixture:
    """Builds a payload whose load commands only resolve on the build machine.

    Two dylibs and a client executable are compiled with ``-install_name`` and
    linked through absolute paths, which is what a wheel build records.  The
    fixture is small enough to be built in a test and real enough that the
    dynamic loader, not a string comparison, decides whether the sanitizer
    produced a portable payload.
    """

    def __init__(self, root):
        self.root = pathlib.Path(root)
        self.src = self.root / "src"
        self.src.mkdir(parents=True, exist_ok=True)
        (self.src / "inner.c").write_text("int sbobino_inner(void) { return 7; }\n")
        (self.src / "outer.c").write_text(
            "int sbobino_inner(void);\n"
            "int sbobino_outer(void) { return sbobino_inner() + 1; }\n"
        )
        (self.src / "client.c").write_text(
            "#include <stdio.h>\n"
            "int sbobino_outer(void);\n"
            'int main(void) { printf("%d\\n", sbobino_outer()); return 0; }\n'
        )

    def install_name_tool(self, *args):
        subprocess.run(
            ["/usr/bin/install_name_tool", *[str(arg) for arg in args]],
            check=True,
            capture_output=True,
        )

    def sign(self, binary):
        subprocess.run(
            ["/usr/bin/codesign", "--force", "--sign", "-", str(binary)],
            check=True,
            capture_output=True,
        )

    def set_install_name(self, binary, value):
        self.install_name_tool("-id", value, binary)
        self.sign(binary)

    def build(self, payload_name="payload"):
        payload = self.root / payload_name
        (payload / "lib").mkdir(parents=True, exist_ok=True)
        (payload / "bin").mkdir(parents=True, exist_ok=True)
        inner = payload / "lib" / "libsbobino_inner.dylib"
        outer = payload / "lib" / "libsbobino_outer.dylib"
        client = payload / "bin" / "sbobino-client"
        common = ["-arch", NATIVE_ARCH, "-Wl,-headerpad_max_install_names"]
        subprocess.run(
            ["cc", "-dynamiclib", *common, "-install_name", str(inner),
             "-o", str(inner), str(self.src / "inner.c")],
            check=True,
            capture_output=True,
        )
        subprocess.run(
            ["cc", "-dynamiclib", *common, "-install_name", str(outer),
             "-o", str(outer), str(self.src / "outer.c"), str(inner)],
            check=True,
            capture_output=True,
        )
        subprocess.run(
            ["cc", *common, "-o", str(client), str(self.src / "client.c"), str(outer)],
            check=True,
            capture_output=True,
        )
        return payload


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class RelocatablePayloadTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    def setUp(self):
        # A path with a space is deliberate: the payload must survive both a
        # relocation and a staging parent that contains spaces.
        self.directory = self.make_test_directory()
        self.fixture = RelocatablePayloadFixture(self.directory)

    def test_audit_reports_internal_absolute_references_at_the_build_root(self):
        payload = self.fixture.build()
        report = portable.audit_runtime(payload)
        internal = [
            item for item in report["violations"]
            if item["kind"] == "absolute_runtime_reference"
        ]
        self.assertEqual(len(internal), 4, report["violations"])
        self.assertTrue(all("resolves inside the payload" in item["detail"] for item in internal))

    def test_audit_detects_a_build_root_reference_from_another_root(self):
        payload = self.fixture.build()
        relocated = self.directory / "audited elsewhere" / "payload"
        relocated.parent.mkdir()
        shutil.copytree(payload, relocated)

        report = portable.audit_runtime(relocated)
        details = [
            item["detail"] for item in report["violations"]
            if item["kind"] == "absolute_runtime_reference"
        ]
        self.assertEqual(len(details), 4, report["violations"])
        self.assertTrue(all("foreign root for payload path" in detail for detail in details))

    def test_sanitize_relocates_the_payload_and_the_loader_accepts_it(self):
        payload = self.fixture.build()
        inner = payload / "lib" / "libsbobino_inner.dylib"
        outer = payload / "lib" / "libsbobino_outer.dylib"
        client = payload / "bin" / "sbobino-client"

        receipt = portable.sanitize_runtime(payload)
        self.assertEqual(len(receipt["repaired_install_names"]), 2)
        self.assertEqual(len(receipt["repaired_dependencies"]), 2)
        self.assertEqual(
            {entry["replacement"] for entry in receipt["repaired_install_names"]},
            {"@rpath/libsbobino_inner.dylib", "@rpath/libsbobino_outer.dylib"},
        )
        self.assertEqual(
            {entry["replacement"] for entry in receipt["repaired_dependencies"]},
            {"@loader_path/libsbobino_inner.dylib", "@loader_path/../lib/libsbobino_outer.dylib"},
        )

        self.assertEqual(portable.read_install_names(inner), ["@rpath/libsbobino_inner.dylib"])
        self.assertIn(
            "@loader_path/libsbobino_inner.dylib", portable.read_linked_dependencies(outer)
        )
        self.assertEqual(portable.audit_runtime(payload)["violations"], [])
        for binary in (inner, outer, client):
            ok, message = portable.codesign_verify(binary)
            self.assertTrue(ok, message)

        # The proof that matters: move the whole tree somewhere else and let the
        # real dynamic loader resolve the rewritten load commands.
        moved = self.directory / "moved payload with space" / "payload"
        moved.parent.mkdir()
        shutil.move(str(payload), str(moved))

        completed = subprocess.run([str(moved / "bin" / "sbobino-client")],
                                   capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout.strip(), "8")

        library = ctypes.CDLL(str(moved / "lib" / "libsbobino_outer.dylib"))
        self.assertEqual(library.sbobino_outer(), 8)
        self.assertEqual(portable.audit_runtime(moved)["violations"], [])

    def test_second_sanitize_is_a_byte_level_no_op(self):
        payload = self.fixture.build()
        portable.sanitize_runtime(payload)
        snapshot = {
            path.relative_to(payload).as_posix(): path.read_bytes()
            for path in sorted(payload.rglob("*"))
            if path.is_file()
        }
        second = portable.sanitize_runtime(payload)
        self.assertEqual(second["modified"], [])
        self.assertEqual(second["repaired_install_names"], [])
        self.assertEqual(second["repaired_dependencies"], [])
        self.assertEqual(second["removed_rpaths"], [])
        after = {
            path.relative_to(payload).as_posix(): path.read_bytes()
            for path in sorted(payload.rglob("*"))
            if path.is_file()
        }
        self.assertTrue(snapshot)
        self.assertEqual(snapshot, after)

    def test_sanitize_repairs_a_dead_external_install_name(self):
        # Mirrors the shipped wheels: Pillow/scipy ship ``<payload>/PIL/.dylibs/
        # libX.dylib`` whose LC_ID_DYLIB is ``/DLC/PIL/.dylibs/libX.dylib`` and
        # whose clients record exactly that delocated path.
        payload = self.fixture.build()
        vendor = payload / "vendor" / ".dylibs"
        vendor.mkdir(parents=True)
        inner = payload / "lib" / "libsbobino_inner.dylib"
        vendored = vendor / "libsbobino_inner.dylib"
        shutil.move(str(inner), str(vendored))
        delocated = "/DLC/vendor/.dylibs/libsbobino_inner.dylib"
        self.fixture.set_install_name(vendored, delocated)
        outer = payload / "lib" / "libsbobino_outer.dylib"
        self.fixture.install_name_tool(
            "-change", str(inner), delocated, outer
        )
        self.fixture.sign(outer)

        report = portable.audit_runtime(payload)
        # The delocated id names a file of this payload, so it is reported as an
        # internal absolute reference rather than as a dead external path.
        self.assertIn(
            "absolute_runtime_reference",
            {
                item["kind"]
                for item in report["violations"]
                if delocated in item["detail"]
            },
        )

        receipt = portable.sanitize_runtime(payload)
        self.assertIn(
            delocated, {entry["install_name"] for entry in receipt["repaired_install_names"]}
        )
        self.assertEqual(portable.read_install_names(vendored), ["@rpath/libsbobino_inner.dylib"])
        self.assertIn(
            "@loader_path/../vendor/.dylibs/libsbobino_inner.dylib",
            portable.read_linked_dependencies(outer),
        )
        self.assertEqual(portable.audit_runtime(payload)["violations"], [])

        moved = self.directory / "moved" / "payload"
        moved.parent.mkdir()
        shutil.move(str(payload), str(moved))
        library = ctypes.CDLL(str(moved / "lib" / "libsbobino_outer.dylib"))
        self.assertEqual(library.sbobino_outer(), 8)

    def test_sanitize_fails_closed_on_a_missing_absolute_dependency(self):
        payload = self.fixture.build()
        outer = payload / "lib" / "libsbobino_outer.dylib"
        (payload / "lib" / "libsbobino_inner.dylib").unlink()

        with self.assertRaises(SystemExit) as raised:
            portable.sanitize_runtime(payload)
        self.assertIn("cannot be repaired offline", str(raised.exception))
        # Nothing was rewritten before the abort: the client still carries the
        # original absolute reference and is untouched.
        self.assertIn(str(outer), portable.read_linked_dependencies(payload / "bin" / "sbobino-client"))

    def test_sanitize_fails_closed_when_a_reference_escapes_via_symlink(self):
        payload = self.fixture.build()
        inner = payload / "lib" / "libsbobino_inner.dylib"
        outside = self.directory / "outside" / "libsbobino_inner.dylib"
        outside.parent.mkdir()
        shutil.move(str(inner), str(outside))
        inner.symlink_to(outside)

        with self.assertRaises(SystemExit) as raised:
            portable.sanitize_runtime(payload)
        self.assertIn("escapes the runtime", str(raised.exception))

    def test_sanitize_fails_closed_on_an_ambiguous_reference(self):
        payload = self.fixture.build()
        duplicate = payload / "extra" / "lib" / "libsbobino_inner.dylib"
        duplicate.parent.mkdir(parents=True)
        shutil.copy2(payload / "lib" / "libsbobino_inner.dylib", duplicate)
        self.fixture.set_install_name(duplicate, "/DLC/fixture/libsbobino_inner.dylib")

        ambiguous = "/nowhere/extra/lib/libsbobino_inner.dylib"
        self.fixture.install_name_tool(
            "-change", str(payload / "lib" / "libsbobino_inner.dylib"), ambiguous,
            payload / "lib" / "libsbobino_outer.dylib",
        )
        self.fixture.sign(payload / "lib" / "libsbobino_outer.dylib")

        report = portable.audit_runtime(payload)
        self.assertIn(
            "ambiguous_runtime_reference",
            {item["kind"] for item in report["violations"]},
        )
        with self.assertRaises(SystemExit) as raised:
            portable.sanitize_runtime(payload)
        self.assertIn("matches several payload files", str(raised.exception))


def build_universal_leaf(root, source_text="int sbobino_leaf(void) { return 3; }\n"):
    """Build a genuine universal dylib with one LC_ID_DYLIB per architecture."""
    root = pathlib.Path(root)
    source = root / "leaf.c"
    source.write_text(source_text)
    slices = []
    for arch in ("x86_64", "arm64"):
        binary = root / "leaf.{}.dylib".format(arch)
        subprocess.run(
            ["cc", "-arch", arch, "-dynamiclib",
             "-install_name", "/build/{}/leaf.dylib".format(arch),
             "-o", str(binary), str(source)],
            check=True,
            capture_output=True,
        )
        slices.append(binary)
    universal = root / "payload" / "lib" / "leaf.dylib"
    universal.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["lipo", "-create", *[str(slice_) for slice_ in slices],
         "-output", str(universal)],
        check=True,
        capture_output=True,
    )
    return universal


def build_bitcode_object(root, arch="arm64"):
    """Build a real LLVM bitcode object (the python.org ``python.o`` shape).

    ``cc -c -flto`` emits an ``0x0b17c0de`` LLVM bitcode wrapper; ``lipo`` then
    wraps one wrapper per architecture in a normal ``0xcafebabe`` universal
    container.  python.org's universal2 3.11 ``config-3.11-darwin/python.o`` is
    exactly that: a Mach-O *container* with no Mach-O *slice*.  Building it here
    keeps the regression test honest without shipping a vendored binary.
    """
    root = pathlib.Path(root)
    source = root / "bitcode.c"
    source.write_text("int sbobino_bitcode(void) { return 5; }\n")
    binary = root / "bitcode.{}.o".format(arch)
    subprocess.run(
        ["cc", "-arch", arch, "-c", "-flto", "-o", str(binary), str(source)],
        check=True,
        capture_output=True,
    )
    return binary


def build_universal_bitcode_object(root, name="python.o"):
    """A universal container whose slices are all LLVM bitcode."""
    root = pathlib.Path(root)
    slices = [
        build_bitcode_object(root, "x86_64"),
        build_bitcode_object(root, "arm64"),
    ]
    universal = root / "payload" / "python3.11" / "config-3.11-darwin" / name
    universal.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["lipo", "-create", *[str(slice_) for slice_ in slices],
         "-output", str(universal)],
        check=True,
        capture_output=True,
    )
    return universal


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class UniversalBinaryTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    """Regression for the parser bug that produced the phantom 21 references."""

    def setUp(self):
        self.root = self.make_test_directory()
        self.universal = build_universal_leaf(self.root)

    def test_install_names_are_reported_per_architecture(self):
        self.assertEqual(
            sorted(portable.read_install_names(self.universal)),
            ["/build/arm64/leaf.dylib", "/build/x86_64/leaf.dylib"],
        )

    def test_no_dependency_is_a_per_architecture_header(self):
        dependencies = portable.read_linked_dependencies(self.universal)
        self.assertEqual(dependencies, ["/usr/lib/libSystem.B.dylib"], dependencies)

    def test_sanitize_rewrites_every_slice_and_stays_green(self):
        receipt = portable.sanitize_runtime(self.universal.parent)
        # One -id call, one record per original per-architecture value.
        self.assertEqual(len(receipt["modified"]), 1)
        self.assertEqual(
            sorted(entry["install_name"] for entry in receipt["repaired_install_names"]),
            ["/build/arm64/leaf.dylib", "/build/x86_64/leaf.dylib"],
        )
        self.assertEqual(
            {entry["replacement"] for entry in receipt["repaired_install_names"]},
            {"@rpath/leaf.dylib"},
        )
        self.assertEqual(portable.read_install_names(self.universal), ["@rpath/leaf.dylib"])
        self.assertEqual(portable.audit_runtime(self.universal.parent)["violations"], [])
        ok, message = portable.codesign_verify(self.universal)
        self.assertTrue(ok, message)
        self.assertEqual(
            len(portable.lipo_archs(self.universal) or []), 2, "architectures were dropped"
        )


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class HostArchitectureQueryTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    """2.0.38 Intel CI failure (run 43692823687, ``macos-15-intel``).

    On that host a bare multi-architecture ``otool -D`` reported only the
    x86_64 LC_ID_DYLIB, so ``read_install_names`` lost the arm64 id and both
    ``test_install_names_are_reported_per_architecture`` and the repaired
    install-name receipt were short by one entry.  The reader now asks for one
    slice at a time with ``otool -arch``, which does not depend on the host
    architecture: the first test pins that behaviour against a tool that only
    reports the native slice, the second uses the real tools in Intel query
    mode when the machine can execute x86_64 code.
    """

    def setUp(self):
        self.root = self.make_test_directory()
        self.universal = build_universal_leaf(self.root)

    def test_a_native_only_otool_dump_does_not_hide_an_architecture(self):
        real_run_command = portable.run_command

        def intel_like_run_command(args, *, check=True):
            arguments = [str(argument) for argument in args]
            if arguments[0] == portable.OTOOL and "-D" in arguments and "-arch" not in arguments:
                # Measured shape of the hosted Intel runner: one section only.
                return subprocess.CompletedProcess(
                    args=arguments,
                    returncode=0,
                    stdout="{path} (architecture x86_64):\n/build/x86_64/leaf.dylib\n".format(
                        path=self.universal
                    ),
                    stderr="",
                )
            return real_run_command(arguments, check=check)

        with mock.patch.object(portable, "run_command", side_effect=intel_like_run_command):
            names = portable.read_install_names(self.universal)

        self.assertEqual(
            sorted(names), ["/build/arm64/leaf.dylib", "/build/x86_64/leaf.dylib"]
        )

    def test_intel_query_mode_reads_every_slice_of_a_universal_binary(self):
        probe = subprocess.run(
            ["/usr/bin/arch", "-x86_64", "/usr/bin/true"],
            capture_output=True,
        )
        if probe.returncode != 0:
            self.skipTest("this machine cannot execute x86_64 code (no Rosetta)")
        code = (
            "import json, pathlib, sys;"
            "sys.path.insert(0, sys.argv[1]);"
            "import pyannote_portable_runtime as portable;"
            "print(json.dumps(sorted(portable.read_install_names(pathlib.Path(sys.argv[2])))))"
        )
        completed = subprocess.run(
            [
                "/usr/bin/arch",
                "-x86_64",
                "/usr/bin/python3",
                "-c",
                code,
                str(SCRIPTS / "lib"),
                str(self.universal),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(
            json.loads(completed.stdout),
            ["/build/arm64/leaf.dylib", "/build/x86_64/leaf.dylib"],
        )


class MachOContainerParsingTests(unittest.TestCase):
    """The container header parser must never trust a magic or a size.

    ``python.o`` shows why: the outer magic is a valid universal container and
    the slices are not Mach-O.  A truncated or hostile container must be
    reported, never half-parsed.
    """

    THIN64 = b"\xcf\xfa\xed\xfe"
    FAT = b"\xca\xfe\xba\xbe"
    FAT64 = b"\xca\xfe\xba\xbf"

    def _write(self, name, data):
        path = pathlib.Path(self.temporary.name) / name
        path.write_bytes(data)
        return path

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sbobino container ")
        self.addCleanup(self.temporary.cleanup)

    @staticmethod
    def thin_header(
        magic=b"\xcf\xfa\xed\xfe",
        cputype=0x0100000C,
        cpusubtype=0,
        filetype=6,
        ncmds=0,
        sizeofcmds=0,
        big_endian=False,
    ):
        order = "big" if big_endian else "little"
        return (
            magic
            + cputype.to_bytes(4, order)
            + cpusubtype.to_bytes(4, order)
            + filetype.to_bytes(4, order)
            + ncmds.to_bytes(4, order)
            + sizeofcmds.to_bytes(4, order)
            + (0).to_bytes(4, order)
            + (0).to_bytes(4, order)
        )

    def fat_container(self, slices, fat64=False, big_endian=True):
        order = "big" if big_endian else "little"
        magic = self.FAT64 if fat64 else self.FAT
        if not big_endian:
            magic = bytes(reversed(magic))
        entry_size = 32 if fat64 else 20
        table_size = 8 + entry_size * len(slices)
        offset = table_size
        table = b""
        payload = b""
        for cputype, cpusubtype, data in slices:
            table += cputype.to_bytes(4, order) + cpusubtype.to_bytes(4, order)
            if fat64:
                table += offset.to_bytes(8, order) + len(data).to_bytes(8, order)
                table += (0).to_bytes(4, order) + (0).to_bytes(4, order)
            else:
                table += offset.to_bytes(4, order) + len(data).to_bytes(4, order)
                table += (0).to_bytes(4, order)
            payload += data
            offset += len(data)
        return magic + len(slices).to_bytes(4, order) + table + payload

    def test_thin_macho_header_is_classified_with_its_file_type(self):
        path = self._write("leaf.dylib", self.thin_header(filetype=6))
        container = portable.read_macho_container(path)
        self.assertEqual(container["format"], "thin")
        self.assertEqual(container["slices"][0]["kind"], "macho")
        self.assertEqual(container["slices"][0]["filetype_name"], "dylib")

    def test_object_file_type_is_distinguished_from_a_dylib(self):
        path = self._write("leaf.o", self.thin_header(filetype=1))
        container = portable.read_macho_container(path)
        self.assertEqual(container["slices"][0]["filetype_name"], "object")
        self.assertEqual(portable.read_install_names(path), [])

    def test_big_endian_thin_header_is_parsed(self):
        header = self.thin_header(magic=b"\xfe\xed\xfa\xcf", filetype=2, big_endian=True)
        path = self._write("leaf", header)
        container = portable.read_macho_container(path)
        self.assertEqual(container["format"], "thin")
        self.assertEqual(container["slices"][0]["filetype_name"], "execute")
        self.assertEqual(container["slices"][0]["arch"], "arm64")

    def test_little_endian_fat_container_is_parsed(self):
        data = self.fat_container(
            [(0x0100000C, 0, self.thin_header())],
            big_endian=False,
        )
        container = portable.read_macho_container(self._write("little.fat", data))
        self.assertEqual(container["format"], "fat32")
        self.assertEqual(container["slices"][0]["arch"], "arm64")

    def test_fat64_container_is_parsed(self):
        data = self.fat_container(
            [(0x0100000C, 0, self.thin_header()), (0x0100000C, 2, self.thin_header())],
            fat64=True,
        )
        container = portable.read_macho_container(self._write("fat64.bin", data))
        self.assertEqual(container["format"], "fat64")
        self.assertEqual([entry["arch"] for entry in container["slices"]], ["arm64", "arm64e"])

    def test_llvm_bitcode_slice_is_not_macho(self):
        container_bytes = self.fat_container(
            [(0x0100000C, 0, b"\xde\xc0\x17\x0b" + b"\x00" * 32)]
        )
        path = self._write("python.o", container_bytes)
        container = portable.read_macho_container(path)
        self.assertEqual(container["slices"][0]["kind"], "llvm_bitcode")
        self.assertEqual(portable.container_macho_slices(container), [])
        self.assertFalse(portable.is_macho(path))

    def test_truncated_thin_header_is_invalid(self):
        path = self._write("short.dylib", self.THIN64 + b"\x00" * 4)
        container = portable.read_macho_container(path)
        self.assertEqual(container["format"], "invalid")
        self.assertIn("thin Mach-O header", container["error"])

    def test_load_commands_beyond_the_file_are_invalid(self):
        header = self.thin_header(ncmds=1, sizeofcmds=64)
        path = self._write("beyond.dylib", header)
        container = portable.read_macho_container(path)
        self.assertEqual(container["format"], "invalid")
        self.assertIn("bounds", container["error"])

    def test_zero_architectures_are_invalid(self):
        data = self.FAT + (0).to_bytes(4, "big") + b"\x00" * 20
        container = portable.read_macho_container(self._write("zero.fat", data))
        self.assertEqual(container["format"], "invalid")
        self.assertIn("architecture count", container["error"])

    def test_implausible_architecture_count_is_invalid(self):
        data = self.FAT + (100000).to_bytes(4, "big") + b"\x00" * 20
        container = portable.read_macho_container(self._write("many.fat", data))
        self.assertEqual(container["format"], "invalid")
        self.assertIn("architecture count", container["error"])

    def test_slice_outside_the_file_is_invalid(self):
        data = self.fat_container([(0x0100000C, 0, self.thin_header())])
        data = data[:-4]
        container = portable.read_macho_container(self._write("cut.fat", data))
        self.assertEqual(container["format"], "invalid")
        self.assertIn("outside the file", container["error"])

    def test_overlapping_slices_are_invalid(self):
        first = self.thin_header()
        data = self.fat_container([(0x0100000C, 0, first), (0x01000007, 3, first)])
        # Rewrite the second record's offset to point inside the first slice:
        # record 1 starts after the 8-byte header and one 20-byte record, and
        # its offset field is 8 bytes into the record.
        second_record = 8 + 20
        patched = bytearray(data)
        patched[second_record + 8 : second_record + 12] = (8 + 20 + 20).to_bytes(
            4, "big"
        )
        container = portable.read_macho_container(self._write("overlap.fat", bytes(patched)))
        self.assertEqual(container["format"], "invalid")
        self.assertIn("overlap", container["error"])

    def test_non_macho_file_is_not_a_container(self):
        path = self._write("notes.txt", b"hello world\n")
        container = portable.read_macho_container(path)
        self.assertEqual(container["format"], "invalid")
        self.assertFalse(portable.is_macho(path))


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class NonMachOContainerTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    """Release run 2038, ARM64 hydration (2026-11-15T08:15:22Z).

    ``install_name_tool -id @rpath/python.o .../config-3.11-darwin/python.o``
    failed with ``input file ... is not a Mach-O file`` because the sanitizer
    accepted a universal container whose slices are LLVM bitcode.  These tests
    fail on the pre-fix module and pin every recovery property: the object is
    never treated as code, never rewritten, never signed, stays byte identical
    and is still reported.
    """

    def setUp(self):
        self.root = self.make_test_directory()

    def test_universal_bitcode_object_is_not_a_mach_o_file(self):
        object_file = build_universal_bitcode_object(self.root)
        container = portable.read_macho_container(object_file)
        self.assertEqual(container["format"], "fat32")
        self.assertEqual(
            [entry["kind"] for entry in container["slices"]],
            ["llvm_bitcode", "llvm_bitcode"],
        )
        self.assertFalse(portable.is_macho(object_file))
        self.assertEqual(portable.iter_macho(object_file.parent), [])

    def test_readers_never_invent_an_install_name_for_bitcode(self):
        object_file = build_universal_bitcode_object(self.root)
        self.assertEqual(portable.read_install_names(object_file), [])
        self.assertIsNone(portable.read_install_name(object_file))

        # The raw tool output is the diagnostic that was misparsed before:
        # prove the parser refuses to turn it into a load command.
        diagnostic = "{path} (architecture arm64): is an LLVM bit-code file\n".format(
            path=object_file
        )
        self.assertEqual(portable.otool_diagnostics(diagnostic), [diagnostic.strip()])
        self.assertEqual(portable.parse_install_names(diagnostic), [])
        self.assertEqual(portable.parse_otool_dependencies(diagnostic), [])

    def test_diagnostic_filter_keeps_real_load_commands(self):
        mixed = (
            "/payload/lib/x.so (architecture arm64): is an LLVM bit-code file\n"
            "/payload/lib/x.so (architecture arm64):\n"
            "\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1)\n"
        )
        self.assertEqual(
            portable.parse_otool_dependencies(mixed), ["/usr/lib/libSystem.B.dylib"]
        )

    def test_audit_discloses_the_bitcode_object_without_failing(self):
        object_file = build_universal_bitcode_object(self.root)
        report = portable.audit_runtime(object_file.parent)
        self.assertEqual(report["violations"], [])
        self.assertEqual(report["binaries"], 0)
        self.assertEqual(
            [item["kind"] for item in report["warnings"]], ["non_macho_container"]
        )
        self.assertIn("LLVM bit-code", report["warnings"][0]["detail"])

    def test_sanitize_is_a_no_op_and_never_runs_install_name_tool(self):
        object_file = build_universal_bitcode_object(self.root)
        before = object_file.read_bytes()
        calls = []
        real_run_command = portable.run_command

        def recording_run_command(args, *, check=True):
            arguments = [str(argument) for argument in args]
            calls.append(arguments)
            return real_run_command(arguments, check=check)

        with mock.patch.object(portable, "run_command", side_effect=recording_run_command):
            receipt = portable.sanitize_runtime(object_file.parent)

        self.assertEqual(receipt["modified"], [])
        self.assertEqual(receipt["repaired_install_names"], [])
        self.assertEqual(receipt["signed"], [])
        self.assertEqual(
            [entry["path"] for entry in receipt["skipped_non_macho"]], ["python.o"]
        )
        self.assertIn("LLVM bit-code", receipt["skipped_non_macho"][0]["slices"])
        self.assertFalse(
            [call for call in calls if call[0] == portable.INSTALL_NAME_TOOL],
            calls,
        )
        self.assertFalse([call for call in calls if call[0] == portable.CODESIGN], calls)
        self.assertEqual(object_file.read_bytes(), before)

    def test_mixed_container_fails_closed_before_any_rewrite(self):
        # A universal binary with a real dylib slice and an LLVM bitcode slice
        # cannot be verified slice by slice, so the sanitizer must refuse it
        # instead of rewriting the half it can read.
        dylib = build_universal_leaf(self.root)  # fat, but we need single slices
        arm64 = self.root / "leaf.arm64.dylib"
        bitcode = build_bitcode_object(self.root, "x86_64")
        mixed = self.root / "payload" / "mixed.dylib"
        mixed.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["lipo", "-create", str(arm64), str(bitcode), "-output", str(mixed)],
            check=True,
            capture_output=True,
        )
        self.assertEqual(
            sorted(entry["kind"] for entry in portable.read_macho_container(mixed)["slices"]),
            ["llvm_bitcode", "macho"],
        )

        before = mixed.read_bytes()
        with self.assertRaises(SystemExit) as raised:
            portable.sanitize_runtime(mixed.parent)
        self.assertIn("mixes Mach-O and non-Mach-O slices", str(raised.exception))
        self.assertEqual(mixed.read_bytes(), before)

        report = portable.audit_runtime(mixed.parent)
        self.assertIn("mixed_slice_macho", {item["kind"] for item in report["violations"]})
        self.assertTrue(dylib.exists())


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class HostManagedEmbeddingTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    """Embedding a host dylib must repair the copy, not only the client.

    Measured on the ordinary hydration (Homebrew python@3.11): the payload's
    ``_ssl`` links ``/opt/homebrew/.../libssl.3.dylib``, which was copied into
    ``lib/embedded-dylibs`` and got a portable ``-id``... while still linking
    ``/opt/homebrew/.../libcrypto.3.dylib`` and advertising
    ``/opt/homebrew/Cellar/openssl@3/.../lib`` as LC_RPATH.  The audit then
    reported ``host_dependency``/``host_rpath`` against the embedded copy and
    the hydration failed after all the expensive work was done.  python.org's
    universal2 runtime has the same shape (``_ssl`` -> libssl/libcrypto under
    /Library/Frameworks), so this is a release blocker, not a local quirk.

    The fixture fakes a host prefix inside the test directory so nothing is
    written outside it; the sanitizer is exercised through its real
    ``HOST_PREFIXES`` branch.
    """

    def setUp(self):
        self.root = self.make_test_directory()
        self.host_root = self.root / "fake host root"
        self.host_lib = self.host_root / "lib"
        self.host_lib.mkdir(parents=True)
        source = self.root / "src"
        source.mkdir()
        (source / "dep.c").write_text("int sbobino_dep(void) { return 4; }\n")
        (source / "host.c").write_text(
            "int sbobino_dep(void);\nint sbobino_host(void) { return sbobino_dep() + 1; }\n"
        )
        (source / "client.c").write_text(
            "#include <stdio.h>\n"
            "int sbobino_host(void);\n"
            'int main(void) { printf("%d\\n", sbobino_host()); return 0; }\n'
        )
        self.dep = self.host_lib / "libsbobino_dep.dylib"
        self.host = self.host_lib / "libsbobino_host.dylib"
        subprocess.run(
            ["cc", "-arch", NATIVE_ARCH, "-dynamiclib", "-Wl,-headerpad_max_install_names",
             "-install_name", str(self.dep), "-o", str(self.dep), str(source / "dep.c")],
            check=True, capture_output=True,
        )
        subprocess.run(
            ["cc", "-arch", NATIVE_ARCH, "-dynamiclib", "-Wl,-headerpad_max_install_names",
             "-install_name", str(self.host), "-o", str(self.host),
             str(source / "host.c"), str(self.dep),
             "-Wl,-rpath," + str(self.host_lib)],
            check=True, capture_output=True,
        )
        for binary in (self.dep, self.host):
            if str(self.host_lib) not in portable.read_otool_rpaths(binary):
                subprocess.run(
                    ["/usr/bin/install_name_tool", "-add_rpath", str(self.host_lib), str(binary)],
                    check=True, capture_output=True,
                )
            subprocess.run(
                ["/usr/bin/codesign", "--force", "--sign", "-", str(binary)],
                check=True, capture_output=True,
            )
        self.payload = self.root / "payload"
        (self.payload / "bin").mkdir(parents=True)
        self.client = self.payload / "bin" / "sbobino-client"
        subprocess.run(
            ["cc", "-arch", NATIVE_ARCH, "-Wl,-headerpad_max_install_names",
             "-o", str(self.client), str(source / "client.c"), str(self.host)],
            check=True, capture_output=True,
        )

    def patched_prefixes(self):
        return mock.patch.object(
            portable, "HOST_PREFIXES", portable.HOST_PREFIXES + (str(self.host_root),)
        )

    def test_host_dylibs_are_embedded_and_the_copies_are_sanitized(self):
        with self.patched_prefixes():
            before = portable.audit_runtime(self.payload)
            self.assertIn("host_dependency", {item["kind"] for item in before["violations"]})

            receipt = portable.sanitize_runtime(self.payload)

            embedded = set(receipt["embedded_copies"])
            self.assertIn("lib/embedded-dylibs/libsbobino_host.dylib", embedded)
            self.assertIn("lib/embedded-dylibs/libsbobino_dep.dylib", embedded)
            # The second pass is what repairs the embedded copy's own deps.
            self.assertGreaterEqual(receipt["passes"], 2, receipt)

            embedded_host = self.payload / "lib" / "embedded-dylibs" / "libsbobino_host.dylib"
            embedded_dep = self.payload / "lib" / "embedded-dylibs" / "libsbobino_dep.dylib"
            self.assertEqual(portable.read_otool_rpaths(embedded_host), [])
            self.assertEqual(portable.read_otool_rpaths(embedded_dep), [])
            self.assertEqual(
                portable.read_linked_dependencies(embedded_host),
                ["@loader_path/libsbobino_dep.dylib", "/usr/lib/libSystem.B.dylib"],
            )
            self.assertIn(
                "@loader_path/../lib/embedded-dylibs/libsbobino_host.dylib",
                portable.read_linked_dependencies(self.client),
            )
            self.assertEqual(portable.read_install_names(embedded_host), ["@rpath/libsbobino_host.dylib"])
            self.assertEqual(portable.read_install_names(embedded_dep), ["@rpath/libsbobino_dep.dylib"])

            after = portable.audit_runtime(self.payload)
            self.assertEqual(after["violations"], [], after["violations"])

            second = portable.sanitize_runtime(self.payload)
            self.assertEqual(second["modified"], [])
            self.assertEqual(second["passes"], 0)
            self.assertGreater(len(receipt["signed"]), 0)

        # The rewritten chain must actually load from somewhere else.
        moved = self.root / "moved payload with space" / "payload"
        moved.parent.mkdir()
        shutil.move(str(self.payload), str(moved))
        completed = subprocess.run([str(moved / "bin" / "sbobino-client")],
                                   capture_output=True, text=True)
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout.strip(), "5")


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class PackagingGuardTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    def setUp(self):
        self.directory = self.make_test_directory()
        self.fixture = MachOFixture(self.directory)

    def _package(self, source, output):
        return subprocess.run(
            [str(SCRIPTS / "package_pyannote_asset.sh"), str(source), "python", str(output)],
            capture_output=True,
            text=True,
        )

    def test_guard_rejects_stale_runtime_and_accepts_repaired_one(self):
        runtime = self.fixture.install_runtime()
        self.fixture.add_rpath(runtime / "lib" / "libfixture.dylib", "/opt/homebrew/lib")
        stale_zip = self.directory / "stale.zip"

        rejected = self._package(runtime, stale_zip)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("host_rpath", rejected.stdout + rejected.stderr)
        self.assertFalse(stale_zip.exists())

        portable.sanitize_runtime(runtime)
        accepted = self._package(runtime, stale_zip)
        self.assertEqual(accepted.returncode, 0, accepted.stdout + accepted.stderr)
        self.assertTrue(stale_zip.exists())


@unittest.skipUnless(MACHO_TOOLING, "needs macOS Mach-O tooling")
class RepairScriptTests(SanitizerTestDirectoryMixin, unittest.TestCase):
    def setUp(self):
        self.directory = self.make_test_directory()
        self.fixture = MachOFixture(self.directory)

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
                "TMPDIR": str(self.directory / "tmp"),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin",
            },
        )

    def test_repair_produces_green_payload_and_leaves_source_red(self):
        root = self.directory
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
        root = self.directory
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
