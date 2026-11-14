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
    Remove host-managed and absolute build-machine LC_RPATH entries and
    host-managed dylib dependencies from a runtime tree in place (the caller owns the tree; this tool never
    clones or copies), then re-sign every modified Mach-O and the enclosing
    ``.app`` bundles, in that order.

``pyannote_portable_runtime.py audit <runtime-root> [--expect-arch ARCH] [--json PATH] [--strict-closure]``
    Strict audit: architecture, host-managed dependency/rpath/install-name
    references, signature validity for signed Mach-O, enclosing ``.app``
    seals and personal-path substrings in load commands.  Exit status 1 on
    violations.

    Absolute non-system references and unresolved ``@rpath`` dependencies are
    reported as warnings by default and promoted to violations with
    ``--strict-closure``.  Rationale, measured on the shipped asset: the
    third-party wheels carry delocated LC_ID values (``/DLC/...``) and
    torchcodec ships parallel ``libtorchcodec_*N`` variants prebuilt against
    FFmpeg major versions that are not bundled (only the active variant is
    imported).  Refusing those by default would block every release for a
    reason the app does not reject at install time and that cannot be fixed
    without replacing the wheels.

``pyannote_portable_runtime.py inventory <root> <output.json>``
    Deterministic inventory (path, type, sha256, size, symlink target) plus a
    rolled-up tree hash, used for the payload manifest.

``pyannote_portable_runtime.py guard-paths <input> <output>``
    Path guard for the repair tool: rejects personal profile paths, an output
    that is the input, and outputs nested inside the input (or vice versa).
    Prints the canonical paths as JSON.

The module stays Python 3.9 compatible because the release scripts call
whatever ``python3`` is first on PATH on the build machine.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
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

# Thin and fat Mach-O magics, both endiannesses.
MACHO_MAGICS = {
    b"\xcf\xfa\xed\xfe",
    b"\xce\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
    b"\xbe\xba\xfe\xca",
    b"\xfe\xed\xfa\xcf",
    b"\xfe\xed\xfa\xce",
}

ARCH_FOR_TRIPLE = {
    "aarch64-apple-darwin": "arm64",
    "x86_64-apple-darwin": "x86_64",
}
TRIPLE_FOR_ARCH = {value: key for key, value in ARCH_FOR_TRIPLE.items()}


# ---------------------------------------------------------------------------
# otool parsing (single copy for every caller)
# ---------------------------------------------------------------------------


def parse_otool_dependencies(output: str) -> list[str]:
    """Return the linked-library paths from ``otool -L`` output."""
    refs = []
    for line in output.splitlines()[1:]:
        stripped = line.strip()
        if not stripped:
            continue
        ref = stripped.split(" (", 1)[0].split(" ", 1)[0].strip()
        if ref:
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
            refs.append(stripped.split("path ", 1)[1].split(" (offset ", 1)[0])
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


def read_otool_dependencies(binary: Path) -> list[str]:
    return parse_otool_dependencies(run_command([OTOOL, "-L", binary]).stdout)


def read_otool_rpaths(binary: Path) -> list[str]:
    return parse_otool_rpaths(run_command([OTOOL, "-l", binary]).stdout)


def read_install_name(binary: Path):
    """Return the LC_ID_DYLIB value, or ``None`` when the file has none.

    ``otool -L`` prints the file's own install name as the first entry for a
    dylib, before its dependencies.  Treating it as a dependency produces
    phantom "absolute dependency" findings (the delocated ``/DLC/...`` ids in
    third-party wheels), so the sanitizer and the audit both drop it here.
    """
    completed = run_command([OTOOL, "-D", binary], check=False)
    if completed.returncode != 0:
        return None
    for line in completed.stdout.splitlines()[1:]:
        stripped = line.strip()
        if stripped:
            return stripped.split(" (", 1)[0].strip()
    return None


def read_linked_dependencies(binary: Path) -> list[str]:
    """Dependencies of ``binary`` with its own install name removed."""
    dependencies = read_otool_dependencies(binary)
    install_name = read_install_name(binary)
    if install_name and dependencies and dependencies[0] == install_name:
        dependencies = dependencies[1:]
    return dependencies


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


# ---------------------------------------------------------------------------
# Discovery
# ---------------------------------------------------------------------------


def is_macho(path: Path) -> bool:
    try:
        with open(path, "rb") as handle:
            header = handle.read(4)
    except OSError:
        return False
    return header in MACHO_MAGICS


def iter_macho(root: Path):
    """Yield ``(relative_path, real_path)`` for every Mach-O file under root.

    Every Mach-O is covered, not only ``*.so``/``*.dylib``: the bundled
    ``lib/Resources/Python.app/Contents/MacOS/Python`` launcher and the
    ``bin/python3`` executable carry their own LC_RPATH list, and a stale
    host rpath there is just as non-portable as one inside a dylib.
    """
    root = Path(root).resolve()
    seen = set()
    results = []
    for path in sorted(root.rglob("*")):
        if path.is_dir() and not path.is_symlink():
            continue
        try:
            real = path.resolve(strict=True)
        except OSError:
            continue
        if not real.is_file() or not is_macho(real):
            continue
        key = str(real)
        if key in seen:
            continue
        seen.add(key)
        results.append((path.relative_to(root).as_posix(), real))
    return results


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


def _resolve_loader_reference(binary: Path, reference: str):
    relative = reference[len("@loader_path/") :]
    return (binary.parent / relative).resolve(strict=False)


def _resolve_rpath_entries(binary: Path, rpaths):
    directories = []
    for entry in rpaths:
        if entry.startswith("@loader_path/"):
            directories.append((binary.parent / entry[len("@loader_path/") :]))
        elif entry.startswith("@executable_path/"):
            directories.append(binary.parent / entry[len("@executable_path/") :])
        elif entry.startswith("/"):
            directories.append(Path(entry))
    return directories


def audit_runtime(root, expect_arch=None, strict_closure=False):
    """Return an audit report dict with ``violations`` and ``warnings``.

    A violation is a concrete portability or integrity defect that the
    companion installer, the release pipeline or the user would hit.  A
    warning is a pre-existing property of third-party wheels that this tool
    does not modify (unsigned ``.so`` covered by app-bundle signing,
    delocated install names, inactive torchcodec FFmpeg variants); with
    ``strict_closure`` those become violations too.
    """
    root = Path(root).resolve()
    if not root.is_dir():
        raise SystemExit("audit root is not a directory: {}".format(root))

    binaries = iter_macho(root)
    by_name = {}
    for _, real in binaries:
        by_name.setdefault(real.name, []).append(real)

    violations = []
    warnings = []
    signed_checked = 0

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
            install_name = read_install_name(binary)
        except SystemExit:
            violation("unreadable_load_commands", binary, "-")
            continue

        if install_name:
            if install_name.startswith(HOST_PREFIXES):
                violation("host_install_name", binary, install_name)
            elif install_name.startswith("/") and not install_name.startswith(SYSTEM_PREFIXES):
                closure_finding("non_system_install_name", binary, install_name)

        for dependency in dependencies:
            if dependency.startswith(HOST_PREFIXES):
                violation("host_dependency", binary, dependency)
                continue
            if dependency in ("", "self"):
                continue
            if dependency.startswith("/"):
                if dependency.startswith(SYSTEM_PREFIXES):
                    continue
                if is_inside(Path(dependency), root) and Path(dependency).is_file():
                    continue
                closure_finding("absolute_dependency", binary, dependency)
                continue
            if dependency.startswith("@loader_path/"):
                resolved = _resolve_loader_reference(binary, dependency)
                if not resolved.is_file():
                    closure_finding("unresolved_loader_dependency", binary, dependency)
                elif not is_inside(resolved, root):
                    violation("outside_payload_dependency", binary, dependency)
                continue
            if dependency.startswith("@executable_path/"):
                continue
            if dependency.startswith("@rpath/"):
                name = dependency[len("@rpath/") :]
                resolved = None
                for directory in _resolve_rpath_entries(binary, rpaths):
                    candidate = (directory / name).resolve(strict=False)
                    if candidate.is_file():
                        resolved = candidate
                        break
                if resolved is None and by_name.get(name):
                    resolved = by_name[name][0]
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

        for reference in dependencies + rpaths + ([install_name] if install_name else []):
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


def sanitize_runtime(root):
    """Remove host-managed references from an existing runtime tree.

    The caller owns the tree: this function never clones, copies or deletes
    the tree itself, and it refuses a root that looks like a personal profile.
    """
    root = Path(root).resolve()
    refuse_personal_root(root)
    embedded_dir = root / "lib" / "embedded-dylibs"

    modified = []
    removed_rpaths = []
    embedded_copies = []
    repaired_install_names = []

    for _, binary in iter_macho(root):
        changed = False

        install_name = read_install_name(binary)
        if install_name and install_name.startswith(HOST_PREFIXES):
            ensure_owner_writable(binary)
            run_command(
                [INSTALL_NAME_TOOL, "-id", "@rpath/{}".format(binary.name), binary]
            )
            repaired_install_names.append(
                {"path": binary.relative_to(root).as_posix(), "install_name": install_name}
            )
            changed = True
        for dependency in read_linked_dependencies(binary):
            if not dependency.startswith(HOST_PREFIXES):
                continue
            source = Path(dependency)
            if not source.exists():
                raise SystemExit(
                    "host-managed dylib '{}' required by '{}' is missing on this "
                    "machine; cannot repair the runtime offline".format(dependency, binary)
                )
            embedded_dir.mkdir(parents=True, exist_ok=True)
            target = embedded_dir / source.name
            if not target.exists():
                shutil.copy2(source, target)
                embedded_copies.append(str(target.relative_to(root)))
            ensure_owner_writable(target)
            run_command([INSTALL_NAME_TOOL, "-id", "@rpath/{}".format(target.name), target])
            relative = os.path.relpath(target, start=binary.parent).replace(os.sep, "/")
            ensure_owner_writable(binary)
            run_command(
                [INSTALL_NAME_TOOL, "-change", dependency, "@loader_path/{}".format(relative), binary]
            )
            changed = True

        removable_rpaths = []
        for entry in read_otool_rpaths(binary):
            if entry.startswith(HOST_PREFIXES):
                removable_rpaths.append((entry, "host_managed"))
            elif entry.startswith("/") and not is_system_reference(entry):
                # Absolute rpaths baked in by wheel builds (/Users/runner/...,
                # /DLC/...) can never resolve on a user machine.  Removing
                # them keeps the payload free of build-machine paths; relative
                # @loader_path/@executable_path entries are untouched.
                removable_rpaths.append((entry, "absolute_build_path"))
        if removable_rpaths:
            ensure_owner_writable(binary)
        for entry, reason in removable_rpaths:
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
                {"path": binary.relative_to(root).as_posix(), "rpath": entry, "reason": reason}
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
    for binary in modified:
        run_command([CODESIGN, "--force", "--sign", "-", binary])
        signed.append(binary.relative_to(root).as_posix())

    bundles = []
    for binary in modified:
        for bundle in _enclosing_app_bundles(binary, root):
            if bundle not in bundles:
                bundles.append(bundle)
    for bundle in bundles:
        run_command([CODESIGN, "--force", "--sign", "-", bundle])

    for binary in modified:
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

    return {
        "root": str(root),
        "modified": [path.relative_to(root).as_posix() for path in modified],
        "removed_rpaths": removed_rpaths,
        "repaired_install_names": repaired_install_names,
        "embedded_copies": embedded_copies,
        "signed": signed,
        "signed_bundles": [str(bundle.relative_to(root)) for bundle in bundles],
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
        entries.append(
            {
                "path": relative,
                "type": "file",
                "sha256": digest.hexdigest(),
                "size": size,
            }
        )

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

    args = parser.parse_args(argv)

    if args.command == "sanitize":
        receipt = sanitize_runtime(args.root)
        if args.json:
            Path(args.json).write_text(json.dumps(receipt, indent=2) + "\n")
        print(
            "sanitized {} Mach-O, removed {} rpath(s), repaired {} install "
            "name(s), copied {} dylib(s), signed {} binary/binaries and {} bundle(s)".format(
                len(receipt["modified"]),
                len(receipt["removed_rpaths"]),
                len(receipt["repaired_install_names"]),
                len(receipt["embedded_copies"]),
                len(receipt["signed"]),
                len(receipt["signed_bundles"]),
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

    return 2


if __name__ == "__main__":
    sys.exit(main())
