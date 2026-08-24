#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)

if (( $# > 0 )); then
  files=("$@")
else
  files=(
    "$repo_root/sbobino_desktop/scripts/package_windows_runtime_asset.ps1"
    "$repo_root/sbobino_desktop/scripts/package_windows_pyannote_runtime.ps1"
    "$repo_root/sbobino_desktop/scripts/windows_release_readiness.ps1"
    "$repo_root/sbobino_desktop/scripts/release_windows_whisper_live_smoke.ps1"
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

hardcoded_generator=$(
  grep -EnH 'Visual Studio [0-9]+ [0-9]{4}' "${files[@]}" || true
)
if [[ -n "$hardcoded_generator" ]]; then
  printf 'hardcoded Visual Studio generator (discover via cmake -E capabilities):\n%s\n' \
    "$hardcoded_generator" >&2
  exit 1
fi

package_script=""
if [[ "$(basename -- "${files[0]}")" == "package_windows_runtime_asset.ps1" ]]; then
  package_script="${files[0]}"
fi

if [[ -n "$package_script" ]]; then
  for required in \
    'function Find-CMakeVisualStudioGenerator' \
    '-E capabilities' \
    'Sort-Object Version, Year -Descending' \
    '$generatorName = Find-CMakeVisualStudioGenerator' \
    'ConvertFrom-Json' \
    'platformSupport' \
    "Version = [int]\$match.Groups['version'].Value" \
    "Year = [int]\$match.Groups['year'].Value" \
    '$visualStudioGenerators.Count -eq 0' \
    'CMake exposes no supported Visual Studio generator'; do
    if ! grep -Fq -- "$required" "$package_script"; then
      printf 'missing CMake Visual Studio generator discovery contract: %s\n' "$required" >&2
      exit 1
    fi
  done
  for required_export in \
    '"parakeet_capi_transcribe_path_json"' \
    '"parakeet_capi_transcribe_pcm_batch_json_lang"'; do
    if ! grep -Fq -- "$required_export" "$package_script"; then
      printf 'missing Parakeet batch worker C-API export contract: %s\n' "$required_export" >&2
      exit 1
    fi
  done
  for required in \
    'function Checkout-WhisperSource' \
    'function Build-WhisperBinaries' \
    'function Find-X64SdlFile' \
    'function Find-SdlCMakePackage' \
    'Find-OneFile $SdlRoot "sdl2-config.cmake"' \
    'Find-X64SdlFile $SdlRoot "SDL2main.lib"' \
    'Find-X64SdlFile $SdlRoot "SDL2.lib"' \
    'whisper-stream-audio-file.patch' \
    'whisper-stream-fifo.patch' \
    'whisper-stream-backlog.patch' \
    'whisper-stream-finalization.patch' \
    'whisper-stream-lossless-drain.patch' \
    '-DWHISPER_SDL2=ON'; do
    if ! grep -Fq -- "$required" "$package_script"; then
      printf 'missing patched Windows Whisper live contract: %s\n' "$required" >&2
      exit 1
    fi
  done
  if grep -Fq -- 'Find-OneFile $SdlRoot "SDL2Config.cmake"' "$package_script"; then
    printf 'Windows Whisper packaging must use the official hyphenated sdl2-config.cmake package\n' >&2
    exit 1
  fi
  if ! grep -Fq -- 'SBOBINO_WHISPER_REPLAY_WAV' \
      "$repo_root/sbobino_desktop/scripts/patches/whisper-stream-audio-file.patch"; then
    printf 'Windows Whisper replay patch is missing its deterministic input hook\n' >&2
    exit 1
  fi
  lossless_patch="$repo_root/sbobino_desktop/scripts/patches/whisper-stream-lossless-drain.patch"
  for required in \
    'std::vector<float> expanded' \
    'std::thread backlog_monitor' \
    'inference_backlog_failed = true'; do
    if ! grep -Fq -- "$required" "$lossless_patch"; then
      printf 'Windows Whisper lossless watchdog contract is missing: %s\n' "$required" >&2
      exit 1
    fi
  done
  regex_literal="'^Visual Studio (?<version>\d+) (?<year>\d{4})\$'"
  if ! grep -Fq -- "$regex_literal" "$package_script"; then
    printf 'missing exact Visual Studio generator name regex contract\n' >&2
    exit 1
  fi
fi

if (( $# == 0 )); then
  whisper_live_script="$repo_root/sbobino_desktop/scripts/release_windows_whisper_live_smoke.ps1"
  for required in \
    'whisper_live_model.json' \
    'ConvertFrom-Json' \
    'immutable URL and SHA-256' \
    'manifest.url'; do
    if ! grep -Fq -- "$required" "$whisper_live_script"; then
      printf 'Windows Whisper live smoke must consume the pinned model manifest: %s\n' "$required" >&2
      exit 1
    fi
  done
  pyannote_script="$repo_root/sbobino_desktop/scripts/package_windows_pyannote_runtime.ps1"
  if ! grep -Fq -- '$ffmpegArchive = $FfmpegArchivePath' "$pyannote_script"; then
    printf 'Windows Pyannote packaging must reuse the staged speech runtime archive\n' >&2
    exit 1
  fi
  for workflow in \
    "$repo_root/.github/workflows/release.yml" \
    "$repo_root/.github/workflows/windows-port.yml"; do
    if ! grep -Fq -- '-FfmpegArchivePath' "$workflow"; then
      printf 'Windows workflow does not pass the staged speech runtime to Pyannote: %s\n' \
        "$workflow" >&2
      exit 1
    fi
  done
fi

printf 'Windows PowerShell source contract passed for %d files.\n' "${#files[@]}"
