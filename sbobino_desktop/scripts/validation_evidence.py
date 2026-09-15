#!/usr/bin/env python3

import argparse
import hashlib
import json
import os
import pathlib
import platform
import subprocess
import sys
from datetime import datetime, timezone


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
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        files.append({"path": relative, "bytes": path.stat().st_size, "sha256": digest})
        aggregate.update(f"{digest}  {relative}\n".encode())

    head = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True
    ).stdout.strip()
    dirty = bool(
        subprocess.run(
            ["git", "status", "--porcelain", "--untracked-files=all"],
            cwd=root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "generated_at": datetime.now(timezone.utc).isoformat(),
                "git_head": head,
                "dirty": dirty,
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


def capture(output: pathlib.Path, command: list[str]) -> int:
    started = datetime.now(timezone.utc)
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
    run = subparsers.add_parser("run")
    run.add_argument("output", type=pathlib.Path)
    run.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.action == "manifest":
        return write_manifest(args.output.resolve())
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("run requires a command after --")
    return capture(args.output.resolve(), command)


if __name__ == "__main__":
    raise SystemExit(main())
