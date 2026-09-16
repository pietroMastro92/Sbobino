#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
from datetime import datetime, timezone


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_manifest(output: pathlib.Path) -> int:
    root = pathlib.Path(__file__).resolve().parents[2]
    listed = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")
    paths = sorted(
        path.decode("utf-8")
        for path in listed
        if path
        and (
            path.startswith(b".github/")
            or path.startswith(b"sbobino_desktop/")
        )
        and not path.startswith(b"sbobino_desktop/validation-evidence/")
    )
    files = []
    aggregate = hashlib.sha256()
    for relative in paths:
        path = root / relative
        if not path.is_file():
            continue
        digest = sha256_file(path)
        files.append({"path": relative, "bytes": path.stat().st_size, "sha256": digest})
        aggregate.update(f"{digest}  {relative}\n".encode())

    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True
    ).stdout.strip()
    status_lines = subprocess.run(
        [
            "git",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".github",
            "sbobino_desktop",
        ],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    dirty_paths = [
        line
        for line in status_lines
        if "sbobino_desktop/validation-evidence/" not in line
    ]
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "generated_at": datetime.now(timezone.utc).isoformat(),
                "git_head": head,
                "dirty": bool(dirty_paths),
                "dirty_paths": dirty_paths,
                "platform": platform.platform(),
                "architecture": platform.machine(),
                "aggregate_sha256": aggregate.hexdigest(),
                "files": files,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return 0


def write_artifact_manifest(output: pathlib.Path, requested: list[pathlib.Path]) -> int:
    files: list[pathlib.Path] = []
    missing: list[str] = []
    for requested_path in requested:
        path = requested_path.resolve()
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            files.extend(candidate for candidate in path.rglob("*") if candidate.is_file())
        else:
            missing.append(str(requested_path))
    records = [
        {
            "path": str(path),
            "bytes": path.stat().st_size,
            "sha256": sha256_file(path),
        }
        for path in sorted(set(files))
    ]
    aggregate = hashlib.sha256()
    for record in records:
        aggregate.update(f"{record['sha256']}  {record['path']}\n".encode())
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "generated_at": datetime.now(timezone.utc).isoformat(),
                "platform": platform.platform(),
                "architecture": platform.machine(),
                "aggregate_sha256": aggregate.hexdigest(),
                "missing": missing,
                "files": records,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return 1 if missing else 0


def capture(output: pathlib.Path, command: list[str]) -> int:
    started = datetime.now(timezone.utc)
    executable = shutil.which(command[0]) or command[0]
    command = [executable, *command[1:]]
    completed = subprocess.run(command, capture_output=True, text=True, errors="replace")
    finished = datetime.now(timezone.utc)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "started_at": started.isoformat(),
                "finished_at": finished.isoformat(),
                "command": command,
                "cwd": os.getcwd(),
                "platform": platform.platform(),
                "architecture": platform.machine(),
                "exit_code": completed.returncode,
                "stdout": completed.stdout,
                "stderr": completed.stderr,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    return completed.returncode


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="action", required=True)
    manifest = subparsers.add_parser("manifest")
    manifest.add_argument("output", type=pathlib.Path)
    artifacts = subparsers.add_parser("artifacts")
    artifacts.add_argument("output", type=pathlib.Path)
    artifacts.add_argument("paths", nargs="+", type=pathlib.Path)
    run = subparsers.add_parser("run")
    run.add_argument("output", type=pathlib.Path)
    run.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.action == "manifest":
        return write_manifest(args.output.resolve())
    if args.action == "artifacts":
        return write_artifact_manifest(args.output.resolve(), args.paths)
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("run requires a command after --")
    return capture(args.output.resolve(), command)


if __name__ == "__main__":
    raise SystemExit(main())
