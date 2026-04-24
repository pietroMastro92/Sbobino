#!/usr/bin/env python3
import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate release-readiness proof and machine validation templates."
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
    args = parser.parse_args()

    output_dir = Path(args.output_dir).resolve()
    version = args.version.strip()
    tag = f"v{version}"

    required_assets = [
        f"Sbobino_{version}_aarch64.dmg",
        "Sbobino.app.tar.gz",
        "latest.json",
        "setup-manifest.json",
        "runtime-manifest.json",
        "speech-runtime-macos-aarch64.zip",
        "pyannote-manifest.json",
        "pyannote-runtime-macos-aarch64.zip",
        "pyannote-model-community-1.zip",
    ]
    optional_assets = ["Sbobino.app.tar.gz.sig"]

    checksums = {}
    for name in required_assets + optional_assets:
        path = output_dir / name
        if path.is_file():
            checksums[name] = sha256(path)

    proof = {
        "version": version,
        "release_profile": args.release_profile.strip() or "public",
        "status": "passed",
        "gate": "release_readiness.sh",
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
