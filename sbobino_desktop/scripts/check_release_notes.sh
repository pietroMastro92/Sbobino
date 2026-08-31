#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <version> <notes-file> [previous-release-ref] [previous-notes-file]" >&2
}

if [[ $# -lt 2 || $# -gt 4 ]]; then
  usage
  exit 1
fi

VERSION=${1#v}
NOTES_FILE=$2
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
REPO_ROOT=$(cd "$ROOT_DIR/.." && pwd)

if [[ ! -f "$NOTES_FILE" ]]; then
  echo "Release notes file not found: $NOTES_FILE" >&2
  exit 1
fi

for section in "Fixes" "New and improved" "Compatibility" "Known issues" "Refs"; do
  if ! grep -Eq "^### ${section}$" "$NOTES_FILE"; then
    echo "Release notes are missing mandatory section: $section" >&2
    exit 1
  fi
done

previous_ref=${3:-${SBOBINO_RELEASE_NOTES_PREVIOUS_REF:-}}
if [[ -z "$previous_ref" ]]; then
  # When the checkout is still sitting on the preceding release tag (the
  # normal local-candidate starting point), HEAD^ skips over that tag and
  # incorrectly selects the release before it. Prefer an exact current tag
  # when it is different from the candidate being checked; once the candidate
  # tag itself is checked out, HEAD^ correctly resolves the prior release.
  current_tag=$(git -C "$REPO_ROOT" describe --tags --exact-match HEAD 2>/dev/null || true)
  if [[ -n "$current_tag" && "${current_tag#v}" != "$VERSION" ]]; then
    previous_ref=$current_tag
  else
    previous_ref=$(git -C "$REPO_ROOT" describe --tags --abbrev=0 "HEAD^" 2>/dev/null || true)
  fi
fi
if [[ -z "$previous_ref" ]]; then
  # Keep the check usable from a clean source archive where tags are not
  # available. Release notes still have to name the exact prior version.
  previous_patch=$(python3 - "$VERSION" <<'PY'
import re
import sys

match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", sys.argv[1])
if not match or int(match.group(3)) == 0:
    raise SystemExit(1)
print(f"v{match.group(1)}.{match.group(2)}.{int(match.group(3)) - 1}")
PY
  ) || {
    echo "Unable to determine previous release ref; pass it explicitly." >&2
    exit 1
  }
  previous_ref=$previous_patch
fi

args=(
  "$VERSION"
  "$previous_ref"
  --notes-file "$NOTES_FILE"
  --check
  --repo-root "$REPO_ROOT"
)

previous_notes=${4:-}
if [[ -z "$previous_notes" ]]; then
  candidate="$ROOT_DIR/docs/release-notes/${previous_ref#v}.md"
  if [[ -f "$candidate" ]]; then
    previous_notes=$candidate
  fi
fi
if [[ -n "$previous_notes" ]]; then
  if [[ ! -f "$previous_notes" ]]; then
    echo "Previous release notes file not found: $previous_notes" >&2
    exit 1
  fi
  args+=(--previous-notes "$previous_notes")
fi

python3 "$ROOT_DIR/scripts/generate_codex_style_release_notes.py" "${args[@]}"
echo "Release notes gate passed for Sbobino $VERSION: $NOTES_FILE"
