#!/usr/bin/env python3
"""Portable Pyannote runtime inspection, sanitization and audit.

This module is the single source of truth for the ``otool`` parsing, the
host-managed-path sanitizer and the strict portability audit of a bundled
Pyannote Python runtime.  It is shared by:

* ``scripts/setup_bundled_pyannote.sh``      (generator, stage tree)
* ``scripts/package_pyannote_asset.sh``      (packaging guard, release asset)
* ``scripts/sanitize_pyannote_portable_runtime.sh`` (repair of a staged tree)

Sharing one implementation is not cosmetic.  The June 2026 asset under
``apps/desktop/src-tauri/resources/pyannote`` was produced by a parser that
read the line after ``cmd LC_RPATH`` (``cmdsize``) instead of ``path``, so
every LC_RPATH was silently skipped and the runtime kept
``/opt/homebrew/lib``.  The companion installer rejects such a runtime on
purpose (``validate_portable_pyannote_native_dependencies`` in
``crates/infrastructure/src/lib.rs``), so the shipped bundle could never be
installed.  Keeping the parser and the audit in one place is what prevents
that class of skew between generator, packager and repair tool.

CLI
---
``pyannote_portable_runtime.py sanitize <runtime-root> [--json PATH]``
    Make a runtime tree position independent, in place (the caller owns the
    tree; this tool never clones or copies), then re-sign every modified
    Mach-O and the enclosing ``.app`` bundles, in that order:

    * LC_ID_DYLIB values that are absolute build-machine paths or bare
      relative build paths become ``@rpath/<basename>``;
    * LC_LOAD_DYLIB targets inside the tree become ``@loader_path/<relative>``;
    * host-managed (``HOST_PREFIXES``) dylibs are still copied into
      ``lib/embedded-dylibs``;
    * LC_RPATH entries that are host managed or absolute are removed.

    Files whose existing *adhoc* signature no longer matches their content are
    re-signed adhoc: another build tool (conda's prefix rewrite) can change
    string data without touching a load command, and no identity exists to
    protect in that case. An invalid non-adhoc signature, and any verification
    failure that is not a known "modified content" or "unsigned" outcome,
    aborts the run instead of being masked.

    Everything else (system libraries, ``@loader_path``/``@executable_path``/
    ``@rpath`` references) is preserved.  A non-system absolute dependency
    whose target cannot be located *and verified* inside the tree aborts the
    run instead of being rewritten: guessing a target by basename is what
    silently produces a payload that only loads on the build machine.

    "Verified" is provenance-sensitive.  An absolute path that exists inside
    the payload at exactly that path is the file dyld would open, so it is
    accepted when the target is a real ``MH_DYLIB`` with a loader-relative (or
    scheduled-for-repair) identity — conda's ``@rpath`` id with an absolute
    client path is the measured osx-64 layout.  A path that does *not* exist on
    the build machine can only be matched by reconstructing its tail, which is
    weaker provenance, so that case still requires the target to declare the
    recorded reference as one of its install names.

``pyannote_portable_runtime.py audit <runtime-root> [--expect-arch ARCH] [--json PATH] [--strict-closure]``
    Strict audit: architecture, host-managed dependency/rpath/install-name
    references, signature validity for signed Mach-O, enclosing ``.app``
    seals and personal-path substrings in load commands.  Exit status 1 on
    violations.

    Mach-O membership is decided from the real per-slice headers, never from
    the container magic alone: python.org's universal2 3.11 runtime ships
    ``lib/python3.11/config-3.11-darwin/python.o`` as a fat container whose two
    slices are LLVM bitcode wrappers.  Containers with no Mach-O slice are
    reported as ``non_macho_container`` warnings (inert data, no load
    commands), a container that mixes Mach-O and non-Mach-O slices is a
    ``mixed_slice_macho`` violation, and a truncated/broken container is a
    ``malformed_macho_container`` violation.  Every Mach-O slice is queried
    explicitly with ``otool -arch <arch>``, so a tool that reports only the
    host architecture in a multi-architecture dump cannot hide half of the
    load commands.

    Absolute non-system *install names* and *dependencies* are violations.  An
    absolute reference that resolves to a file inside the audited tree is
    reported as ``absolute_runtime_reference`` regardless of the root prefix:
    detection is anchored on the payload layout, not on ``/Users`` or
    ``/Volumes``, so a build root recorded under a scratch volume cannot hide
    the defect.  A reference that matches more than one payload file is
    ``ambiguous_runtime_reference`` (fail closed, never a basename guess).

    Unresolved ``@rpath`` dependencies stay warnings by default and become
    violations with ``--strict-closure``.  Rationale, measured on the shipped
    asset: torchcodec ships parallel ``libtorchcodec_*N`` variants prebuilt
    against FFmpeg major versions that are not bundled (only the active
    variant is imported).  Refusing those by default would block every release
    for a reason the app does not reject at install time and that cannot be
    fixed without replacing the wheels.

``pyannote_portable_runtime.py inventory <root> <output.json>``
    Deterministic inventory (path, type, sha256, size, symlink target) plus a
    rolled-up tree hash, used for the payload manifest.  Files that carry a
    Mach-O container magic also record their ``macho`` metadata (container
    format, per-slice architecture and Mach-O file type), so an object file is
    distinguishable from a dylib and an executable, and a universal container
    whose slices are LLVM bitcode is visible instead of implied.

``pyannote_portable_runtime.py guard-paths <input> <output>``
    Path guard for the repair tool: rejects personal profile paths, an output
    that is the input, and outputs nested inside the input (or vice versa).
    Prints the canonical paths as JSON.

``pyannote_portable_runtime.py guard-root <path>``
    Path guard for the generator: rejects the same personal/lockbox roots for
    a single directory (used to vet the staging parent before any download).

The module stays Python 3.9 compatible because the release scripts call
whatever ``python3`` is first on PATH on the build machine.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path

HOST_PREFIXES = ("/opt/homebrew", "/usr/local", "/Library/Frameworks")
SYSTEM_PREFIXES = ("/usr/lib/", "/System/", "/usr/libexec/")
PERSONAL_PATH_MARKERS = (
    "/Library/Application Support/",
    "/private/",
    "/pairing",
    "master_key_v1",
    "pairing.enc",
)
# Load commands are the only place where a path can make the loader reach
# outside the payload, so the audit forbids user-home roots there as well.
# Embedded build metadata (DWARF unit names, compiled-in PREFIX strings) is
# not a loader reference and is reported separately by the string scan in the
# repair evidence.
LOAD_COMMAND_PATH_MARKERS = PERSONAL_PATH_MARKERS + ("/Users/", "/home/")

OTOOL = "/usr/bin/otool"
INSTALL_NAME_TOOL = "/usr/bin/install_name_tool"
CODESIGN = "/usr/bin/codesign"
LIPO = "/usr/bin/lipo"

# Thin Mach-O magics and universal ("fat") container magics, both endiannesses.
THIN_MACHO_MAGICS = {
    b"\xcf\xfa\xed\xfe",
    b"\xce\xfa\xed\xfe",
    b"\xfe\xed\xfa\xcf",
    b"\xfe\xed\xfa\xce",
}
FAT_MACHO_MAGICS = {
    b"\xca\xfe\xba\xbe",  # fat32, big endian
    b"\xbe\xba\xfe\xca",  # fat32, little endian
}
FAT64_MACHO_MAGICS = {
    b"\xca\xfe\xba\xbf",  # fat64, big endian (64-bit slice offsets)
    b"\xbf\xba\xfe\xca",  # fat64, little endian
}
MACHO_MAGICS = THIN_MACHO_MAGICS | FAT_MACHO_MAGICS | FAT64_MACHO_MAGICS

# A container magic is *not* evidence that the slices are Mach-O.  python.org's
# universal2 3.11 installer ships
# ``lib/python3.11/config-3.11-darwin/python.o`` as a fat container
# (0xcafebabe) whose two slices are LLVM bitcode wrappers (0x0b17c0de):
#
#   $ otool -D .../python.o
#   .../python.o (architecture x86_64): is an LLVM bit-code file
#   .../python.o (architecture arm64): is an LLVM bit-code file
#   $ install_name_tool -id @rpath/python.o .../python.o
#   .../python.o is not a Mach-O file
#
# `otool` exits 0 and prints a diagnostic where load commands would be, so a
# parser that skips only section headers mistakes the diagnostic for an
# LC_ID_DYLIB and the sanitizer "plans" a rewrite for a file that has none.
# Release candidate 2.0.38 died exactly there (release run 2038, ARM64 job,
# 2026-11-15T08:15:22Z).  Every consumer therefore parses the container itself,
# bounds-checks it, classifies each slice from its own header and queries
# `otool -arch <arch>` only for slices that really are Mach-O.
LLVM_BITCODE_MAGICS = {
    b"\xde\xc0\x17\x0b",  # wrapper, little endian (measured on python.org 3.11.9)
    b"\x0b\x17\xc0\xde",  # wrapper, big endian
    b"BC\xc0\xde",  # raw bitcode stream
}

MH_MAGIC = 0xFEEDFACE
MH_MAGIC_64 = 0xFEEDFACF
MH_FILE_TYPES = {
    1: "object",
    2: "execute",
    3: "fvmlib",
    4: "core",
    5: "preload",
    6: "dylib",
    7: "dylinker",
    8: "bundle",
    9: "dylib_stub",
    10: "dsym",
    11: "kext_bundle",
}
MH_DYLIB = 6
CPU_ARCH_ABI64 = 0x01000000
CPU_TYPE_X86 = 7
CPU_TYPE_X86_64 = CPU_TYPE_X86 | CPU_ARCH_ABI64
CPU_TYPE_ARM = 12
CPU_TYPE_ARM64 = CPU_TYPE_ARM | CPU_ARCH_ABI64
CPU_SUBTYPE_LOW_MASK = 0x00FFFFFF
# A universal container larger than this is not a real macOS payload and would
# only make the parser allocate on attacker-controlled input.
MAX_FAT_ARCHITECTURES = 64

ARCH_FOR_TRIPLE = {
    "aarch64-apple-darwin": "arm64",
    "x86_64-apple-darwin": "x86_64",
}
TRIPLE_FOR_ARCH = {value: key for key, value in ARCH_FOR_TRIPLE.items()}


def arch_name(cputype, cpusubtype):
    """Return the architecture name for a ``cputype``/``cpusubtype`` pair."""
    if cputype is None or cpusubtype is None:
        return None
    subtype = cpusubtype & CPU_SUBTYPE_LOW_MASK
    if cputype == CPU_TYPE_ARM64:
        # arm64e (pointer authentication ABI) only exists as a stdlib value on
        # macOS; nothing in the Pyannote payload ships it today.
        return "arm64e" if subtype == 2 else "arm64"
    if cputype == CPU_TYPE_X86_64:
        return "x86_64h" if subtype == 8 else "x86_64"
    if cputype == CPU_TYPE_ARM:
        return "arm"
    if cputype == CPU_TYPE_X86:
        return "i386"
    return None


def _read_at(handle, offset, size):
    """Read ``size`` bytes at ``offset``, or ``None`` when the file is short."""
    handle.seek(offset)
    data = handle.read(size)
    if len(data) != size:
        return None
    return data


def _thin_macho_header(handle, offset, limit):
    """Parse a thin Mach-O header at ``offset`` (bounded by ``limit``)."""
    magic = _read_at(handle, offset, 4)
    if magic is None or magic not in THIN_MACHO_MAGICS:
        return None
    big_endian = magic in (b"\xfe\xed\xfa\xce", b"\xfe\xed\xfa\xcf")
    is_64 = magic in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf")
    header_size = 32 if is_64 else 28
    raw = _read_at(handle, offset, header_size)
    if raw is None or offset + header_size > limit:
        return None
    order = "big" if big_endian else "little"
    cputype = int.from_bytes(raw[4:8], order, signed=False)
    cpusubtype = int.from_bytes(raw[8:12], order, signed=False)
    filetype = int.from_bytes(raw[12:16], order, signed=False)
    ncmds = int.from_bytes(raw[16:20], order, signed=False)
    sizeofcmds = int.from_bytes(raw[20:24], order, signed=False)
    start = offset + header_size
    if start + sizeofcmds > limit:
        return {"error": "load commands exceed the file bounds"}
    if ncmds > 100000:
        return {"error": "implausible load-command count {}".format(ncmds)}
    return {
        "cputype": cputype,
        "cpusubtype": cpusubtype,
        "filetype": filetype,
        "filetype_name": MH_FILE_TYPES.get(filetype, "unknown-{}".format(filetype)),
        "ncmds": ncmds,
        "sizeofcmds": sizeofcmds,
        "bits": 64 if is_64 else 32,
        "error": None,
    }


def read_macho_container(path):
    """Classify a file as thin/fat Mach-O and inspect every slice header.

    Returns ``{"format": ..., "slices": [...], "error": ...}`` where ``format``
    is ``thin``, ``fat32``, ``fat64`` or ``invalid`` and every slice carries
    ``kind`` (``macho``, ``llvm_bitcode`` or ``unknown``).  A container that
    only has a Mach-O-looking magic but whose slices are not Mach-O is not a
    Mach-O file for the purposes of the sanitizer and the audit.

    The parse is entirely local and bounds checked: offsets, sizes, slice count
    and load-command tables are validated against the real file length, so a
    truncated or hostile container is reported instead of trusted.
    """
    path = Path(path)
    try:
        size = path.stat().st_size
    except OSError as error:
        return {"format": "invalid", "slices": [], "error": "stat failed: {}".format(error)}

    try:
        handle = open(path, "rb")
    except OSError as error:
        return {"format": "invalid", "slices": [], "error": "open failed: {}".format(error)}

    with handle:
        magic = _read_at(handle, 0, 4)
        if magic is None:
            return {"format": "invalid", "slices": [], "error": "file is shorter than a magic"}

        if magic in THIN_MACHO_MAGICS:
            header = _thin_macho_header(handle, 0, size)
            if header is None or header.get("error"):
                return {
                    "format": "invalid",
                    "slices": [],
                    "error": (header or {}).get("error", "unreadable thin Mach-O header"),
                }
            slice_entry = {
                "index": 0,
                "offset": 0,
                "size": size,
                "arch": arch_name(header["cputype"], header["cpusubtype"]),
                "kind": "macho",
                "magic": magic.hex(),
                "cputype": header["cputype"],
                "cpusubtype": header["cpusubtype"],
                "filetype": header["filetype"],
                "filetype_name": header["filetype_name"],
            }
            return {"format": "thin", "slices": [slice_entry], "error": None}

        if magic not in FAT_MACHO_MAGICS and magic not in FAT64_MACHO_MAGICS:
            return {"format": "invalid", "slices": [], "error": "not a Mach-O container"}

        big_endian = magic in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf")
        is_fat64 = magic in FAT64_MACHO_MAGICS
        order = "big" if big_endian else "little"
        header = _read_at(handle, 0, 8)
        if header is None:
            return {"format": "invalid", "slices": [], "error": "truncated fat header"}
        nfat_arch = int.from_bytes(header[4:8], order, signed=False)
        if nfat_arch == 0 or nfat_arch > MAX_FAT_ARCHITECTURES:
            return {
                "format": "invalid",
                "slices": [],
                "error": "implausible universal binary architecture count {}".format(nfat_arch),
            }

        entry_size = 32 if is_fat64 else 20
        table_size = 8 + nfat_arch * entry_size
        if table_size > size:
            return {
                "format": "invalid",
                "slices": [],
                "error": "universal binary header table exceeds the file",
            }
        table = _read_at(handle, 8, nfat_arch * entry_size)
        if table is None:
            return {"format": "invalid", "slices": [], "error": "truncated universal binary table"}

        slices = []
        ranges = []
        for index in range(nfat_arch):
            entry = table[index * entry_size : (index + 1) * entry_size]
            if entry_size == 32:
                cputype = int.from_bytes(entry[0:4], order, signed=False)
                cpusubtype = int.from_bytes(entry[4:8], order, signed=False)
                offset = int.from_bytes(entry[8:16], order, signed=False)
                slice_size = int.from_bytes(entry[16:24], order, signed=False)
            else:
                cputype = int.from_bytes(entry[0:4], order, signed=False)
                cpusubtype = int.from_bytes(entry[4:8], order, signed=False)
                offset = int.from_bytes(entry[8:12], order, signed=False)
                slice_size = int.from_bytes(entry[12:16], order, signed=False)
            if slice_size == 0 or offset < table_size or offset + slice_size > size:
                return {
                    "format": "invalid",
                    "slices": [],
                    "error": "universal slice {} lies outside the file".format(index),
                }
            for start, end in ranges:
                if offset < end and start < offset + slice_size:
                    return {
                        "format": "invalid",
                        "slices": [],
                        "error": "universal slices overlap at index {}".format(index),
                    }
            ranges.append((offset, offset + slice_size))

            slice_magic = _read_at(handle, offset, 4)
            kind = "unknown"
            header_info = None
            if slice_magic in THIN_MACHO_MAGICS:
                parsed = _thin_macho_header(handle, offset, offset + slice_size)
                if parsed is None or parsed.get("error"):
                    return {
                        "format": "invalid",
                        "slices": [],
                        "error": "slice {}: {}".format(
                            index, (parsed or {}).get("error", "unreadable Mach-O header")
                        ),
                    }
                header_info = parsed
                kind = "macho"
            elif slice_magic in LLVM_BITCODE_MAGICS:
                kind = "llvm_bitcode"
            slices.append(
                {
                    "index": index,
                    "offset": offset,
                    "size": slice_size,
                    "arch": arch_name(
                        cputype, cpusubtype
                    )
                    or (arch_name(header_info["cputype"], header_info["cpusubtype"]) if header_info else None),
                    "kind": kind,
                    "magic": slice_magic.hex() if slice_magic else None,
                    "cputype": header_info["cputype"] if header_info else cputype,
                    "cpusubtype": header_info["cpusubtype"] if header_info else cpusubtype,
                    "filetype": header_info["filetype"] if header_info else None,
                    "filetype_name": header_info["filetype_name"] if header_info else None,
                }
            )
        return {
            "format": "fat64" if is_fat64 else "fat32",
            "slices": slices,
            "error": None,
        }


def container_macho_slices(container):
    return [entry for entry in container["slices"] if entry["kind"] == "macho"]


def container_non_macho_slices(container):
    return [entry for entry in container["slices"] if entry["kind"] != "macho"]


def describe_slice_kinds(container):
    kinds = []
    for entry in container["slices"]:
        kind = entry["kind"]
        if kind == "llvm_bitcode":
            kind = "LLVM bit-code"
        label = "{}:{}".format(entry["arch"] or "unknown", kind)
        kinds.append(label)
    return ", ".join(kinds)


# ---------------------------------------------------------------------------
# otool parsing (single copy for every caller)
# ---------------------------------------------------------------------------

# ``otool`` prints one section per architecture.  A thin file starts with
# ``/path/to/file:``; a fat file repeats ``/path/to/file (architecture arm64):``
# before each slice.  Those header lines are *not* load commands.  The
# pre-November-2026 parser only skipped line 1, so every fat binary contributed
# a phantom dependency equal to its own absolute path (11 of 449 files in the
# 2.0.36 payload) and the sanitizer would have tried to rewrite a load command
# that does not exist.
_OTOOL_SECTION_HEADER = re.compile(
    r"^(?P<path>.*?)(?: \(architecture [A-Za-z0-9_]+\))?:$"
)

# ``otool`` exits 0 and prints a *diagnostic* where load commands would be when
# a slice is not Mach-O, for example::
#
#   /payload/python.o (architecture arm64): is an LLVM bit-code file
#   /payload/thing.dylib: is not a Mach-O file
#
# Such a line is not a section header (it does not end with ``:``) and must
# never be mistaken for an install name or a dependency.
_OTOOL_FILE_DIAGNOSTIC = re.compile(
    r"^(?P<path>.+?)(?: \(architecture [A-Za-z0-9_]+\))?: is (?:an? )?"
    r"(?P<kind>LLVM bit-code|not a Mach-O|not an object) file\.?$"
)


def otool_diagnostics(output: str) -> list[str]:
    """Return the ``is not a Mach-O file`` style diagnostics of an otool dump."""
    found = []
    for line in output.splitlines():
        stripped = line.strip()
        if stripped and _OTOOL_FILE_DIAGNOSTIC.match(stripped):
            found.append(stripped)
    return found


def _otool_section_lines(output: str):
    """Yield load-command lines, skipping per-file/per-arch headers."""
    for line in output.splitlines():
        stripped = line.strip()
        if not stripped or _OTOOL_SECTION_HEADER.match(stripped):
            continue
        if _OTOOL_FILE_DIAGNOSTIC.match(stripped):
            # Never let a tool diagnostic masquerade as a load command.
            continue
        yield stripped


def parse_install_names(output: str) -> list[str]:
    """Return the distinct LC_ID_DYLIB values of an ``otool -D`` dump."""
    refs = []
    for stripped in _otool_section_lines(output):
        ref = stripped.split(" (", 1)[0].strip()
        if ref and ref not in refs:
            refs.append(ref)
    return refs


def parse_otool_dependencies(output: str) -> list[str]:
    """Return the linked-library paths from ``otool -L`` output.

    The reference is everything before the ``(compatibility version ...)``
    suffix; splitting on the first space would truncate a path that contains
    one (``/Volumes/My Disk/...``), which is a real staging layout.

    Duplicates are collapsed: a fat binary lists the same system libraries once
    per slice, and counting them twice inflates every report.
    """
    refs = []
    for stripped in _otool_section_lines(output):
        ref = stripped.split(" (", 1)[0].strip()
        if ref and ref not in refs:
            refs.append(ref)
    return refs


def parse_otool_rpaths(output: str) -> list[str]:
    """Return the ``path`` values of every LC_RPATH in ``otool -l`` output.

    ``otool -l`` prints a multi-line block per LC_RPATH command::

        cmd LC_RPATH
        cmdsize 88
        path /opt/homebrew/... (offset 12)

    The line immediately after ``cmd LC_RPATH`` is ``cmdsize``, not ``path``.
    Reading the next line only (as the pre-October-2026 script did) silently
    skips every rpath.  Use a sticky flag and clear it when the matching
    ``path`` line is consumed.
    """
    refs = []
    in_rpath = False
    for line in output.splitlines():
        stripped = line.strip()
        if stripped == "cmd LC_RPATH":
            in_rpath = True
            continue
        if in_rpath and stripped.startswith("path "):
            ref = stripped.split("path ", 1)[1].split(" (offset ", 1)[0]
            if ref not in refs:
                refs.append(ref)
            in_rpath = False
    return refs


# ---------------------------------------------------------------------------
# Process helpers
# ---------------------------------------------------------------------------


def run_command(args, *, check=True):
    completed = subprocess.run(
        [str(arg) for arg in args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and completed.returncode != 0:
        raise SystemExit(
            "command failed: {} (exit {})\n{}{}".format(
                " ".join(str(arg) for arg in args),
                completed.returncode,
                completed.stdout,
                completed.stderr,
            )
        )
    return completed


def _container_arch_names(binary: Path, container) -> list[str]:
    """Return one ``otool -arch`` name per slice, cross-checked with ``lipo``.

    ``-arch`` is an arch-selection flag, independent of the host architecture,
    so a universal binary is inspected slice by slice instead of relying on the
    multi-architecture dump of a single ``otool`` call.  The dumped form is not
    portable across tool versions/hosts: on the hosted Intel runners
    (macos-15-intel, Xcode 16.x) a bare ``otool -D <fat>`` reported only the
    x86_64 LC_ID_DYLIB, which failed ``test_install_names_are_reported_per_
    architecture`` and the repaired-install-name receipt (CI run 43692823687).
    """
    names = lipo_archs(binary)
    slices = container["slices"]
    if not names or len(names) != len(slices):
        raise SystemExit(
            "cannot pair the universal slices of '{}' with 'lipo -archs' "
            "(lipo: {}; container: {}); refusing to query the wrong slice".format(
                binary, names, len(slices)
            )
        )
    for entry, name in zip(slices, names):
        mapped = entry["arch"]
        if mapped and mapped != name:
            raise SystemExit(
                "slice {} of '{}' has cputype {} ({}), but lipo reports '{}'; "
                "refusing to query the wrong architecture".format(
                    entry["index"], binary, entry["cputype"], mapped, name
                )
            )
    return names


def _macho_slice_queries(binary: Path, container):
    """Return ``[(arch_name_or_None, slice)]`` for the slices we may query."""
    if container["format"] == "invalid":
        raise SystemExit(
            "cannot read Mach-O container '{}': {}".format(binary, container["error"])
        )
    macho_slices = container_macho_slices(container)
    others = container_non_macho_slices(container)
    if others and macho_slices:
        raise SystemExit(
            "'{}' mixes Mach-O and non-Mach-O slices ({}); load commands cannot "
            "be read per slice, refusing a partial rewrite".format(
                binary, describe_slice_kinds(container)
            )
        )
    if container["format"] == "thin":
        return [(None, entry) for entry in macho_slices]
    names = _container_arch_names(binary, container)
    return [
        (names[index], entry)
        for index, entry in enumerate(container["slices"])
        if entry["kind"] == "macho"
    ]


def _read_slice_load_commands(binary: Path, container, flag: str, selector=None) -> str:
    """Run ``otool <flag>`` per Mach-O slice and return the merged raw output.

    An empty stdout from a slice we already know is Mach-O means the tool
    ignored the request (``otool -arch`` prints nothing and exits 0 for a
    non-matching arch); that is a hard failure, never an empty load-command
    list.
    """
    queries = _macho_slice_queries(binary, container)
    if selector is not None:
        queries = [(name, entry) for name, entry in queries if selector(entry)]
    outputs = []
    for name, entry in queries:
        if name is None:
            completed = run_command([OTOOL, flag, binary])
        else:
            completed = run_command([OTOOL, "-arch", name, flag, binary])
        if not completed.stdout.strip():
            raise SystemExit(
                "otool {}{} produced no output for slice '{}' of '{}'; refusing "
                "to treat a missing load-command dump as an empty one".format(
                    "" if name is None else "-arch {} ".format(name),
                    flag,
                    entry["arch"] or entry["index"],
                    binary,
                )
            )
        diagnostics = otool_diagnostics(completed.stdout)
        if diagnostics:
            raise SystemExit(
                "otool reported '{}' for a slice the container declares as "
                "Mach-O: {}".format(diagnostics[0], binary)
            )
        outputs.append(completed.stdout)
    return "\n".join(outputs)


def read_otool_dependencies(binary: Path) -> list[str]:
    container = read_macho_container(binary)
    if container["format"] == "invalid":
        raise SystemExit(
            "cannot read Mach-O container '{}': {}".format(binary, container["error"])
        )
    return parse_otool_dependencies(_read_slice_load_commands(binary, container, "-L"))


def read_otool_rpaths(binary: Path) -> list[str]:
    container = read_macho_container(binary)
    if container["format"] == "invalid":
        raise SystemExit(
            "cannot read Mach-O container '{}': {}".format(binary, container["error"])
        )
    return parse_otool_rpaths(_read_slice_load_commands(binary, container, "-l"))


def read_install_names(binary: Path) -> list[str]:
    """Return every distinct LC_ID_DYLIB value of ``binary``.

    ``otool -D`` prints one section per architecture, so a universal binary can
    legitimately carry two different install names (measured: the
    ``google/_upb`` wheels embed a per-arch ``bazel-out/...`` id).  Reporting
    only the first slice hides half of the defects, so every Mach-O slice is
    queried explicitly.

    Only ``MH_DYLIB`` slices can carry an LC_ID_DYLIB; object files (``.o``)
    and executables report none by definition, which is decided from the slice
    header instead of from an empty tool answer.
    """
    container = read_macho_container(binary)
    if container["format"] == "invalid":
        return parse_install_names(run_command([OTOOL, "-D", binary]).stdout)
    dylib_slices = [
        entry
        for entry in container_macho_slices(container)
        if entry["filetype"] == MH_DYLIB
    ]
    if not dylib_slices:
        # Still prove the container is queryable/consistent (raises on mixed
        # containers) without inventing an install name for a non-dylib.
        _macho_slice_queries(binary, container)
        return []
    output = _read_slice_load_commands(
        binary, container, "-D", selector=lambda entry: entry["filetype"] == MH_DYLIB
    )
    return parse_install_names(output)


def read_install_name(binary: Path):
    """Return the first LC_ID_DYLIB value, or ``None`` when the file has none.

    ``otool -L`` prints the file's own install name as the first entry for a
    dylib, before its dependencies.  Treating it as a dependency produces
    phantom "absolute dependency" findings (the delocated ``/DLC/...`` ids in
    third-party wheels), so the sanitizer and the audit both drop it here.
    """
    names = read_install_names(binary)
    return names[0] if names else None


def read_linked_dependencies(binary: Path) -> list[str]:
    """Dependencies of ``binary`` with every own install name removed."""
    install_names = set(read_install_names(binary))
    if not install_names:
        return read_otool_dependencies(binary)
    return [
        dependency
        for dependency in read_otool_dependencies(binary)
        if dependency not in install_names
    ]


def lipo_archs(binary: Path):
    completed = run_command([LIPO, "-archs", binary], check=False)
    if completed.returncode != 0:
        return None
    return [arch for arch in completed.stdout.split() if arch]


def codesign_verify(path: Path):
    """Return ``(ok, message)`` for ``codesign --verify --strict``."""
    completed = run_command([CODESIGN, "--verify", "--strict", path], check=False)
    message = (completed.stderr or completed.stdout).strip()
    return completed.returncode == 0, message


def codesign_signature_kind(path: Path):
    """Return the signature kind recorded by ``codesign -dvvv`` (``adhoc`` etc.).

    An ``adhoc`` signature is the linker's identity-less signature: it is
    invalidated by *any* byte change, including a relocation or prefix rewrite
    performed by a different tool. A real identity ("Developer ID
    Application" and friends) must never be replaced silently, so the sanitizer
    consults this value before re-signing a binary it did not modify itself.
    """
    completed = run_command([CODESIGN, "-dvvv", path], check=False)
    text = "{}\n{}".format(completed.stdout or "", completed.stderr or "")
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("Signature="):
            return line.split("=", 1)[1].strip()
    return None


def _plan_signature_repairs(binaries):
    """Classify pre-existing signature damage before any payload mutation."""
    repairs = []
    for relative, binary in binaries:
        ok, message = codesign_verify(binary)
        if ok:
            continue
        if "not signed at all" in message or "code object is not signed" in message:
            # Preserve unsigned code as a visible audit warning.
            continue
        if "invalid signature" not in message and "code or signature have been modified" not in message:
            raise SystemExit(
                "'{}' could not be verified ({}); refusing to guess whether the "
                "payload is intact".format(relative, message.replace("\n", " | ")[:200])
            )
        kind = codesign_signature_kind(binary)
        if kind != "adhoc":
            raise SystemExit(
                "'{}' carries a '{}' signature that no longer matches its content "
                "({}); refusing to replace a real identity".format(
                    relative, kind or "unknown", message.replace("\n", " | ")[:160]
                )
            )
        repairs.append((relative, binary))
    return repairs


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def is_macho(path: Path) -> bool:
    """True only when the file really contains at least one Mach-O slice.

    A universal container whose slices are LLVM bitcode (python.org's bundled
    ``python.o``) has a Mach-O *container* magic but no load commands at all;
    treating it as Mach-O is what made the 2.0.38 ARM64 release run call
    ``install_name_tool -id`` on it.
    """
    container = read_macho_container(path)
    return container["format"] != "invalid" and bool(container_macho_slices(container))


def scan_macho_payload(root: Path):
    """Classify every file below ``root`` that carries a Mach-O container magic.

    Returns a dict with four lists of ``(relative_path, real_path, ...)``:

    ``macho``      every slice is Mach-O: readable and rewritable load commands
    ``non_macho``  no slice is Mach-O (LLVM bitcode objects): no load commands
    ``mixed``      some slices are Mach-O and some are not: fail closed
    ``invalid``    container magic with a broken/truncated header

    Symlinked payload files are visited once, through their real target, so a
    payload that ships the same binary twice is not rewritten twice.
    """
    root = Path(root).resolve()
    result = {"macho": [], "non_macho": [], "mixed": [], "invalid": []}
    seen = set()
    for path in sorted(root.rglob("*")):
        if path.is_dir() and not path.is_symlink():
            continue
        try:
            real = path.resolve(strict=True)
        except OSError:
            continue
        if not real.is_file():
            continue
        key = str(real)
        if key in seen:
            continue
        try:
            with open(real, "rb") as handle:
                magic = handle.read(4)
        except OSError:
            continue
        if magic not in MACHO_MAGICS:
            continue
        seen.add(key)
        relative = path.relative_to(root).as_posix()
        container = read_macho_container(real)
        if container["format"] == "invalid":
            result["invalid"].append((relative, real, container))
            continue
        macho_slices = container_macho_slices(container)
        other_slices = container_non_macho_slices(container)
        if macho_slices and other_slices:
            result["mixed"].append((relative, real, container))
        elif macho_slices:
            result["macho"].append((relative, real, container))
        else:
            result["non_macho"].append((relative, real, container))
    return result


def iter_macho(root: Path):
    """Yield ``(relative_path, real_path)`` for every Mach-O file under root.

    Every Mach-O is covered, not only ``*.so``/``*.dylib``: the bundled
    ``lib/Resources/Python.app/Contents/MacOS/Python`` launcher and the
    ``bin/python3`` executable carry their own LC_RPATH list, and a stale
    host rpath there is just as non-portable as one inside a dylib.

    LLVM-bitcode objects and mixed containers are excluded: they have no
    verifiable load commands for the slice we could not parse.
    """
    return [(relative, real) for relative, real, _ in scan_macho_payload(root)["macho"]]


def is_inside(path: Path, root: Path) -> bool:
    try:
        Path(path).resolve().relative_to(Path(root).resolve())
        return True
    except ValueError:
        return False


def payload_symlinks(root):
    """Return ``(escaping, dangling)`` symlink lists for a payload root."""
    root = Path(root).resolve()
    escaping = []
    dangling = []
    for path in sorted(root.rglob("*")):
        if not path.is_symlink():
            continue
        try:
            target = path.resolve(strict=True)
        except OSError:
            dangling.append(path)
            continue
        if not is_inside(target, root):
            escaping.append((path, target))
    return escaping, dangling


# ---------------------------------------------------------------------------
# Audit
# ---------------------------------------------------------------------------


def is_system_reference(value: str) -> bool:
    return value in ("/usr/lib", "/System", "/usr/libexec") or value.startswith(SYSTEM_PREFIXES)


def payload_file_index(root: Path):
    """Return ``{relative posix path}`` for every file below ``root``.

    Used to decide whether an absolute path recorded in a load command names a
    file of *this* payload.  Anchoring detection on the payload layout (rather
    than on a prefix such as ``/Users``) is what keeps the audit honest when
    the build root lives on a scratch volume like ``/Volumes/UltraDisk``.
    """
    root = Path(root).resolve()
    index = set()
    for path in root.rglob("*"):
        if path.is_dir() and not path.is_symlink():
            continue
        index.add(path.relative_to(root).as_posix())
    return index


def classify_absolute_reference(reference: str, root: Path, index) -> dict:
    """Classify an absolute load-command path against the audited payload.

    Returns ``{"kind": ..., ...}`` with ``kind`` one of:

    ``not_absolute``  not an absolute path (``@rpath/...``, ``@loader_path/...``)
    ``inside``        resolves to an existing file inside the payload root
    ``suffix``        names a payload file through a foreign root prefix; the
                      tail matches exactly one path inside the payload
    ``ambiguous``     the tail matches more than one payload file: never guess
    ``external``      no payload file matches; the reference is dead/host-only

    Matching requires at least two path components in the tail, so a bare
    basename such as ``libomp.dylib`` can never be "found" in the payload by
    accident.
    """
    if not reference.startswith("/"):
        return {"kind": "not_absolute"}

    root = Path(root).resolve()
    candidate = Path(reference)
    try:
        resolved = candidate.resolve(strict=True)
    except OSError:
        resolved = None
    if resolved is not None and resolved.is_file() and is_inside(resolved, root):
        return {
            "kind": "inside",
            "target": str(resolved),
            "relative": resolved.relative_to(root).as_posix(),
        }

    parts = [part for part in reference.split("/") if part]
    matches = set()
    for offset in range(1, len(parts) - 1):
        relative = "/".join(parts[offset:])
        if relative in index:
            matches.add(relative)
    if len(matches) > 1:
        return {"kind": "ambiguous", "candidates": sorted(matches)}
    if matches:
        relative = matches.pop()
        return {"kind": "suffix", "relative": relative, "target": str(root / relative)}
    return {"kind": "external"}


def _resolve_loader_reference(binary: Path, reference: str):
    relative = reference[len("@loader_path/") :]
    return (binary.parent / relative).resolve(strict=False)


def _resolve_rpath_entries(binary: Path, rpaths, executable_dir=None):
    directories = []
    for entry in rpaths:
        if entry == "@loader_path":
            directories.append(binary.parent)
        elif entry.startswith("@loader_path/"):
            directories.append(binary.parent / entry[len("@loader_path/") :])
        elif executable_dir is not None and entry == "@executable_path":
            directories.append(Path(executable_dir))
        elif executable_dir is not None and entry.startswith("@executable_path/"):
            directories.append(Path(executable_dir) / entry[len("@executable_path/") :])
        elif entry.startswith("/"):
            directories.append(Path(entry))
    return directories


def _resolve_direct_rpath(binary, reference, rpaths):
    """Resolve only the binary's declared runpaths, never a basename search.

    Inherited runpaths and the host executable are unknown to a static audit;
    unresolved references remain closure findings rather than claimed matches.
    """
    name = reference[len("@rpath/") :]
    for directory in _resolve_rpath_entries(binary, rpaths):
        candidate = (directory / name).resolve(strict=False)
        if candidate.is_file():
            return candidate
    return None


def audit_runtime(root, expect_arch=None, strict_closure=False):
    """Return an audit report dict with ``violations`` and ``warnings``.

    A violation is a concrete portability or integrity defect that the
    companion installer, the release pipeline or the user would hit.  A
    warning is a pre-existing property of third-party wheels that this tool
    does not modify (unsigned ``.so`` covered by app-bundle signing, inactive
    torchcodec FFmpeg variants); with ``strict_closure`` those become
    violations too.
    """
    root = Path(root).resolve()
    if not root.is_dir():
        raise SystemExit("audit root is not a directory: {}".format(root))

    scan = scan_macho_payload(root)
    binaries = [(relative, real) for relative, real, _ in scan["macho"]]
    file_index = payload_file_index(root)
    violations = []
    warnings = []
    signed_checked = 0

    for relative, _, container in scan["invalid"]:
        violations.append(
            {
                "kind": "malformed_macho_container",
                "path": relative,
                "detail": container["error"],
            }
        )
    for relative, _, container in scan["mixed"]:
        violations.append(
            {
                "kind": "mixed_slice_macho",
                "path": relative,
                "detail": "slices are {}; load commands cannot be verified for "
                "the non-Mach-O slice".format(describe_slice_kinds(container)),
            }
        )
    for relative, _, container in scan["non_macho"]:
        # python.org's universal2 ``python.o`` is a fat container of LLVM
        # bitcode: an inert data object with no load commands and no install
        # name.  It must not be audited as code, but it must not be hidden
        # either, so it is disclosed as a warning.
        warnings.append(
            {
                "kind": "non_macho_container",
                "path": relative,
                "detail": "{}; no load commands".format(describe_slice_kinds(container)),
            }
        )

    def violation(kind, path, detail):
        violations.append(
            {
                "kind": kind,
                "path": path.relative_to(root).as_posix() if is_inside(path, root) else str(path),
                "detail": detail,
            }
        )

    def closure_finding(kind, path, detail):
        entry = {
            "kind": kind,
            "path": path.relative_to(root).as_posix() if is_inside(path, root) else str(path),
            "detail": detail,
        }
        if strict_closure:
            violations.append(entry)
        else:
            warnings.append(entry)

    for relative, binary in binaries:
        archs = lipo_archs(binary)
        if archs is None:
            violation("not_macho", binary, "lipo could not read the Mach-O header")
            continue
        if expect_arch and expect_arch not in archs:
            violation(
                "arch_mismatch",
                binary,
                "expected {} but found {}".format(expect_arch, " ".join(archs)),
            )

        try:
            dependencies = read_linked_dependencies(binary)
            rpaths = read_otool_rpaths(binary)
            install_names = read_install_names(binary)
        except SystemExit:
            violation("unreadable_load_commands", binary, "-")
            continue

        def reference_detail(reference, classified):
            if classified["kind"] == "inside":
                return "{} (resolves inside the payload: {})".format(
                    reference, classified["relative"]
                )
            if classified["kind"] == "suffix":
                return "{} (foreign root for payload path: {})".format(
                    reference, classified["relative"]
                )
            return reference

        for install_name in install_names:
            if install_name.startswith(HOST_PREFIXES):
                violation("host_install_name", binary, install_name)
                continue
            if install_name.startswith("/"):
                if install_name.startswith(SYSTEM_PREFIXES):
                    continue
                classified = classify_absolute_reference(install_name, root, file_index)
                if classified["kind"] in ("inside", "suffix"):
                    violation(
                        "absolute_runtime_reference",
                        binary,
                        "id " + reference_detail(install_name, classified),
                    )
                elif classified["kind"] == "ambiguous":
                    violation(
                        "ambiguous_runtime_reference",
                        binary,
                        "id {} matches {}".format(
                            install_name, ", ".join(classified["candidates"])
                        ),
                    )
                else:
                    # Dead build-machine / delocation prefix (/DLC/...,
                    # /opt/llvm-openmp/...).  Nothing resolves through it, but
                    # shipping it would keep a non-portable absolute path in the
                    # payload and would hide a future real dependency behind a
                    # warning.
                    violation("non_portable_install_name", binary, install_name)
                continue
            if not install_name.startswith(("@rpath/", "@loader_path/", "@executable_path/")):
                violation("non_portable_install_name", binary, install_name)

        for dependency in dependencies:
            if dependency.startswith(HOST_PREFIXES):
                violation("host_dependency", binary, dependency)
                continue
            if dependency in ("", "self"):
                continue
            if dependency.startswith("/"):
                if dependency.startswith(SYSTEM_PREFIXES):
                    continue
                classified = classify_absolute_reference(dependency, root, file_index)
                if classified["kind"] in ("inside", "suffix"):
                    violation(
                        "absolute_runtime_reference",
                        binary,
                        reference_detail(dependency, classified),
                    )
                elif classified["kind"] == "ambiguous":
                    violation(
                        "ambiguous_runtime_reference",
                        binary,
                        "dependency {} matches {}".format(
                            dependency, ", ".join(classified["candidates"])
                        ),
                    )
                else:
                    # A dependency must be openable at load time: an absolute
                    # non-system path that is not in the payload is dead on a
                    # user machine.  Fail closed.
                    violation("absolute_dependency", binary, dependency)
                continue
            if dependency.startswith("@loader_path/"):
                resolved = _resolve_loader_reference(binary, dependency)
                if not resolved.is_file():
                    closure_finding("unresolved_loader_dependency", binary, dependency)
                elif not is_inside(resolved, root):
                    violation("outside_payload_dependency", binary, dependency)
                continue
            if dependency.startswith("@executable_path/"):
                closure_finding("unknown_executable_dependency", binary, dependency)
                continue
            if dependency.startswith("@rpath/"):
                resolved = _resolve_direct_rpath(binary, dependency, rpaths)
                if resolved is None:
                    closure_finding("unresolved_rpath_dependency", binary, dependency)
                elif not is_inside(resolved, root):
                    violation("outside_payload_dependency", binary, dependency)
                continue
            warnings.append(
                {
                    "kind": "unrecognized_dependency",
                    "path": relative,
                    "detail": dependency,
                }
            )

        for rpath in rpaths:
            if rpath.startswith(HOST_PREFIXES):
                violation("host_rpath", binary, rpath)
            elif rpath.startswith("/") and not is_system_reference(rpath):
                # Wheel-built extensions (sklearn, Pillow) keep absolute
                # build-machine rpaths such as /Users/runner/... .  A portable
                # payload must not resolve through any absolute path.
                violation("absolute_rpath", binary, rpath)

        for reference in dependencies + rpaths + install_names:
            for marker in LOAD_COMMAND_PATH_MARKERS:
                if marker in reference:
                    violation("personal_path_reference", binary, reference)

        ok, message = codesign_verify(binary)
        if ok:
            signed_checked += 1
        elif "not signed at all" in message or "code object is not signed" in message:
            warnings.append(
                {"kind": "unsigned_macho", "path": relative, "detail": "no code signature"}
            )
        else:
            violation("invalid_signature", binary, message.replace("\n", " | ")[:400])

    for bundle in sorted(root.rglob("*.app")):
        if not bundle.is_dir():
            continue
        ok, message = codesign_verify(bundle)
        if not ok:
            violation(
                "invalid_bundle_signature",
                bundle,
                message.replace("\n", " | ")[:400],
            )

    escaping, dangling = payload_symlinks(root)
    for path, target in escaping:
        violation("symlink_escapes_payload", path, str(target))
    for path in dangling:
        warnings.append(
            {
                "kind": "dangling_symlink",
                "path": path.relative_to(root).as_posix(),
                "detail": "target does not exist",
            }
        )

    return {
        "root": str(root),
        "expect_arch": expect_arch,
        "strict_closure": strict_closure,
        "binaries": len(binaries),
        "signed_valid": signed_checked,
        "violations": violations,
        "warnings": warnings,
    }


def format_audit_report(report) -> str:
    lines = [
        "pyannote portable runtime audit",
        "  root:           {}".format(report["root"]),
        "  expect arch:    {}".format(report["expect_arch"] or "-"),
        "  strict closure: {}".format(report["strict_closure"]),
        "  macho:          {}".format(report["binaries"]),
        "  signed ok:      {}".format(report["signed_valid"]),
        "  warnings:       {}".format(len(report["warnings"])),
        "  violations:     {}".format(len(report["violations"])),
    ]
    warning_counts = {}
    for item in report["warnings"]:
        warning_counts[item["kind"]] = warning_counts.get(item["kind"], 0) + 1
    for kind in sorted(warning_counts):
        lines.append("  warning {:<30} {}".format(kind, warning_counts[kind]))
    for item in report["violations"]:
        lines.append("VIOLATION {kind}: {path} -> {detail}".format(**item))
    if report["violations"]:
        lines.append("AUDIT FAILED: {} violation(s)".format(len(report["violations"])))
    else:
        lines.append("AUDIT OK")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Sanitizer
# ---------------------------------------------------------------------------


def ensure_owner_writable(path: Path) -> None:
    mode = path.stat().st_mode
    if not (mode & stat.S_IWUSR):
        path.chmod(mode | stat.S_IWUSR)


def _enclosing_app_bundles(binary: Path, root: Path):
    bundles = []
    for parent in binary.parents:
        if parent.suffix == ".app":
            bundles.append(parent)
        if parent == root:
            break
    return bundles


def is_portable_install_name(value: str) -> bool:
    """True when a recorded install name can resolve on any machine."""
    return (
        value.startswith(("@rpath/", "@loader_path/", "@executable_path/"))
        or is_system_reference(value)
    )


def _contained_dylib_identity(target: Path, binary: Path, dependency: str, ids_by_binary):
    """Verify a contained rewrite target is a real dylib; return its identities.

    dyld can only satisfy an ``LC_LOAD_DYLIB`` with an ``MH_DYLIB``.  An object
    file, an LLVM bit-code wrapper, a bundle (``.so`` extension module) or an
    executable has no loadable dylib identity, so a reference that resolves to
    one of them proves the planned rewrite is wrong even though the path
    exists.  The declared install names are read from the *pre-sanitize*
    snapshot so a pass never validates a value it has already rewritten.
    """
    container = read_macho_container(target)
    if container["error"]:
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', whose Mach-O container is "
            "unreadable ({}); refusing an unverifiable rewrite".format(
                dependency, binary, target, container["error"]
            )
        )
    macho_slices = container_macho_slices(container)
    if not macho_slices:
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', which carries no Mach-O "
            "slice ({}); dyld cannot load it".format(
                dependency, binary, target, describe_slice_kinds(container)
            )
        )
    if container_non_macho_slices(container):
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', which mixes Mach-O and "
            "non-Mach-O slices ({}); refusing an unverifiable rewrite".format(
                dependency, binary, target, describe_slice_kinds(container)
            )
        )
    identities = list(ids_by_binary.get(str(target), []))
    not_a_dylib = [
        slice_entry for slice_entry in macho_slices if slice_entry["filetype"] != MH_DYLIB
    ]
    if not_a_dylib:
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', which is a {} slice, not an "
            "MH_DYLIB (declared: {}); dyld cannot satisfy a load command with it".format(
                dependency,
                binary,
                target,
                ", ".join(sorted({entry["filetype_name"] for entry in not_a_dylib})),
                ", ".join(identities) or "none",
            )
        )
    if not identities:
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', which declares no install "
            "name; refusing an unverifiable rewrite".format(dependency, binary, target)
        )
    return identities


def resolve_internal_dependency(
    root: Path, binary: Path, dependency: str, index, ids_by_binary, planned_ids=None
):
    """Map an absolute non-system dependency to a verified file in ``root``.

    Fails closed instead of guessing.  Two provenances are accepted, and they
    are deliberately *not* equivalent:

    ``inside``
        The recorded absolute path exists inside the payload at exactly that
        path, so what is verified here is the file dyld would open: no name
        reconstruction is involved.  The target must still be a genuine
        ``MH_DYLIB`` whose declared identities are already portable or are
        scheduled for a portable rewrite in this pass.  Demanding that its
        ``LC_ID_DYLIB`` repeat the absolute path is *not* a dyld requirement
        (an absolute ``LC_LOAD_DYLIB`` is opened directly) and it rejects the
        conda layout measured on conda-forge osx-64
        ``ncurses 6.6 hcc0dc9a_1``: ``libncurses.6.dylib`` records
        ``<prefix>/lib/libtinfo.6.dylib`` while ``libtinfo.6.dylib`` declares
        ``@rpath/libtinfo.6.dylib``.  conda documents loader-relative links as
        the relocatable form (``@rpath``/``@loader_path``), so a portable
        identity and an absolute client path are a normal combination.

    ``suffix``
        The recorded path does not exist on this machine and the tail was
        reconstructed against the payload index.  That provenance is weaker,
        so the original rule stays: the target must declare the recorded
        reference as one of its install names.  A basename guess can never be
        validated by an identity it does not carry.
    """
    classified = classify_absolute_reference(dependency, root, index)
    if classified["kind"] == "ambiguous":
        raise SystemExit(
            "dependency '{}' of '{}' matches several payload files ({}); refusing "
            "to guess which one to link".format(
                dependency, binary, ", ".join(classified["candidates"])
            )
        )
    if classified["kind"] not in ("inside", "suffix"):
        raise SystemExit(
            "dependency '{}' of '{}' is an absolute path outside the payload and "
            "does not exist on this machine; the runtime cannot be repaired "
            "offline".format(dependency, binary)
        )

    target = Path(classified["target"])
    try:
        target_real = target.resolve(strict=True)
    except OSError:
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', which does not exist; "
            "refusing to link a dangling reference".format(dependency, binary, target)
        )
    if not is_inside(target_real, root):
        raise SystemExit(
            "dependency '{}' of '{}' resolves to '{}', outside the payload root "
            "'{}'; refusing a reference that escapes the runtime".format(
                dependency, binary, target_real, root
            )
        )
    if target_real != binary.resolve():
        target_ids = ids_by_binary.get(str(target_real), [])
        if classified["kind"] == "inside":
            target_ids = _contained_dylib_identity(
                target_real, binary, dependency, ids_by_binary
            )
            # A non-portable identity is repaired by this same pass (see
            # ``planned_ids``), so it is acceptable only when that rewrite is
            # actually scheduled; anything else would keep a build-machine
            # path in the shipped payload.
            unportable = [
                value for value in target_ids if not is_portable_install_name(value)
            ]
            if unportable and str(target_real) not in (planned_ids or {}):
                raise SystemExit(
                    "dependency '{}' of '{}' resolves to '{}', which declares the "
                    "non-portable install name(s) {} that no rewrite in this pass "
                    "would repair; refusing an unverifiable rewrite".format(
                        dependency, binary, target_real, ", ".join(unportable)
                    )
                )
        elif dependency not in target_ids:
            raise SystemExit(
                "dependency '{}' of '{}' resolves to '{}' but that file does not "
                "declare the reference as an install name (declared: {}); refusing "
                "an unverifiable rewrite".format(
                    dependency,
                    binary,
                    target_real,
                    ", ".join(target_ids) or "none",
                )
            )
    return target_real


# A payload that embeds a host-managed dylib pulls in that dylib's own host
# references, so one planning pass is not enough: the copy only becomes visible
# to the scanner on the next pass.  Four passes cover the measured
# ``_ssl -> libssl -> libcrypto`` chain with headroom; not converging is a hard
# failure, never a partially repaired payload.
MAX_SANITIZE_PASSES = 4


def sanitize_runtime(root):
    """Make an existing runtime tree position independent, in place.

    The caller owns the tree: this function never clones, copies or deletes
    the tree itself, and it refuses a root that looks like a personal profile.

    Rewrites absolute LC_ID_DYLIB values (build root or delocated prefix) to
    ``@rpath/<name>`` (layout-scoped for duplicate basenames) and absolute
    dependencies that point into the tree to ``@loader_path/<relative>``;
    host-managed dylibs are embedded under ``lib/embedded-dylibs`` and
    absolute/host LC_RPATH entries are removed.  System libraries and
    ``@loader_path``/``@executable_path``/``@rpath`` references are preserved.

    An embedded copy is itself a payload binary with its own load commands
    (measured: python.org's ``libssl.3.dylib`` links
    ``/Library/Frameworks/Python.framework/.../libcrypto.3.dylib`` and carries a
    host LC_RPATH).  Repairing only the client would ship a runtime that still
    fails ``validate_portable_pyannote_native_dependencies`` in the companion
    installer, so the pass is repeated until it reaches a fixpoint.  That also
    keeps a second ``sanitize`` call a byte-level no-op.
    """
    root = Path(root).resolve()
    refuse_personal_root(root)

    aggregate = {
        "root": str(root),
        "modified": [],
        "removed_rpaths": [],
        "repaired_install_names": [],
        "repaired_dependencies": [],
        "embedded_copies": [],
        "signed": [],
        "signed_bundles": [],
        "repaired_signatures": [],
        "skipped_non_macho": [],
        "passes": 0,
    }
    seen_modified = set()
    seen_skipped = set()
    embedded_sources = {}
    for pass_index in range(1, MAX_SANITIZE_PASSES + 1):
        receipt = _sanitize_pass(root, embedded_sources)
        for entry in receipt["modified"]:
            if entry not in seen_modified:
                seen_modified.add(entry)
                aggregate["modified"].append(entry)
        for key in (
            "removed_rpaths",
            "repaired_install_names",
            "repaired_dependencies",
            "embedded_copies",
            "signed",
            "signed_bundles",
            "repaired_signatures",
        ):
            aggregate[key].extend(receipt[key])
        for entry in receipt["skipped_non_macho"]:
            if entry["path"] not in seen_skipped:
                seen_skipped.add(entry["path"])
                aggregate["skipped_non_macho"].append(entry)
        if receipt["modified"]:
            aggregate["passes"] = pass_index
        if not receipt["modified"]:
            return aggregate

    raise SystemExit(
        "the sanitizer did not converge after {} passes; the payload keeps "
        "gaining host-managed references (last pass embedded {}). Refusing to "
        "ship a partially repaired runtime.".format(
            MAX_SANITIZE_PASSES, ", ".join(aggregate["embedded_copies"][-5:]) or "none"
        )
    )


def _sanitize_pass(root, embedded_sources):
    """One plan-then-mutate pass over the current payload tree.

    ``embedded_sources`` maps an embedded target path to the realpath of the
    host dylib it was copied from, and is shared across passes so a basename
    collision is detected even when the two clients are seen in different
    passes.
    """
    root = Path(root).resolve()
    refuse_personal_root(root)
    embedded_dir = root / "lib" / "embedded-dylibs"

    scan = scan_macho_payload(root)
    # Fail closed before planning a single rewrite: a container we cannot
    # classify slice by slice must never be half-repaired.
    if scan["invalid"]:
        relative, _, container = scan["invalid"][0]
        raise SystemExit(
            "'{}' has a Mach-O container magic but an unreadable header ({}); "
            "refusing to sanitize a payload this tool cannot verify".format(
                relative, container["error"]
            )
        )
    if scan["mixed"]:
        relative, _, container = scan["mixed"][0]
        raise SystemExit(
            "'{}' mixes Mach-O and non-Mach-O slices ({}); install_name_tool "
            "would act on a container this tool cannot fully verify, refusing "
            "the rewrite".format(relative, describe_slice_kinds(container))
        )
    binaries = [(relative, real) for relative, real, _ in scan["macho"]]
    skipped_non_macho = [
        {
            "path": relative,
            "format": container["format"],
            "slices": describe_slice_kinds(container),
        }
        for relative, _, container in scan["non_macho"]
    ]
    file_index = payload_file_index(root)
    # Snapshot every install name before touching anything: a dependency is
    # verified against the *original* ids of its target, not against a value
    # this run may already have rewritten.
    ids_by_binary = {str(real): read_install_names(real) for _, real in binaries}
    names = {}
    for _, binary in binaries:
        names.setdefault(binary.name, set()).add(str(binary))
    planned_ids = {}
    for relative, binary in binaries:
        if any(not is_portable_install_name(value) for value in ids_by_binary[str(binary)]):
            # Duplicate basenames must not acquire the same dyld identity.
            name = binary.name
            if len(names[name]) > 1:
                scope = hashlib.sha256(relative.encode("utf-8")).hexdigest()[:16]
                name = "sbobino-{}-{}".format(scope, name)
            planned_ids[str(binary)] = "@rpath/{}".format(name)

    # Plan first, mutate later: an unresolvable or ambiguous reference aborts
    # the run before a single load command is rewritten, so a failed repair
    # can never leave a half-sanitized, half-signed tree behind.
    plans = []
    planned_embedded_sources = dict(embedded_sources)
    for relative, binary in binaries:
        plan = {
            "relative": relative,
            "binary": binary,
            "embed": [],
            "dependencies": [],
            "install_name": None,
            "rpaths": [],
        }

        for dependency in read_linked_dependencies(binary):
            if dependency.startswith("@rpath/"):
                target = _resolve_direct_rpath(binary, dependency, read_otool_rpaths(binary))
                if target is not None and str(target) in planned_ids:
                    replacement = "@loader_path/{}".format(
                        Path(os.path.relpath(target, start=binary.parent)).as_posix()
                    )
                    plan["dependencies"].append((dependency, replacement))
                continue
            if dependency.startswith(HOST_PREFIXES):
                source = Path(dependency)
                if not source.exists():
                    raise SystemExit(
                        "host-managed dylib '{}' required by '{}' is missing on this "
                        "machine; cannot repair the runtime offline".format(dependency, binary)
                    )
                target_key = str(embedded_dir / source.name)
                source_real = os.path.realpath(source)
                previous = planned_embedded_sources.get(target_key)
                if previous is not None and previous != source_real:
                    raise SystemExit(
                        "cannot embed '{}': '{}' was already planned from '{}' "
                        "and they are different files with the same basename".format(
                            source, target_key, previous
                        )
                    )
                planned_embedded_sources[target_key] = source_real
                plan["embed"].append((dependency, source))
                continue
            if not dependency.startswith("/") or is_system_reference(dependency):
                continue
            target = resolve_internal_dependency(
                root, binary, dependency, file_index, ids_by_binary, planned_ids
            )
            replacement = "@loader_path/{}".format(
                Path(os.path.relpath(target, start=binary.parent)).as_posix()
            )
            if replacement != dependency:
                plan["dependencies"].append((dependency, replacement))

        unportable_ids = [
            install_name
            for install_name in ids_by_binary[str(binary)]
            if not is_portable_install_name(install_name)
        ]
        if unportable_ids:
            # One -id call rewrites every architecture slice; the recorded
            # receipt keeps each original value.
            plan["install_name"] = (planned_ids[str(binary)], unportable_ids)

        for entry in read_otool_rpaths(binary):
            if entry.startswith(HOST_PREFIXES):
                plan["rpaths"].append((entry, "host_managed"))
            elif entry.startswith("/") and not is_system_reference(entry):
                # Absolute rpaths baked in by wheel builds (/Users/runner/...,
                # /DLC/...) can never resolve on a user machine.  Removing
                # them keeps the payload free of build-machine paths; relative
                # @loader_path/@executable_path entries are untouched.
                plan["rpaths"].append((entry, "absolute_build_path"))

        plans.append(plan)

    # Signature refusals belong to the read-only plan too. In particular, an
    # invalid real identity must not be discovered after other files changed.
    signature_repairs = _plan_signature_repairs(binaries)
    modified = []
    removed_rpaths = []
    embedded_copies = []
    repaired_install_names = []
    repaired_dependencies = []
    embedded_planned = set()
    embedded_targets = []

    embedded_sources.update(planned_embedded_sources)
    for plan in plans:
        binary = plan["binary"]
        relative = plan["relative"]
        changed = False

        for dependency, source in plan["embed"]:
            embedded_dir.mkdir(parents=True, exist_ok=True)
            target = embedded_dir / source.name
            if target not in embedded_planned:
                embedded_planned.add(target)
                if not target.exists():
                    shutil.copy2(source, target)
                    embedded_copies.append(str(target.relative_to(root)))
                ensure_owner_writable(target)
                run_command([INSTALL_NAME_TOOL, "-id", "@rpath/{}".format(target.name), target])
                embedded_targets.append(target)
            replacement = "@loader_path/{}".format(
                Path(os.path.relpath(target, start=binary.parent)).as_posix()
            )
            ensure_owner_writable(binary)
            run_command([INSTALL_NAME_TOOL, "-change", dependency, replacement, binary])
            repaired_dependencies.append(
                {"path": relative, "dependency": dependency, "replacement": replacement}
            )
            changed = True

        for dependency, replacement in plan["dependencies"]:
            ensure_owner_writable(binary)
            run_command([INSTALL_NAME_TOOL, "-change", dependency, replacement, binary])
            repaired_dependencies.append(
                {"path": relative, "dependency": dependency, "replacement": replacement}
            )
            changed = True

        if plan["install_name"]:
            replacement, originals = plan["install_name"]
            ensure_owner_writable(binary)
            run_command([INSTALL_NAME_TOOL, "-id", replacement, binary])
            for install_name in originals:
                repaired_install_names.append(
                    {
                        "path": relative,
                        "install_name": install_name,
                        "replacement": replacement,
                    }
                )
            changed = True

        if plan["rpaths"]:
            ensure_owner_writable(binary)
        for entry, reason in plan["rpaths"]:
            # install_name_tool removes one occurrence per call; loop until the
            # command no longer reports that rpath (duplicates are legal).
            for _ in range(8):
                completed = run_command(
                    [INSTALL_NAME_TOOL, "-delete_rpath", entry, binary], check=False
                )
                if entry not in read_otool_rpaths(binary):
                    break
                if completed.returncode != 0:
                    raise SystemExit(
                        "install_name_tool could not delete rpath '{}' from '{}': {}".format(
                            entry, binary, completed.stderr.strip()
                        )
                    )
            removed_rpaths.append(
                {"path": relative, "rpath": entry, "reason": reason}
            )
            changed = True

        if changed:
            modified.append(binary)

    # Signing order: inner Mach-O first, enclosing .app bundles afterwards.
    # That is the standard order for nested code and keeps CodeResources
    # consistent.  (Measured on Python.app: re-signing only the nested
    # executable is already enough for codesign --verify, because the bundle's
    # main executable carries the bundle signature; signing the bundle is
    # therefore defensive, not load-bearing.)
    signed = []
    for binary in embedded_targets:
        run_command([CODESIGN, "--force", "--sign", "-", binary])
        signed.append(binary.relative_to(root).as_posix())
    for binary in modified:
        run_command([CODESIGN, "--force", "--sign", "-", binary])
        signed.append(binary.relative_to(root).as_posix())

    bundles = []
    for binary in embedded_targets + modified:
        for bundle in _enclosing_app_bundles(binary, root):
            if bundle not in bundles:
                bundles.append(bundle)
    for bundle in bundles:
        run_command([CODESIGN, "--force", "--sign", "-", bundle])

    for binary in embedded_targets + modified:
        ok, message = codesign_verify(binary)
        if not ok:
            raise SystemExit(
                "re-signed binary failed verification: {}: {}".format(binary, message)
            )
    for bundle in bundles:
        ok, message = codesign_verify(bundle)
        if not ok:
            raise SystemExit(
                "re-signed bundle failed verification: {}: {}".format(bundle, message)
            )

    # A relocation performed by another tool breaks an adhoc signature without
    # touching a single load command: conda's own prefix rewrite replaces the
    # build prefix inside string data, so 15 conda-forge osx-64 binaries arrive
    # with a signature that no longer matches their content (measured on the
    # 2.0.39 Intel stack).  Such a signature carries no identity to protect, and
    # the payload cannot be shipped with `codesign --verify` failing on files
    # nobody here modified, so it is re-signed adhoc.  Anything else is refused:
    # an unknown failure or a *real* identity that no longer matches must never
    # be silently replaced.
    resigned = []
    for relative, binary in signature_repairs:
        if binary in modified:
            continue
        ensure_owner_writable(binary)
        run_command([CODESIGN, "--force", "--sign", "-", binary])
        ok, message = codesign_verify(binary)
        if not ok:
            raise SystemExit(
                "re-signed '{}' still fails verification: {}".format(relative, message)
            )
        resigned.append(relative)
        modified.append(binary)
        signed.append(relative)
        for bundle in _enclosing_app_bundles(binary, root):
            if bundle not in bundles:
                bundles.append(bundle)
                run_command([CODESIGN, "--force", "--sign", "-", bundle])
                ok, message = codesign_verify(bundle)
                if not ok:
                    raise SystemExit(
                        "re-signed bundle '{}' failed verification: {}".format(bundle, message)
                    )

    return {
        "root": str(root),
        "modified": [path.relative_to(root).as_posix() for path in modified],
        "removed_rpaths": removed_rpaths,
        "repaired_install_names": repaired_install_names,
        "repaired_dependencies": repaired_dependencies,
        "embedded_copies": embedded_copies,
        "signed": signed,
        "signed_bundles": [str(bundle.relative_to(root)) for bundle in bundles],
        # Files that arrived with an adhoc signature invalidated by another
        # tool (conda's prefix rewrite) and were re-signed here.
        "repaired_signatures": resigned,
        # Containers with no Mach-O slice at all (python.org's universal2
        # ``python.o`` is fat LLVM bitcode).  They are data, never rewritten;
        # the receipt keeps them visible instead of silently skipping a file.
        "skipped_non_macho": skipped_non_macho,
    }


# ---------------------------------------------------------------------------
# Path guard
# ---------------------------------------------------------------------------


def refuse_personal_root(path):
    """Reject paths that could point at a personal Companion profile."""
    text = str(Path(path).expanduser())
    home = str(Path.home())
    for marker in PERSONAL_PATH_MARKERS:
        if marker in text:
            raise SystemExit(
                "refusing personal/lockbox path '{}' (matched '{}')".format(path, marker)
            )
    library = os.path.join(home, "Library")
    if text == home or text.startswith(library + os.sep) or text == library:
        raise SystemExit("refusing path inside the user Library: '{}'".format(path))


def guard_paths(input_path, output_path):
    input_real = Path(os.path.realpath(input_path))
    output_real = Path(os.path.realpath(output_path))
    refuse_personal_root(input_path)
    refuse_personal_root(output_path)
    if input_real == output_real:
        raise SystemExit("input and output resolve to the same path: {}".format(input_real))
    if is_inside(output_real, input_real):
        raise SystemExit(
            "output '{}' is nested inside input '{}'; refusing to write into the source".format(
                output_real, input_real
            )
        )
    if is_inside(input_real, output_real):
        raise SystemExit(
            "input '{}' is nested inside output '{}'; refusing a destructive output root".format(
                input_real, output_real
            )
        )
    return {"input": str(input_real), "output": str(output_real)}


# ---------------------------------------------------------------------------
# Inventory
# ---------------------------------------------------------------------------


def inventory_macho_metadata(path: Path):
    """Return Mach-O type metadata for an inventory entry, or ``None``.

    The manifest has to distinguish an object file from a dylib and from an
    executable, and it has to stay honest about containers whose slices are not
    Mach-O at all (python.org's universal2 ``python.o`` is fat LLVM bitcode).
    ``kind`` is ``macho``, ``llvm_bitcode``, ``mixed``, ``unknown`` or
    ``invalid``; each slice records its own architecture and Mach-O file type,
    taken from the real Mach-O header of that slice.
    """
    try:
        with open(path, "rb") as handle:
            magic = handle.read(4)
    except OSError:
        return None
    if magic not in MACHO_MAGICS:
        return None
    container = read_macho_container(path)
    if container["format"] == "invalid":
        return {"format": "invalid", "kind": "invalid", "error": container["error"], "slices": []}
    macho_slices = container_macho_slices(container)
    other_slices = container_non_macho_slices(container)
    if macho_slices and other_slices:
        kind = "mixed"
    elif macho_slices:
        kind = "macho"
    elif other_slices:
        kind = "llvm_bitcode" if all(
            entry["kind"] == "llvm_bitcode" for entry in other_slices
        ) else "unknown"
    else:
        kind = "unknown"
    return {
        "format": container["format"],
        "kind": kind,
        "archs": [entry["arch"] for entry in container["slices"]],
        "slices": [
            {
                "arch": entry["arch"],
                "kind": entry["kind"],
                "filetype": entry["filetype_name"],
            }
            for entry in container["slices"]
        ],
    }


def build_inventory(root):
    root = Path(root).resolve()
    entries = []
    total_bytes = 0
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            entries.append(
                {"path": relative, "type": "symlink", "target": os.readlink(path), "size": 0}
            )
            continue
        if path.is_dir():
            entries.append({"path": relative, "type": "dir", "size": 0})
            continue
        if not path.is_file():
            continue
        digest = hashlib.sha256()
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1 << 20), b""):
                digest.update(chunk)
        size = path.stat().st_size
        total_bytes += size
        entry = {
            "path": relative,
            "type": "file",
            "sha256": digest.hexdigest(),
            "size": size,
        }
        macho = inventory_macho_metadata(path)
        if macho is not None:
            entry["macho"] = macho
        entries.append(entry)

    rolled = hashlib.sha256()
    for entry in entries:
        rolled.update(
            "\t".join(str(entry.get(key, "")) for key in ("path", "type", "sha256", "target", "size")).encode()
        )
        rolled.update(b"\n")

    return {
        "root": str(root),
        "generated_at": datetime.datetime.now(datetime.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        "entries": len(entries),
        "files": sum(1 for entry in entries if entry["type"] == "file"),
        "symlinks": sum(1 for entry in entries if entry["type"] == "symlink"),
        "bytes": total_bytes,
        "tree_sha256": rolled.hexdigest(),
        "inventory": entries,
    }


def write_sha256sums(inventory, output_path):
    lines = []
    for entry in inventory["inventory"]:
        if entry["type"] == "file":
            lines.append("{}  {}".format(entry["sha256"], entry["path"]))
        elif entry["type"] == "symlink":
            lines.append("{}  {} -> {}".format("symlink", entry["path"], entry["target"]))
    Path(output_path).write_text("\n".join(lines) + "\n")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subparsers = parser.add_subparsers(dest="command", required=True)

    sanitize = subparsers.add_parser("sanitize")
    sanitize.add_argument("root")
    sanitize.add_argument("--json")

    audit = subparsers.add_parser("audit")
    audit.add_argument("root")
    audit.add_argument("--expect-arch", choices=sorted(TRIPLE_FOR_ARCH))
    audit.add_argument("--json")
    audit.add_argument("--quiet", action="store_true")
    audit.add_argument(
        "--strict-closure",
        action="store_true",
        help="treat absolute non-system and unresolved @rpath references as violations",
    )

    inventory = subparsers.add_parser("inventory")
    inventory.add_argument("root")
    inventory.add_argument("output")
    inventory.add_argument("--sums")

    guard = subparsers.add_parser("guard-paths")
    guard.add_argument("input")
    guard.add_argument("output")

    guard_root = subparsers.add_parser("guard-root")
    guard_root.add_argument("path")

    args = parser.parse_args(argv)

    if args.command == "sanitize":
        receipt = sanitize_runtime(args.root)
        if args.json:
            Path(args.json).write_text(json.dumps(receipt, indent=2) + "\n")
        print(
            "sanitized {} Mach-O, removed {} rpath(s), repaired {} install "
            "name(s) and {} dependency reference(s), copied {} dylib(s), signed "
            "{} binary/binaries and {} bundle(s), re-signed {} adhoc signature(s) "
            "invalidated by another tool, skipped {} non-Mach-O container(s)".format(
                len(receipt["modified"]),
                len(receipt["removed_rpaths"]),
                len(receipt["repaired_install_names"]),
                len(receipt["repaired_dependencies"]),
                len(receipt["embedded_copies"]),
                len(receipt["signed"]),
                len(receipt["signed_bundles"]),
                len(receipt["repaired_signatures"]),
                len(receipt["skipped_non_macho"]),
            )
        )
        return 0

    if args.command == "audit":
        report = audit_runtime(
            args.root, expect_arch=args.expect_arch, strict_closure=args.strict_closure
        )
        if args.json:
            Path(args.json).write_text(json.dumps(report, indent=2) + "\n")
        if not args.quiet:
            print(format_audit_report(report))
        return 1 if report["violations"] else 0

    if args.command == "inventory":
        inventory_data = build_inventory(args.root)
        Path(args.output).write_text(json.dumps(inventory_data, indent=2) + "\n")
        if args.sums:
            write_sha256sums(inventory_data, args.sums)
        print(
            "{} entries, {} files, {} bytes, tree_sha256 {}".format(
                inventory_data["entries"],
                inventory_data["files"],
                inventory_data["bytes"],
                inventory_data["tree_sha256"],
            )
        )
        return 0

    if args.command == "guard-paths":
        print(json.dumps(guard_paths(args.input, args.output)))
        return 0

    if args.command == "guard-root":
        refuse_personal_root(args.path)
        # Resolve existing symlink parents even when the leaf does not exist;
        # this command runs before the staging shell creates any directory.
        refuse_personal_root(Path(args.path).resolve(strict=False))
        print(json.dumps({"root": str(Path(args.path))}))
        return 0

    return 2


if __name__ == "__main__":
    sys.exit(main())
