#!/usr/bin/env python3
import argparse
import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path


def required_candidate_assets(version: str) -> list[str]:
    """Return the complete, uploadable asset set for one candidate."""

    return [
        f"Sbobino_{version}_aarch64.dmg",
        f"Sbobino_{version}_x86_64.dmg",
        f"Sbobino_{version}_aarch64.app.tar.gz",
        f"Sbobino_{version}_aarch64.app.tar.gz.sig",
        f"Sbobino_{version}_x86_64.app.tar.gz",
        f"Sbobino_{version}_x86_64.app.tar.gz.sig",
        f"Sbobino_{version}_windows_x86_64-setup.exe",
        f"Sbobino_{version}_windows_x86_64.nsis.zip",
        f"Sbobino_{version}_windows_x86_64.nsis.zip.sig",
        "latest.json",
        "setup-manifest.json",
        "runtime-manifest.json",
        "speech-runtime-macos-aarch64.zip",
        "speech-runtime-macos-x86_64.zip",
        "speech-runtime-windows-x86_64.zip",
        "pyannote-manifest.json",
        "pyannote-runtime-macos-aarch64.zip",
        "pyannote-runtime-macos-x86_64.zip",
        "pyannote-runtime-windows-x86_64.zip",
        "pyannote-model-community-1.zip",
        "release-notes.md",
    ]


def _zip_members(path: Path) -> set[str]:
    import zipfile

    try:
        with zipfile.ZipFile(path) as archive:
            names = {
                name.replace("\\", "/").lstrip("./")
                for name in archive.namelist()
                if name and not name.endswith("/")
            }
    except (OSError, zipfile.BadZipFile) as error:
        raise SystemExit(f"Candidate asset is not a readable ZIP archive: {path}: {error}") from error
    if not names:
        raise SystemExit(f"Candidate asset ZIP is empty: {path}")
    return names


def _require_zip_member(path: Path, expected: str) -> None:
    if expected not in _zip_members(path):
        raise SystemExit(f"Candidate asset {path.name} is missing packaged member {expected}.")


def _validate_packaged_runtime_assets(output_dir: Path) -> None:
    for filename in ("speech-runtime-macos-aarch64.zip", "speech-runtime-macos-x86_64.zip"):
        path = output_dir / filename
        for binary in ("ffmpeg", "whisper-cli", "whisper-stream", "parakeet-cli", "parakeet-batch-json"):
            _require_zip_member(path, f"runtime/bin/{binary}")

    windows_runtime = output_dir / "speech-runtime-windows-x86_64.zip"
    for binary in (
        "ffmpeg.exe",
        "whisper-cli.exe",
        "whisper-stream.exe",
        "parakeet-cli.exe",
        "parakeet-batch-json.exe",
    ):
        _require_zip_member(windows_runtime, f"runtime/bin/{binary}")

    for filename in ("pyannote-runtime-macos-aarch64.zip", "pyannote-runtime-macos-x86_64.zip"):
        _require_zip_member(output_dir / filename, "python/bin/python3")
    _require_zip_member(output_dir / "pyannote-runtime-windows-x86_64.zip", "python/python.exe")
    _require_zip_member(output_dir / "pyannote-model-community-1.zip", "model/config.yaml")


def _validate_manifest_versions(output_dir: Path, version: str) -> None:
    documents: dict[str, dict] = {}
    for filename in ("latest.json", "setup-manifest.json", "runtime-manifest.json", "pyannote-manifest.json"):
        try:
            document = json.loads((output_dir / filename).read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise SystemExit(f"Candidate manifest {filename} is not valid JSON: {error}") from error
        if not isinstance(document, dict):
            raise SystemExit(f"Candidate manifest {filename} must contain a JSON object.")
        if document.get("app_version", document.get("version")) != version:
            raise SystemExit(f"Candidate manifest {filename} does not identify version {version}.")
        documents[filename] = document
    if documents["setup-manifest.json"].get("release_tag") != f"v{version}":
        raise SystemExit(f"Candidate setup-manifest.json does not identify tag v{version}.")


def _validate_release_notes(output_dir: Path) -> None:
    try:
        notes = (output_dir / "release-notes.md").read_text(encoding="utf-8")
    except OSError as error:
        raise SystemExit(f"Candidate release-notes.md is not readable: {error}") from error
    for section in ("Fixes", "New and improved", "Compatibility", "Known issues", "Refs"):
        if not re.search(rf"^### {re.escape(section)}$", notes, re.MULTILINE):
            raise SystemExit(f"Candidate release-notes.md is missing mandatory section: {section}")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate the release-readiness proof for a candidate."
    )
    parser.add_argument("output_dir", help="Directory containing the candidate release assets")
    parser.add_argument("version", help="Release version without the leading v")
    parser.add_argument(
        "--release-profile",
        default="public",
        help="Release profile stored in release-readiness-proof.json",
    )
    parser.add_argument(
        "--commit-sha",
        default="",
        help="Commit SHA embedded into validation templates",
    )
    parser.add_argument("--repo-slug", default="pietroMastro92/Sbobino")
    args = parser.parse_args()

    output_dir = Path(args.output_dir).resolve()
    version = args.version.strip().removeprefix("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise SystemExit(f"Release version must be semantic MAJOR.MINOR.PATCH (got '{version}').")
    if not output_dir.is_dir():
        raise SystemExit(f"Candidate asset directory does not exist: {output_dir}")
    tag = f"v{version}"

    required_assets = required_candidate_assets(version)
    # Keep this field for consumers of the previous proof schema, but there
    # are no optional candidate assets: signatures and notes are mandatory.
    optional_assets: list[str] = []

    missing_assets = [
        name
        for name in required_assets
        if not (output_dir / name).is_file() or (output_dir / name).stat().st_size <= 0
    ]
    if missing_assets:
        raise SystemExit(
            "Missing or empty required candidate assets: " + ", ".join(missing_assets)
        )

    _validate_packaged_runtime_assets(output_dir)
    _validate_manifest_versions(output_dir, version)
    _validate_release_notes(output_dir)

    checksums = {name: sha256(output_dir / name) for name in required_assets}

    proof = {
        "version": version,
        "release_profile": args.release_profile.strip() or "public",
        "status": "passed",
        "gate": "release_readiness.sh",
        "commit_sha": args.commit_sha.strip(),
        "repo_slug": args.repo_slug.strip(),
        "generated_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "required_assets": required_assets,
        "optional_assets": optional_assets,
        "sha256": checksums,
    }
    (output_dir / "release-readiness-proof.json").write_text(
        json.dumps(proof, indent=2) + "\n",
        encoding="utf-8",
    )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
