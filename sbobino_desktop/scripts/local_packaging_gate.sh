#!/usr/bin/env bash
# One-command LOCAL prerelease packaging gate for the bundled Pyannote runtime.
#
# It performs the real thing, not a simulation:
#   1. preflight (host, toolchain, disk, path guards) — fails before any
#      expensive work when the machine cannot produce releasable evidence
#   2. the three packaging/staging/review unit entrypoints
#   3. a complete native hydration with the release command
#      (scripts/setup_bundled_pyannote.sh --force), assembled into an external
#      resource root, never into the tracked app resources
#   4. the official packager, the strict audit, the real product entrypoint, the
#      same entrypoint after relocating the payload to a path with spaces, both
#      import orders of pyannote.audio/torchcodec, signature checks, sanitizer
#      byte-idempotence and a load-command scan for build-root/host references
#
# It never dispatches, pushes, tags or uploads anything.
#
# The target is the architecture of *this* process, exactly as the release
# workflow selects it: `uname -m` inside the shell that runs the gate.  On
# x86_64 the gate drives the Intel conda branch of setup_bundled_pyannote.sh
# and therefore needs `conda` and a Python that really contains an x86_64
# slice (for example /usr/bin/python3 under `/usr/bin/arch -x86_64`).
#
# Usage:
#   local_packaging_gate.sh --work-dir DIR [options]
#
#   --work-dir DIR        Absolute, task-owned directory on a build volume.
#                         Everything (TMPDIR, pip/conda caches, staging, output,
#                         evidence) is created below it.
#   --unit-only           Run only the unit entrypoints (seconds).
#   --verify-only DIR     Skip hydration and verify an existing resource root.
#   --python-version X.Y  Interpreter version for the hydration. Default 3.11.
#   --python PATH         Interpreter used by the gate itself and by the unit
#                         suites. Default: python3 from PATH. Its Mach-O slices
#                         must match the target architecture, otherwise the
#                         unit suites would build fixtures for another machine.
#   --source-framework D  Optional python.org framework directory
#                         (…/Versions/3.11) whose bundled python.o must appear
#                         in the hydrated payload byte for byte.
#   --interpreter-dir D   Directory prepended to PATH for the hydration (for
#                         example a python.org framework's bin/, so the run
#                         uses that interpreter instead of the system one).
#   --keep                Keep the hydrated payload and evidence for review.
#
# Exit status: 0 only when every executed check passed.
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
PORTABLE="$SCRIPT_DIR/lib/pyannote_portable_runtime.py"
HYDRATE="$SCRIPT_DIR/setup_bundled_pyannote.sh"
PACKAGE="$SCRIPT_DIR/package_pyannote_asset.sh"
ENTRYPOINT="$SCRIPT_DIR/pyannote_diarize.py"

WORK_DIR=""
UNIT_ONLY=0
VERIFY_ONLY=""
PYTHON_VERSION=3.11
SOURCE_FRAMEWORK=""
INTERPRETER_DIR=""
KEEP=0
GATE_PYTHON=${GATE_PYTHON:-python3}
HOST_ARCH=$(uname -m)
TARGET_TRIPLE=""
EXPECT_ARCH=""
PROCESS_MODE="unknown"
STAGE_DIR_FROM_LOG=""

usage() {
  sed -n '2,44p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --work-dir) WORK_DIR=${2:-}; shift 2 ;;
    --unit-only) UNIT_ONLY=1; shift ;;
    --verify-only) VERIFY_ONLY=${2:-}; shift 2 ;;
    --python-version) PYTHON_VERSION=${2:-}; shift 2 ;;
    --python) GATE_PYTHON=${2:-}; shift 2 ;;
    --source-framework) SOURCE_FRAMEWORK=${2:-}; shift 2 ;;
    --interpreter-dir) INTERPRETER_DIR=${2:-}; shift 2 ;;
    --keep) KEEP=1; shift ;;
    -h|--help) usage ;;
    *) echo "Unknown argument: $1" >&2; usage ;;
  esac
done

STATUS=0
CHECKS=""

record() {
  local result=$1
  local name=$2
  shift 2
  local detail=${*:-}
  CHECKS+="${result}\t${name}\t${detail}\n"
  printf '%-4s %s%s\n' "$result" "$name" "${detail:+ — $detail}"
  if [[ "$result" != "PASS" && "$result" != "SKIP" && "$result" != "INFO" ]]; then
    STATUS=1
  fi
}

preflight() {
  local failures=0
  if [[ "$(uname -s)" != "Darwin" ]]; then
    record FAIL preflight.host "this gate needs a macOS host"
    return 1
  fi
  local tool
  for tool in /usr/bin/otool /usr/bin/lipo /usr/bin/codesign /usr/bin/install_name_tool \
              /usr/bin/ditto /usr/bin/rsync /usr/bin/curl /usr/bin/shasum python3 git; do
    if [[ ! -x "$tool" ]] && ! command -v "$tool" >/dev/null 2>&1; then
      record FAIL "preflight.tool.$(basename "$tool")" "missing"
      failures=$((failures + 1))
    fi
  done
  if ! command -v git-lfs >/dev/null 2>&1; then
    record FAIL preflight.tool.git-lfs "missing (the model is an LFS repository)"
    failures=$((failures + 1))
  fi
  if [[ -z "$WORK_DIR" || "$WORK_DIR" != /* ]]; then
    record FAIL preflight.work_dir "--work-dir must be an absolute path"
    return 1
  fi
  if ! python3 "$PORTABLE" guard-root "$WORK_DIR" >/dev/null 2>&1; then
    record FAIL preflight.work_dir "refused by the personal-path guard: $WORK_DIR"
    return 1
  fi
  if ! WORK_DIR=$(python3 - "$WORK_DIR" "$REPO_ROOT" <<'PY'
import sys
from pathlib import Path

work, checkout = (Path(value).resolve() for value in sys.argv[1:])
if work == checkout or checkout in work.parents:
    raise SystemExit("work directory must live outside the checkout")
if work == Path(work.anchor):
    raise SystemExit("the filesystem root is not a task work directory")
print(work)
PY
  ); then
    record FAIL preflight.work_dir "must live outside the checkout"
    return 1
  fi
  if ! python3 "$PORTABLE" guard-root "$WORK_DIR" >/dev/null 2>&1; then
    record FAIL preflight.work_dir "canonical path refused by the personal-path guard"
    return 1
  fi

  # The target is the architecture of this *process*, never a guess: the
  # release workflow picks the same branch from `uname -m` inside the shell it
  # runs, so a gate started under a fabricated uname wrapper cannot silently
  # claim Intel evidence.
  HOST_ARCH=$(uname -m)
  case "$HOST_ARCH" in
    arm64)
      TARGET_TRIPLE="aarch64-apple-darwin"
      EXPECT_ARCH="arm64"
      ;;
    x86_64)
      TARGET_TRIPLE="x86_64-apple-darwin"
      EXPECT_ARCH="x86_64"
      ;;
    *)
      record FAIL preflight.arch "unsupported host architecture $(uname -m)"
      return 1
      ;;
  esac
  local translated
  translated=$(sysctl -n sysctl.proc_translated 2>/dev/null || true)
  case "${translated:-}" in
    1) PROCESS_MODE="rosetta-translated" ;;
    0) PROCESS_MODE="native" ;;
    *) PROCESS_MODE="unknown" ;;
  esac
  record INFO preflight.target "$EXPECT_ARCH ($TARGET_TRIPLE), process mode $PROCESS_MODE, Python $PYTHON_VERSION"

  # A gate interpreter without a slice for the target would build fixtures for
  # the other machine and turn every Mach-O assertion into a coincidence.
  local gate_python_path gate_python_archs
  gate_python_path=$(command -v "$GATE_PYTHON" 2>/dev/null)
  if [[ -z "$gate_python_path" ]]; then
    record FAIL preflight.python "interpreter not found: $GATE_PYTHON"
    return 1
  fi
  gate_python_archs=$(/usr/bin/lipo -archs "$gate_python_path" 2>/dev/null || true)
  if [[ "$gate_python_archs" != *"$EXPECT_ARCH"* ]]; then
    record FAIL preflight.python "$gate_python_path has slices '${gate_python_archs:-unknown}', not $EXPECT_ARCH (pass --python)"
    return 1
  fi
  record INFO preflight.python "$gate_python_path ($gate_python_archs)"

  if [[ "$EXPECT_ARCH" == "x86_64" ]]; then
    if ! command -v conda >/dev/null 2>&1; then
      record FAIL preflight.conda "the Intel branch of setup_bundled_pyannote.sh needs conda on PATH"
      return 1
    fi
    record INFO preflight.conda "$(command -v conda), $(conda --version 2>/dev/null), CONDA_SUBDIR=osx-64"
  fi

  mkdir -p "$WORK_DIR" || { record FAIL preflight.work_dir "cannot create $WORK_DIR"; return 1; }
  local available_kb required_kb=12582912
  # The Intel hydration also solves a conda environment with torch/ffmpeg and
  # compiles TorchCodec, so it needs a larger reserve than the wheel install.
  if [[ "$EXPECT_ARCH" == "x86_64" ]]; then
    required_kb=20971520
  fi
  available_kb=$(df -Pk "$WORK_DIR" | awk 'NR==2 {print $4}')
  if [[ -z "$available_kb" ]] || [[ "$available_kb" -lt $required_kb ]]; then
    record FAIL preflight.disk "needs >= $((required_kb / 1024)) MiB free, found $((available_kb / 1024)) MiB"
    return 1
  fi
  if [[ "$failures" -ne 0 ]]; then
    return 1
  fi
  record PASS preflight "macOS $(sw_vers -productVersion), $EXPECT_ARCH ($PROCESS_MODE), Python $PYTHON_VERSION, $((available_kb / 1024)) MiB free"
}

run_unit_entrypoints() {
  local tmp="$WORK_DIR/tmp"
  mkdir -p "$tmp"
  export TMPDIR="$tmp"
  export SBOBINO_RELOCATION_TEST_ROOT="$tmp/relocation"
  export SBOBINO_STAGING_TEST_ROOT="$tmp/staging"
  mkdir -p "$SBOBINO_RELOCATION_TEST_ROOT" "$SBOBINO_STAGING_TEST_ROOT"
  local suite
  for suite in test_pyannote_portable_runtime.py test_setup_pyannote_staging.py test_pyannote_review_regressions.py; do
    local log="$EVIDENCE/unit-${suite%.py}.log"
    if (cd "$REPO_ROOT" && "$GATE_PYTHON" "scripts/$suite" >"$log" 2>&1); then
      record PASS "unit.$suite" "$(grep -E '^Ran [0-9]+ tests' "$log" | tail -1) [$EXPECT_ARCH process]"
    else
      record FAIL "unit.$suite" "see $log"
      tail -20 "$log"
    fi
  done
}

hydrate() {
  local output="$WORK_DIR/output/pyannote"
  rm -rf "$output"
  mkdir -p "$WORK_DIR/tmp" "$WORK_DIR/pip-cache"
  export TMPDIR="$WORK_DIR/tmp"
  export PIP_CACHE_DIR="$WORK_DIR/pip-cache"
  export PIP_DISABLE_PIP_VERSION_CHECK=1
  # Task-owned caches only: the base conda installation (and every personal
  # environment) is used read-only through its executable.
  export CONDA_PKGS_DIRS="$WORK_DIR/conda-pkgs"
  export CONDA_ENVS_PATH="$WORK_DIR/conda-envs"
  mkdir -p "$CONDA_PKGS_DIRS" "$CONDA_ENVS_PATH"
  if [[ "$EXPECT_ARCH" == "x86_64" ]]; then
    # The local base resolver is a native arm64 conda; without an explicit
    # target it would create an osx-arm64 environment inside an "Intel" run.
    export CONDA_SUBDIR=osx-64
  fi
  if [[ -n "$INTERPRETER_DIR" ]]; then
    export PATH="$INTERPRETER_DIR:$PATH"
  fi
  local log="$EVIDENCE/hydrate.log"
  local started ended
  started=$(date +%s)
  record INFO hydrate.target "$TARGET_TRIPLE, CONDA_SUBDIR=${CONDA_SUBDIR:-unset}, pkgs=$CONDA_PKGS_DIRS"
  if [[ -n "$INTERPRETER_DIR" ]]; then
    record INFO hydrate.interpreter "$(PATH="$INTERPRETER_DIR:$PATH" command -v "python$PYTHON_VERSION" || echo unresolved)"
  fi
  if (cd "$REPO_ROOT" && bash "$HYDRATE" --force --python-version "$PYTHON_VERSION" \
        --resource-root "$output" >"$log" 2>&1); then
    ended=$(date +%s)
    record PASS hydrate.complete "real download/build/sanitize/audit in $((ended - started))s"
    RESOURCE_ROOT="$output"
    STAGE_DIR_FROM_LOG=$(sed -n 's/^Staging runtime in //p' "$log" | head -1)
  else
    record FAIL hydrate.complete "see $log"
    tail -30 "$log"
    return 1
  fi
  if grep -q "AUDIT OK" "$log"; then
    record PASS hydrate.audit "$(grep -E '^sanitized ' "$log" | tail -1)"
  else
    record FAIL hydrate.audit "hydration did not report AUDIT OK"
  fi
}

verify_idempotence() {
  local runtime=$1
  local label=$2
  local copy="$WORK_DIR/idempotence-$label/pyannote"
  rm -rf "$WORK_DIR/idempotence-$label"
  mkdir -p "$(dirname "$copy")"
  /usr/bin/ditto "$runtime" "$copy"
  python3 "$PORTABLE" inventory "$copy" "$EVIDENCE/idempotence-$label-before.json" >/dev/null 2>&1
  local receipt="$EVIDENCE/idempotence-$label-sanitize.json"
  if ! python3 "$PORTABLE" sanitize "$copy" --json "$receipt" \
      >"$EVIDENCE/idempotence-$label-sanitize.log" 2>&1; then
    record FAIL "verify.idempotence.$label" "second sanitize failed, see $EVIDENCE/idempotence-$label-sanitize.log"
    rm -rf "$WORK_DIR/idempotence-$label"
    return 1
  fi
  python3 "$PORTABLE" inventory "$copy" "$EVIDENCE/idempotence-$label-after.json" >/dev/null 2>&1
  local verdict
  verdict=$(python3 - "$EVIDENCE/idempotence-$label-before.json" \
      "$EVIDENCE/idempotence-$label-after.json" "$receipt" <<'PY'
import json
import sys

before, after, receipt = (json.load(open(path)) for path in sys.argv[1:])
modified = len(receipt["modified"])
if before["tree_sha256"] != after["tree_sha256"]:
    print("tree hash changed by a second sanitize")
elif modified:
    print("{} file(s) rewritten by a second sanitize".format(modified))
elif receipt["removed_rpaths"] or receipt["repaired_install_names"] or receipt["repaired_dependencies"]:
    print("second sanitize reported repairs on a sanitized tree")
else:
    print("byte-identical ({} files, {})".format(after["files"], after["tree_sha256"][:16]))
PY
)
  if [[ "$verdict" == byte-identical* ]]; then
    record PASS "verify.idempotence.$label" "$verdict"
  else
    record FAIL "verify.idempotence.$label" "$verdict"
  fi
  rm -rf "$WORK_DIR/idempotence-$label"
}

verify_load_commands() {
  local runtime=$1
  local label=$2
  local report="$EVIDENCE/load-commands-$label.json"
  local verdict
  verdict=$(python3 - "$PORTABLE" "$runtime" "$report" "$WORK_DIR" "$HOME" "$STAGE_DIR_FROM_LOG" <<'PY'
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(sys.argv[1]).parent))
import pyannote_portable_runtime as portable  # noqa: E402

root = pathlib.Path(sys.argv[2])
output = pathlib.Path(sys.argv[3])
markers = [value for value in sys.argv[4:] if value]
# Host-managed and personal locations a shipped runtime must never resolve
# through; the payload itself can live on any volume, so its own path is not a
# marker unless a load command still records it after sanitization.
markers += ["/opt/homebrew", "/usr/local/opt", "miniconda", "anaconda", "conda-meta", "/Users/"]
findings = []
checked = 0
for relative, binary, _container in portable.scan_macho_payload(root)["macho"]:
    checked += 1
    for value in (portable.read_linked_dependencies(binary)
                  + portable.read_otool_rpaths(binary)
                  + portable.read_install_names(binary)):
        for marker in markers:
            if marker and marker in value:
                findings.append({"path": relative, "reference": value, "marker": marker})
                break
output.write_text(json.dumps({"root": str(root), "markers": sorted(set(markers)),
                              "binaries": checked, "findings": findings}, indent=2) + "\n")
if findings:
    print("{} load command(s) still reference a build/host location, see {}".format(
        len(findings), output))
else:
    print("{} Mach-O clean of build-root/host load commands".format(checked))
PY
)
  if [[ "$verdict" == *"clean of build-root"* ]]; then
    record PASS "verify.load_commands.$label" "$verdict"
  else
    record FAIL "verify.load_commands.$label" "$verdict"
  fi
}

verify_payload() {
  local root=$1
  local runtime="$root/python/$TARGET_TRIPLE"
  local stdlib="$runtime/lib/python$PYTHON_VERSION"
  local label=$2

  local missing=""
  local required
  for required in "lib/python$PYTHON_VERSION/encodings" "lib/python$PYTHON_VERSION/lib-dynload" \
                  "lib/python$PYTHON_VERSION/types.py" "lib/python$PYTHON_VERSION/traceback.py" \
                  "lib/python$PYTHON_VERSION/collections/__init__.py" \
                  "lib/python$PYTHON_VERSION/collections/abc.py" \
                  "lib/python$PYTHON_VERSION/site-packages" bin/python3; do
    [[ -e "$runtime/$required" ]] || missing+=" $required"
  done
  if [[ -n "$missing" ]]; then
    record FAIL "verify.layout.$label" "missing:$missing"
  else
    record PASS "verify.layout.$label" "Rust-side completeness contract satisfied"
  fi

  local audit_log="$EVIDENCE/audit-$label.txt"
  if python3 "$PORTABLE" audit "$runtime" --expect-arch "$EXPECT_ARCH" \
      --json "$EVIDENCE/audit-$label.json" >"$audit_log" 2>&1; then
    record PASS "verify.audit.$label" "0 violations ($EXPECT_ARCH), $(python3 -c 'import json,sys;print(len(json.load(open(sys.argv[1]))["warnings"]))' "$EVIDENCE/audit-$label.json") warnings, strict_closure=false"
  else
    record FAIL "verify.audit.$label" "see $audit_log"
  fi
  # Not a gate failure by contract (torchcodec ships inactive FFmpeg variants),
  # but the count has to be visible instead of implied.
  python3 "$PORTABLE" audit "$runtime" --expect-arch "$EXPECT_ARCH" --strict-closure --quiet \
      --json "$EVIDENCE/audit-$label-strict.json" >/dev/null 2>&1
  record INFO "verify.audit.$label.strict_closure" "$(python3 -c 'import json,sys;print(len(json.load(open(sys.argv[1]))["violations"]),"closure violation(s)")' "$EVIDENCE/audit-$label-strict.json" 2>/dev/null || echo 'unavailable')"

  verify_idempotence "$runtime" "$label"
  verify_load_commands "$runtime" "$label"

  if [[ "$label" == "original" ]]; then
    local zip="$EVIDENCE/pyannote-runtime-$EXPECT_ARCH.zip"
    rm -f "$zip"
    if bash "$PACKAGE" "$root/python" python "$zip" >"$EVIDENCE/package.log" 2>&1; then
      record PASS "verify.package" "package_pyannote_asset.sh $(( $(stat -f%z "$zip") / 1024 / 1024 )) MiB ($EXPECT_ARCH)"
      rm -f "$zip"
    else
      record FAIL "verify.package" "see $EVIDENCE/package.log"
    fi
  fi

  local audio="$EVIDENCE/probe.wav"
  local require_turns=0
  if [[ ! -f "$audio" ]]; then
    if /usr/bin/say -o "$EVIDENCE/probe.aiff" \
        "This is a short recording used to check the bundled speaker diarization runtime. It should contain at least one speaker turn." \
        >/dev/null 2>&1 &&
       /usr/bin/afconvert -f WAVE -d LEI16@16000 -c 1 "$EVIDENCE/probe.aiff" "$audio" >/dev/null 2>&1; then
      require_turns=1
    else
      python3 - "$audio" <<'PY'
import math, struct, sys, wave
path = sys.argv[1]
rate, frames = 16000, []
for index in range(rate * 3):
    t = index / rate
    frequency = 220.0 if int(t * 2) % 2 == 0 else 440.0
    frames.append(struct.pack("<h", int(12000 * math.sin(2 * math.pi * frequency * t))))
with wave.open(path, "wb") as handle:
    handle.setnchannels(1)
    handle.setsampwidth(2)
    handle.setframerate(rate)
    handle.writeframes(b"".join(frames))
PY
    fi
  else
    require_turns=$AUDIO_IS_SPEECH
  fi
  AUDIO_IS_SPEECH=$require_turns

  local output="$EVIDENCE/entrypoint-$label.json"
  if env -i PATH="/usr/bin:/bin" PYTHONHOME="$runtime" \
      PYTHONPATH="$stdlib:$stdlib/lib-dynload:$stdlib/site-packages" \
      PYTHONNOUSERSITE=1 \
      "$runtime/bin/python3" "$ENTRYPOINT" --audio-path "$audio" \
      --model-path "$root/model" --device cpu >"$output" \
      2>"$EVIDENCE/entrypoint-$label.stderr.log"; then
    local turns
    turns=$(python3 - "$output" <<'PY'
import json, sys
try:
    data = json.load(open(sys.argv[1]))
except json.JSONDecodeError as error:
    # The product contract is a single JSON document on stdout
    # (crates/infrastructure/src/adapters/pyannote.rs parses the whole
    # stdout with serde_json), so anything printed by a library in front of it
    # breaks the app, not just this check.
    print("__invalid_json__:{}".format(error))
    raise SystemExit(0)
if isinstance(data, list):
    turns = data
else:
    turns = data.get("turns") or data.get("speakers") or []
print(len(turns))
PY
)
    if [[ "$turns" == __invalid_json__:* ]]; then
      record FAIL "verify.entrypoint.$label" "stdout is not one JSON document (${turns#__invalid_json__:}); the app parses the whole stdout"
    elif [[ "$require_turns" == "1" && "${turns:-0}" -lt 1 ]]; then
      record FAIL "verify.entrypoint.$label" "ran but detected no speaker turn on real speech"
    else
      record PASS "verify.entrypoint.$label" "pyannote_diarize.py exit 0, $turns turn(s)"
    fi
  else
    record FAIL "verify.entrypoint.$label" "see $EVIDENCE/entrypoint-$label.stderr.log"
  fi

  local order code
  for order in pyannote-first torchcodec-first; do
    if [[ "$order" == "pyannote-first" ]]; then
      code='import pyannote.audio, torchcodec, torch, torchaudio; print("ok")'
    else
      code='import torchcodec, torch, torchaudio, pyannote.audio; print("ok")'
    fi
    if env -i PATH="/usr/bin:/bin" PYTHONHOME="$runtime" \
        PYTHONPATH="$stdlib:$stdlib/lib-dynload:$stdlib/site-packages" \
        PYTHONNOUSERSITE=1 "$runtime/bin/python3" -c "$code" \
        >"$EVIDENCE/import-$label-$order.log" 2>&1; then
      record PASS "verify.import.$label.$order"
    else
      record FAIL "verify.import.$label.$order" "see $EVIDENCE/import-$label-$order.log"
    fi
  done

  local versions
  versions=$(env -i PATH="/usr/bin:/bin" PYTHONHOME="$runtime" \
      PYTHONPATH="$stdlib:$stdlib/lib-dynload:$stdlib/site-packages" \
      PYTHONNOUSERSITE=1 "$runtime/bin/python3" -c \
      'from importlib.metadata import version; print("torch", version("torch"), "| torchaudio", version("torchaudio"), "| torchcodec", version("torchcodec"), "| pyannote.audio", version("pyannote.audio"), "| numpy", version("numpy"))' \
      2>&1 | tail -1)
  record INFO "verify.versions.$label" "$versions"

  local signed invalid
  signed=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["signed_valid"])' "$EVIDENCE/audit-$label.json")
  invalid=$(python3 -c 'import json,sys;print(len([i for i in json.load(open(sys.argv[1]))["violations"] if i["kind"] in ("invalid_signature","invalid_bundle_signature")]))' "$EVIDENCE/audit-$label.json")
  if [[ "${invalid:-1}" -eq 0 && "${signed:-0}" -gt 0 ]]; then
    record PASS "verify.signatures.$label" "$signed Mach-O verified, 0 invalid"
  else
    record FAIL "verify.signatures.$label" "$invalid invalid signature(s)"
  fi
  local binary verify_output
  for binary in "$runtime/bin/python3" "$runtime/lib/libpython$PYTHON_VERSION.dylib"; do
    if /usr/bin/codesign --verify --strict "$binary" >"$EVIDENCE/signature-$label-$(basename "$binary").log" 2>&1; then
      record PASS "verify.signature.$label.$(basename "$binary")"
    elif grep -qE "not signed at all|code object is not signed" "$EVIDENCE/signature-$label-$(basename "$binary").log"; then
      # The audit treats an unsigned third-party binary as a warning, never as
      # a violation: conda's osx-64 python ships unsigned, and an Intel payload
      # cannot be notarized code. A *broken* signature still fails below.
      record INFO "verify.signature.$label.$(basename "$binary")" "unsigned (dependency build ships it unsigned; audit reports unsigned_macho)"
    else
      record FAIL "verify.signature.$label.$(basename "$binary")" "see $EVIDENCE/signature-$label-$(basename "$binary").log"
    fi
  done

  if [[ -n "$SOURCE_FRAMEWORK" ]]; then
    local source_python_o="$SOURCE_FRAMEWORK/lib/python$PYTHON_VERSION/config-$PYTHON_VERSION-darwin/python.o"
    local payload_python_o="$runtime/lib/python$PYTHON_VERSION/config-$PYTHON_VERSION-darwin/python.o"
    if [[ -f "$source_python_o" && -f "$payload_python_o" ]]; then
      if [[ "$(shasum -a 256 "$source_python_o" | awk '{print $1}')" \
            == "$(shasum -a 256 "$payload_python_o" | awk '{print $1}')" ]]; then
        record PASS "verify.provenance.$label" "python.o carried through unchanged"
      else
        record FAIL "verify.provenance.$label" "python.o differs from the input framework"
      fi
    fi
  fi
}

relocation_check() {
  local root=$1
  local relocated="$WORK_DIR/relocated payload with space/pyannote"
  rm -rf "$WORK_DIR/relocated payload with space"
  mkdir -p "$(dirname "$relocated")"
  /usr/bin/ditto "$root" "$relocated"
  verify_payload "$relocated" relocated
  rm -rf "$WORK_DIR/relocated payload with space"
}

snapshot_sources() {
  python3 - "$SCRIPT_DIR" "$1" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

scripts, output = map(Path, sys.argv[1:])
names = ["local_packaging_gate.sh", "setup_bundled_pyannote.sh",
         "lib/pyannote_portable_runtime.py", "package_pyannote_asset.sh",
         "pyannote_diarize.py", "release_readiness.sh", "test_pyannote_portable_runtime.py",
         "test_setup_pyannote_staging.py", "test_pyannote_review_regressions.py"]
output.write_text(json.dumps({name: hashlib.sha256((scripts / name).read_bytes()).hexdigest()
                              for name in names}, indent=2, sort_keys=True) + "\n")
PY
}

EVIDENCE=""
AUDIO_IS_SPEECH=0
RESOURCE_ROOT=""

if ! preflight; then
  echo "" >&2
  echo "gate aborted in preflight: no expensive step was started" >&2
  exit 1
fi

EVIDENCE="$WORK_DIR/evidence/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$EVIDENCE"
record PASS evidence.dir "$EVIDENCE"
if ! snapshot_sources "$EVIDENCE/source-before.json"; then
  record FAIL provenance "cannot record the exact source inputs"
  exit 1
fi

if [[ -n "$VERIFY_ONLY" ]]; then
  RESOURCE_ROOT=$VERIFY_ONLY
  if [[ ! -d "$RESOURCE_ROOT" ]]; then
    record FAIL verify.root "no such resource root: $RESOURCE_ROOT"
  else
    verify_payload "$RESOURCE_ROOT" original
    relocation_check "$RESOURCE_ROOT"
  fi
else
  run_unit_entrypoints
  if [[ $UNIT_ONLY -eq 0 && $STATUS -eq 0 ]]; then
    if hydrate; then
      verify_payload "$RESOURCE_ROOT" original
      relocation_check "$RESOURCE_ROOT"
    fi
  fi
fi

if snapshot_sources "$EVIDENCE/source-after.json" \
    && cmp -s "$EVIDENCE/source-before.json" "$EVIDENCE/source-after.json"; then
  record PASS provenance "source hashes unchanged throughout this run"
else
  record FAIL provenance "source inputs changed during the gate; repeat on a frozen tree"
fi

{
  printf 'gate=local-packaging\n'
  printf 'mode=%s\n' "$([[ $UNIT_ONLY -eq 1 ]] && echo units || { [[ -n "$VERIFY_ONLY" ]] && echo verify || echo full-hydration; })"
  printf 'generated_at_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'work_dir=%s\n' "$WORK_DIR"
  printf 'resource_root=%s\n' "${RESOURCE_ROOT:-}"
  printf 'host_kernel_arch=%s\n' "$HOST_ARCH"
  printf 'target=%s\n' "$TARGET_TRIPLE"
  printf 'expect_arch=%s\n' "$EXPECT_ARCH"
  printf 'process_mode=%s\n' "$PROCESS_MODE"
  printf 'gate_python=%s (%s)\n' "$GATE_PYTHON" "$(command -v "$GATE_PYTHON" 2>/dev/null)"
  printf 'python_version=%s\n' "$PYTHON_VERSION"
  printf 'conda_subdir=%s\n' "${CONDA_SUBDIR:-unset}"
  printf 'conda_pkgs_dirs=%s\n' "${CONDA_PKGS_DIRS:-unset}"
  printf 'stage_dir=%s\n' "${STAGE_DIR_FROM_LOG:-}"
  printf 'host=%s macOS %s\n' "$(uname -m)" "$(sw_vers -productVersion)"
  printf 'xcode=%s\n' "$(xcodebuild -version 2>/dev/null | head -1)"
  printf 'status=%s\n' "$([[ $STATUS -eq 0 ]] && echo PASS || echo FAIL)"
  printf 'checks:\n'
  printf "%b" "$CHECKS" | sed 's/^/  - /'
} >"$EVIDENCE/gate-summary.txt"

printf '\n'
if [[ $STATUS -eq 0 ]]; then
  printf 'GATE RESULT: PASS (evidence: %s)\n' "$EVIDENCE"
else
  printf 'GATE RESULT: FAIL (evidence: %s)\n' "$EVIDENCE"
fi
if [[ $KEEP -eq 0 && $STATUS -ne 0 ]]; then
  printf 'Hydrated output kept for inspection: %s\n' "${RESOURCE_ROOT:-}"
fi
exit $STATUS
