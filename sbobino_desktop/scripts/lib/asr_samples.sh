#!/usr/bin/env bash

export LC_ALL=C

asr_fail() {
  echo "error: $*" >&2
  exit 1
}

asr_repo_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd
}

asr_default_artemis_audio() {
  local root_dir=$1
  local candidate
  for candidate in \
    "$root_dir/../../sbobino_website/assets/artemis-ii-names-new-craters-integrity-and-carroll.mp3" \
    "$root_dir/../../sbobino_website/assets/demo-audio.mp3"; do
    if [[ -f "$candidate" ]]; then
      cd "$(dirname "$candidate")" && printf '%s/%s\n' "$(pwd)" "$(basename "$candidate")"
      return 0
    fi
  done
  return 1
}

asr_default_parakeet_fixture_dir() {
  local app_id=${SBOBINO_APP_ID:-com.sbobino.desktop}
  printf '%s\n' "$HOME/Library/Application Support/$app_id/parakeet-fixtures/tests/fixtures"
}

asr_resolve_source() {
  local root_dir=${1:-$(asr_repo_root)}
  local sample=${SBOBINO_ASR_SAMPLE:-artemis}

  ASR_SOURCE_KIND=$sample
  case "$sample" in
    artemis)
      if [[ -n "${SBOBINO_ARTEMIS_AUDIO:-}" ]]; then
        ASR_SOURCE_PATH=$SBOBINO_ARTEMIS_AUDIO
      elif [[ -n "${SBOBINO_PARAKEET_AUDIO:-}" ]]; then
        ASR_SOURCE_PATH=$SBOBINO_PARAKEET_AUDIO
      else
        ASR_SOURCE_PATH=$(asr_default_artemis_audio "$root_dir") \
          || asr_fail "SBOBINO_ASR_SAMPLE=artemis requires SBOBINO_ARTEMIS_AUDIO when the Artemis asset is not present"
      fi
      ;;
    parakeet_fixture)
      local fixture=${SBOBINO_PARAKEET_FIXTURE:-speech.wav}
      if [[ "$fixture" = /* ]]; then
        ASR_SOURCE_PATH=$fixture
      else
        local fixture_dir=${SBOBINO_PARAKEET_FIXTURES_DIR:-$(asr_default_parakeet_fixture_dir)}
        ASR_SOURCE_PATH="$fixture_dir/$fixture"
      fi
      ;;
    *)
      asr_fail "unsupported SBOBINO_ASR_SAMPLE '$sample' (expected artemis or parakeet_fixture)"
      ;;
  esac

  [[ "$ASR_SOURCE_PATH" = /* ]] || asr_fail "resolved ASR source must be absolute: $ASR_SOURCE_PATH"
  [[ -f "$ASR_SOURCE_PATH" ]] || asr_fail "ASR source file not found: $ASR_SOURCE_PATH"
}

asr_is_wav_16k_mono_pcm16() {
  local audio=$1
  command -v ffprobe >/dev/null 2>&1 || return 1
  local probe
  probe=$(ffprobe -v error \
    -select_streams a:0 \
    -show_entries stream=codec_name,sample_rate,channels \
    -of default=noprint_wrappers=1 "$audio" 2>/dev/null || true)
  [[ "$probe" == *"codec_name=pcm_s16le"* ]] \
    && [[ "$probe" == *"sample_rate=16000"* ]] \
    && [[ "$probe" == *"channels=1"* ]]
}

asr_prepare_wav() {
  local source_path=$1
  local output_path=$2

  command -v ffmpeg >/dev/null 2>&1 || asr_fail "missing required command: ffmpeg"

  if asr_is_wav_16k_mono_pcm16 "$source_path"; then
    ASR_NORMALIZED_WAV=$source_path
  else
    ffmpeg -y -nostdin \
      -i "$source_path" \
      -map 0:a:0 \
      -vn -sn -dn \
      -map_metadata -1 \
      -ar 16000 \
      -ac 1 \
      -c:a pcm_s16le \
      -f wav \
      "$output_path" >/dev/null 2>&1
    ASR_NORMALIZED_WAV=$output_path
  fi
}

asr_audio_duration_seconds() {
  local audio=$1
  command -v ffprobe >/dev/null 2>&1 || asr_fail "missing required command: ffprobe"
  local duration
  duration=$(LC_ALL=C ffprobe -v error \

    -show_entries format=duration \
    -of default=noprint_wrappers=1:nokey=1 \
    "$audio" 2>/dev/null || true)
  [[ -n "$duration" ]] || asr_fail "unable to determine audio duration for $audio"
  LC_ALL=C printf '%.3f\n' "$duration"

}

asr_print_source_report() {
  local duration=$1
  echo "source_kind=$ASR_SOURCE_KIND"
  echo "source_path=$ASR_SOURCE_PATH"
  echo "normalized_wav=$ASR_NORMALIZED_WAV"
  echo "duration_seconds=$duration"
}
