#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 || "$(uname -s)" != Darwin ]]; then
  echo "Usage (macOS 15+): bash $0 <absolute proof/build directory>" >&2
  exit 2
fi
PROOF_ROOT=$1
[[ "$PROOF_ROOT" = /* ]] || { echo "An absolute output path is required" >&2; exit 2; }
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
mkdir -p "$PROOF_ROOT"/{tmp,module-cache,swift-cache}
export TMPDIR="$PROOF_ROOT/tmp"
export SBOBINO_TEST_TMPDIR="$TMPDIR"
export CLANG_MODULE_CACHE_PATH="$PROOF_ROOT/module-cache"
export SWIFTPM_MODULECACHE_OVERRIDE="$CLANG_MODULE_CACHE_PATH"
STATUS=0
xcrun swift test --disable-keychain --disable-netrc --only-use-versions-from-resolved-file \
  --package-path "$SCRIPT_DIR/fluid_diarizer" --scratch-path "$PROOF_ROOT/swift-build" \
  --cache-path "$PROOF_ROOT/swift-cache" --configuration release --jobs 2 \
  > "$PROOF_ROOT/swift-test.log" 2>&1 || STATUS=$?
python3 - "$PROOF_ROOT" "$SCRIPT_DIR" "$STATUS" <<'PY'
import hashlib, json, platform, subprocess, sys
from pathlib import Path
root, scripts, code = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3])
package = scripts/'fluid_diarizer'
sources = [p for p in package.rglob('*') if p.is_file()
           and '.build' not in p.relative_to(package).parts
           and (p.suffix in ('.swift', '.json') or p.name == 'Package.resolved')]
binary = root/'swift-build/release/sbobino-diarizer'
files = sources + [scripts/'test_fluid_diarizer_native.sh'] + ([binary] if binary.is_file() else [])
record = dict(exit_code=code, platform=platform.platform(), machine=platform.machine(),
              source_sha=subprocess.check_output(['git', '-C', str(scripts), 'rev-parse', 'HEAD'], text=True).strip(),
              swift=subprocess.check_output(['xcrun', 'swift', '--version'], text=True).strip(),
              scope='native helper compilation and unit tests only; no model/microphone/quality claim',
              sha256={str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(files)})
(root/'native-tests.json').write_text(json.dumps(record, indent=2)+'\n')
print(json.dumps(dict(exit_code=code, platform=record['platform'], machine=record['machine'])))
PY
cat "$PROOF_ROOT/swift-test.log"
exit "$STATUS"
