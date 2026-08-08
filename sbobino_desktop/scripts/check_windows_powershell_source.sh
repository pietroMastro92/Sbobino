#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)

if (( $# > 0 )); then
  files=("$@")
else
  files=(
    "$repo_root/sbobino_desktop/scripts/package_windows_runtime_asset.ps1"
    "$repo_root/sbobino_desktop/scripts/windows_release_readiness.ps1"
    "$repo_root/.github/workflows/release.yml"
    "$repo_root/.github/workflows/windows-port.yml"
  )
fi

for file in "${files[@]}"; do
  if [[ ! -f "$file" ]]; then
    printf 'missing Windows PowerShell source: %s\n' "$file" >&2
    exit 1
  fi
done

# In an interpolated PowerShell string, `$name:` is parsed as a scoped
# variable reference. A value followed by a literal colon must therefore use
# `${name}:`. Keep legitimate scoped variables (for example `$env:PATH`) out
# of this source-contract check.
ambiguous=$(
  awk '
    {
      remaining = $0
      while (match(remaining, /\$[A-Za-z_][A-Za-z0-9_]*:/)) {
        token = substr(remaining, RSTART, RLENGTH)
        scope = tolower(substr(token, 2, length(token) - 2))
        if (scope !~ /^(env|global|script|local|private|using|this)$/) {
          printf "%s:%d:%s\n", FILENAME, FNR, token
        }
        remaining = substr(remaining, RSTART + RLENGTH)
      }
    }
  ' "${files[@]}"
)

if [[ -n "$ambiguous" ]]; then
  printf 'ambiguous PowerShell variable interpolation (use ${name}:):\n%s\n' "$ambiguous" >&2
  exit 1
fi

printf 'Windows PowerShell source contract passed for %d files.\n' "${#files[@]}"
