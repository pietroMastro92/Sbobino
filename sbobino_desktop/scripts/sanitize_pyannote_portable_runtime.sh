#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: sanitize_pyannote_portable_runtime.sh \
         --runtime <input-runtime-dir> \
         --model <input-model-dir> \
         --output <output-pyannote-root> \
         --evidence <output-evidence-dir> \
         [--arch arm64|x86_64]

Repairs an existing, non-portable Pyannote runtime payload offline and writes
a complete, installer-ready `pyannote` resource root:

  <output>/python/<target-triple>/...   sanitized, re-signed runtime
  <output>/model/...                    model copied unchanged

The input tree is never modified: it is cloned (APFS copy-on-write when the
volumes allow it) into <output> first, and only the clone is sanitized. The
sanitizer, the otool parsers and the strict audit come from
scripts/lib/pyannote_portable_runtime.py, the same module used by
setup_bundled_pyannote.sh and package_pyannote_asset.sh.

Refuses: personal profile paths (`.../Library/Application Support/...`),
private/ lockbox paths, pairing material, an output that is the input, and
nested input/output paths. Does not create, overwrite or delete anything
outside <output> and <evidence>.

Exit status: 0 only when the repaired payload passes the strict audit.
EOF
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

RUNTIME_DIR=""
MODEL_DIR=""
OUTPUT_DIR=""
EVIDENCE_DIR=""
EXPECTED_ARCH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtime)
      RUNTIME_DIR=${2:-}
      shift 2
      ;;
    --model)
      MODEL_DIR=${2:-}
      shift 2
      ;;
    --output)
      OUTPUT_DIR=${2:-}
      shift 2
      ;;
    --evidence)
      EVIDENCE_DIR=${2:-}
      shift 2
      ;;
    --arch)
      EXPECTED_ARCH=${2:-}
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$RUNTIME_DIR" || -z "$MODEL_DIR" || -z "$OUTPUT_DIR" || -z "$EVIDENCE_DIR" ]]; then
  usage
  exit 1
fi

need_cmd python3
need_cmd otool
need_cmd lipo
need_cmd install_name_tool
need_cmd codesign
need_cmd find

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PYANNOTE_PORTABLE_RUNTIME_LIB="$SCRIPT_DIR/lib/pyannote_portable_runtime.py"

if [[ ! -f "$PYANNOTE_PORTABLE_RUNTIME_LIB" ]]; then
  echo "Missing shared runtime helper: $PYANNOTE_PORTABLE_RUNTIME_LIB" >&2
  exit 1
fi

if [[ ! -d "$RUNTIME_DIR" ]]; then
  echo "Runtime directory not found: $RUNTIME_DIR" >&2
  exit 1
fi
if [[ ! -d "$MODEL_DIR" ]]; then
  echo "Model directory not found: $MODEL_DIR" >&2
  exit 1
fi
if [[ ! -f "$MODEL_DIR/config.yaml" ]]; then
  echo "Model directory has no config.yaml: $MODEL_DIR" >&2
  exit 1
fi

# Path guard: canonical paths, no personal profile, no destructive overlap.
GUARD_JSON=$(python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" guard-paths "$RUNTIME_DIR" "$OUTPUT_DIR")
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" guard-paths "$MODEL_DIR" "$OUTPUT_DIR" >/dev/null
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" guard-paths "$EVIDENCE_DIR" "$OUTPUT_DIR" >/dev/null
RUNTIME_REAL=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["input"])' "$GUARD_JSON")
OUTPUT_REAL=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["output"])' "$GUARD_JSON")

if [[ ! -d "$RUNTIME_REAL/lib" ]]; then
  echo "Runtime directory has no lib/: $RUNTIME_REAL" >&2
  exit 1
fi

python_binary=""
for candidate in "$RUNTIME_REAL/bin/python3" "$RUNTIME_REAL"/bin/python3.*; do
  if [[ -x "$candidate" ]] && lipo -archs "$candidate" >/dev/null 2>&1; then
    python_binary=$candidate
    break
  fi
done
if [[ -z "$python_binary" ]]; then
  echo "Runtime directory has no executable bin/python3: $RUNTIME_REAL" >&2
  exit 1
fi

ARCH_FROM_BINARY=$(lipo -archs "$python_binary" | awk '{print $1}')
ARCH_FROM_NAME=""
case "$(basename "$RUNTIME_REAL")" in
  aarch64-apple-darwin) ARCH_FROM_NAME=arm64 ;;
  x86_64-apple-darwin) ARCH_FROM_NAME=x86_64 ;;
esac

if [[ -z "$EXPECTED_ARCH" ]]; then
  EXPECTED_ARCH=${ARCH_FROM_NAME:-$ARCH_FROM_BINARY}
fi
if [[ "$EXPECTED_ARCH" != "$ARCH_FROM_BINARY" ]]; then
  echo "Requested arch '$EXPECTED_ARCH' does not match bin/python3 arch '$ARCH_FROM_BINARY'" >&2
  exit 1
fi
if [[ -n "$ARCH_FROM_NAME" && "$ARCH_FROM_NAME" != "$EXPECTED_ARCH" ]]; then
  echo "Directory name implies arch '$ARCH_FROM_NAME' but bin/python3 is '$ARCH_FROM_BINARY'" >&2
  exit 1
fi

case "$EXPECTED_ARCH" in
  arm64) TARGET_TRIPLE=aarch64-apple-darwin ;;
  x86_64) TARGET_TRIPLE=x86_64-apple-darwin ;;
  *)
    echo "Unsupported architecture '$EXPECTED_ARCH' (expected arm64 or x86_64)" >&2
    exit 1
    ;;
esac

if [[ -e "$OUTPUT_REAL" ]] && [[ -n "$(ls -A "$OUTPUT_REAL" 2>/dev/null || true)" ]]; then
  echo "Output '$OUTPUT_REAL' already exists and is not empty; refusing to overwrite." >&2
  exit 1
fi
if [[ -e "$EVIDENCE_DIR" ]] && [[ -n "$(ls -A "$EVIDENCE_DIR" 2>/dev/null || true)" ]]; then
  echo "Evidence '$EVIDENCE_DIR' already exists and is not empty; refusing to overwrite." >&2
  exit 1
fi

mkdir -p "$OUTPUT_REAL/python/$TARGET_TRIPLE" "$OUTPUT_REAL/model" "$EVIDENCE_DIR"

echo "== source inventory (inputs, read only) =="
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" inventory "$RUNTIME_REAL" \
  "$EVIDENCE_DIR/inventory-source-runtime.json" \
  --sums "$EVIDENCE_DIR/sha256sums-source-runtime.txt"
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" inventory "$MODEL_DIR" \
  "$EVIDENCE_DIR/inventory-source-model.json" \
  --sums "$EVIDENCE_DIR/sha256sums-source-model.txt"

MARKER="$EVIDENCE_DIR/source-preservation.marker"
: >"$MARKER"
touch "$MARKER"

echo "== pre-audit of the input runtime (expected RED on a stale asset) =="
set +e
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" audit "$RUNTIME_REAL" --expect-arch "$EXPECTED_ARCH" \
  --json "$EVIDENCE_DIR/pre-audit-source.json" \
  >"$EVIDENCE_DIR/pre-audit-source.txt" 2>&1
PRE_AUDIT_STATUS=$?
set -e
cat "$EVIDENCE_DIR/pre-audit-source.txt"

clone_tree() {
  local source=$1
  local target=$2
  mkdir -p "$target"
  if cp -cRp "$source/." "$target/" 2>/dev/null; then
    return 0
  fi
  echo "  (APFS clone unavailable for '$source', falling back to a plain copy)"
  rm -rf "$target"
  mkdir -p "$target"
  cp -Rp "$source/." "$target/"
}

echo "== cloning the runtime into the staged payload (CoW, source untouched) =="
clone_tree "$RUNTIME_REAL" "$OUTPUT_REAL/python/$TARGET_TRIPLE"
clone_tree "$MODEL_DIR" "$OUTPUT_REAL/model"

echo "== sanitizing the staged runtime =="
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" sanitize "$OUTPUT_REAL/python/$TARGET_TRIPLE" \
  --json "$EVIDENCE_DIR/sanitize-receipt.json"

echo "== strict audit of the staged runtime =="
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" audit "$OUTPUT_REAL/python/$TARGET_TRIPLE" \
  --expect-arch "$EXPECTED_ARCH" \
  --json "$EVIDENCE_DIR/post-audit-payload.json" \
  >"$EVIDENCE_DIR/post-audit-payload.txt"
cat "$EVIDENCE_DIR/post-audit-payload.txt"

echo "== payload inventory =="
python3 "$PYANNOTE_PORTABLE_RUNTIME_LIB" inventory "$OUTPUT_REAL" \
  "$EVIDENCE_DIR/inventory-payload.json" \
  --sums "$EVIDENCE_DIR/sha256sums-payload.txt"

echo "== source preservation check =="
if [[ -n "$(find "$RUNTIME_REAL" -newer "$MARKER" -print -quit 2>/dev/null)" ]]; then
  echo "The input runtime was modified during the repair:" >&2
  find "$RUNTIME_REAL" -newer "$MARKER" -print | head -20 >&2
  exit 1
fi

python3 - "$EVIDENCE_DIR" "$RUNTIME_REAL" "$MODEL_DIR" "$OUTPUT_REAL" "$EXPECTED_ARCH" "$TARGET_TRIPLE" "$PRE_AUDIT_STATUS" <<'PY'
import json
import pathlib
import platform
import subprocess
import sys

evidence = pathlib.Path(sys.argv[1])
runtime = sys.argv[2]
model = sys.argv[3]
output = sys.argv[4]
arch = sys.argv[5]
triple = sys.argv[6]
pre_status = int(sys.argv[7])


def version(command):
    try:
        completed = subprocess.run(command, capture_output=True, text=True)
    except OSError as error:
        return "unavailable: {}".format(error)
    lines = (completed.stdout or completed.stderr).strip().splitlines()
    return lines[0] if lines else "unknown"


def summary(name):
    data = json.loads((evidence / name).read_text())
    return {
        "root": data["root"],
        "macho": data["binaries"],
        "signed_valid": data["signed_valid"],
        "violations": len(data["violations"]),
        "violation_kinds": sorted({item["kind"] for item in data["violations"]}),
        "warnings": len(data["warnings"]),
        "warning_kinds": sorted({item["kind"] for item in data["warnings"]}),
    }


payload_inventory = json.loads((evidence / "inventory-payload.json").read_text())
receipt = json.loads((evidence / "sanitize-receipt.json").read_text())

report = {
    "schema": 1,
    "task": "pyannote-portable-runtime-repair",
    "arch": arch,
    "triple": triple,
    "inputs": {
        "runtime": runtime,
        "model": model,
        "source_runtime_tree_sha256": json.loads(
            (evidence / "inventory-source-runtime.json").read_text()
        )["tree_sha256"],
        "source_model_tree_sha256": json.loads(
            (evidence / "inventory-source-model.json").read_text()
        )["tree_sha256"],
    },
    "output": {
        "payload_root": output,
        "tree_sha256": payload_inventory["tree_sha256"],
        "files": payload_inventory["files"],
        "symlinks": payload_inventory["symlinks"],
        "bytes": payload_inventory["bytes"],
    },
    "audit": {
        "pre_source_red": summary("pre-audit-source.json"),
        "pre_source_exit_status": pre_status,
        "post_payload_green": summary("post-audit-payload.json"),
    },
    "sanitize_receipt": {
        "modified": receipt["modified"],
        "removed_rpaths": receipt["removed_rpaths"],
        "repaired_install_names": receipt["repaired_install_names"],
        "signed": receipt["signed"],
        "signed_bundles": receipt["signed_bundles"],
    },
    "tools": {
        "host_arch": platform.machine(),
        "python": sys.version.split()[0],
        "otool": version(["/usr/bin/otool", "--version"]),
        "sw_vers": version(["/usr/bin/sw_vers"]),
        "xcode_select": version(["/usr/bin/xcode-select", "-p"]),
    },
}
(evidence / "repair.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({"payload": output, "evidence": str(evidence), "repair": report["output"]}, indent=2))
PY

echo "Repaired pyannote payload: $OUTPUT_REAL"
echo "Evidence:                  $EVIDENCE_DIR"
