#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT_DIR/scripts/lib/asr_samples.sh"

MODEL_FILENAME=${SBOBINO_PARAKEET_MODEL:-tdt-0.6b-v3-q4_k.gguf}
MODEL_URL="https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/$MODEL_FILENAME"
NEMOTRON_STREAMING_Q4_MODEL="nemotron-3.5-asr-streaming-0.6b-q4_k.gguf"
NEMOTRON_STREAMING_Q4_URL="https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/$NEMOTRON_STREAMING_Q4_MODEL"

fail() {
  echo "error: $*" >&2
  exit 1
}

is_positive_integer() {
  [[ "${1:-}" =~ ^[1-9][0-9]*$ ]]
}

is_nonnegative_integer() {
  [[ "${1:-}" =~ ^[0-9]+$ ]]
}

valid_worker_snapshot_row() {
  is_positive_integer "${1:-}" &&
    is_positive_integer "${2:-}" &&
    is_nonnegative_integer "${3:-}"
}

format_mib_from_kib() {
  local kib=$1
  is_nonnegative_integer "$kib" || return 1
  LC_NUMERIC=C awk -v kib="$kib" 'BEGIN { printf "%.2f", kib / 1024.0 }'
}

format_mib_from_bytes() {
  local bytes=$1
  is_nonnegative_integer "$bytes" || return 1
  LC_NUMERIC=C awk -v bytes="$bytes" 'BEGIN { printf "%.2f", bytes / (1024.0 * 1024.0) }'
}

worker_process_group_snapshot_from_ps() {
  local worker_path=$1
  awk -v worker="$worker_path" '
    NF >= 4 {
      pid = $1
      process_group = $2
      rss_kib = $3
      $1 = $2 = $3 = ""
      sub(/^[[:space:]]+/, "")
      command = $0
      if (index(command, worker) == 1 &&
          (pid !~ /^[1-9][0-9]*$/ ||
           process_group !~ /^[1-9][0-9]*$/ ||
           rss_kib !~ /^[0-9]+$/)) {
        printf "%s\t%s\t%s\n", pid, process_group, rss_kib
        next
      }
      if (pid ~ /^[1-9][0-9]*$/ &&
          process_group ~ /^[1-9][0-9]*$/ &&
          rss_kib ~ /^[0-9]+$/) {
        group_rss_kib[process_group] += rss_kib
        if (index(command, worker) == 1) {
          worker_group[pid] = process_group
        }
      }
    }
    END {
      for (pid in worker_group) {
        process_group = worker_group[pid]
        printf "%s\t%s\t%.0f\n", pid, process_group, group_rss_kib[process_group]
      }
    }
  '
}

run_worker_rss_watchdog_self_check() {
  local worker_path="/tmp/sbobino-self-check/parakeet-batch-json"
  local ps_fixture
  ps_fixture=$'4242 4242 409600 /tmp/sbobino-self-check/parakeet-batch-json --model model.gguf\n4243 4242 450560 /tmp/sbobino-self-check/parakeet-helper\n7777 7777 1024 /usr/bin/unrelated'
  local snapshot
  snapshot=$(printf '%s\n' "$ps_fixture" | worker_process_group_snapshot_from_ps "$worker_path")
  local worker_pid process_group rss_kib extra
  IFS=$'\t' read -r worker_pid process_group rss_kib extra <<< "$snapshot"
  if [[ -n "$extra" ]] || ! valid_worker_snapshot_row "$worker_pid" "$process_group" "$rss_kib"; then
    fail "watchdog self-check could not parse a valid worker PID/PGID/RSS snapshot: $snapshot"
  fi
  [[ "$worker_pid" == "4242" && "$process_group" == "4242" && "$rss_kib" == "860160" ]] ||
    fail "watchdog self-check computed the wrong process-group RSS: $snapshot"

  local malformed_worker_pid malformed_process_group malformed_rss
  local malformed_snapshot='4242\t4242\t860160'
  IFS=$'\t' read -r malformed_worker_pid malformed_process_group malformed_rss <<< "$malformed_snapshot"
  ! valid_worker_snapshot_row "$malformed_worker_pid" "$malformed_process_group" "$malformed_rss" ||
    fail "watchdog self-check accepted literal backslash snapshot separators"

  local peak_mib limit_mib
  peak_mib=$(format_mib_from_kib "$rss_kib")
  limit_mib=$(format_mib_from_bytes "$((6 * 1024 * 1024 * 1024))")
  [[ "$peak_mib" == "840.00" && "$limit_mib" == "6144.00" ]] ||
    fail "watchdog self-check reported incorrect MiB values: peak=$peak_mib limit=$limit_mib"
  echo "watchdog self-check passed: pid=$worker_pid process_group=$process_group peak=${peak_mib}MiB limit=${limit_mib}MiB"
}

if [[ "${SBOBINO_PARAKEET_WATCHDOG_SELF_CHECK:-0}" == "1" ]]; then
  run_worker_rss_watchdog_self_check
  exit 0
fi

require_abs_path() {
  local name=$1
  local value=${!name:-}
  if [[ -z "$value" ]]; then
    fail "$name must be set"
  fi
  if [[ "$value" != /* ]]; then
    fail "$name must be an absolute path: $value"
  fi
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "missing required command: $1"
  fi
}

require_abs_path SBOBINO_PARAKEET_CLI
require_abs_path SBOBINO_PARAKEET_MODELS_DIR

[[ -x "$SBOBINO_PARAKEET_CLI" ]] || fail "SBOBINO_PARAKEET_CLI is not executable: $SBOBINO_PARAKEET_CLI"
[[ -d "$SBOBINO_PARAKEET_MODELS_DIR" ]] || fail "SBOBINO_PARAKEET_MODELS_DIR is not a directory: $SBOBINO_PARAKEET_MODELS_DIR"

PARAKEET_WORKER="$(cd "$(dirname "$SBOBINO_PARAKEET_CLI")" && pwd)/parakeet-batch-json"
[[ -x "$PARAKEET_WORKER" ]] || fail "missing executable Parakeet batch worker next to CLI: $PARAKEET_WORKER"

# The adapter owns the production ceiling too.  This harness independently
# watches the actual native worker process group, so a host-level regression
# cannot silently bypass the in-process monitor.
DEFAULT_WORKER_RSS_LIMIT_BYTES=$((6 * 1024 * 1024 * 1024))
WORKER_RSS_LIMIT_BYTES=${SBOBINO_PARAKEET_WORKER_RSS_LIMIT_BYTES:-$DEFAULT_WORKER_RSS_LIMIT_BYTES}
[[ "$WORKER_RSS_LIMIT_BYTES" =~ ^[1-9][0-9]*$ ]] || fail "SBOBINO_PARAKEET_WORKER_RSS_LIMIT_BYTES must be a positive integer"
if (( WORKER_RSS_LIMIT_BYTES > DEFAULT_WORKER_RSS_LIMIT_BYTES )); then
  echo "capping SBOBINO_PARAKEET_WORKER_RSS_LIMIT_BYTES at the 6 GiB safety ceiling" >&2
  WORKER_RSS_LIMIT_BYTES=$DEFAULT_WORKER_RSS_LIMIT_BYTES
fi
export SBOBINO_PARAKEET_WORKER_RSS_LIMIT_BYTES=$WORKER_RSS_LIMIT_BYTES

# Set this for a deliberately long smoke fixture. Short normal smoke sources
# still run correctly but cannot start the long-file worker by design.
REQUIRE_WORKER_RSS_MONITOR=${SBOBINO_PARAKEET_REQUIRE_WORKER_RSS_MONITOR:-0}
case "$REQUIRE_WORKER_RSS_MONITOR" in
  0|1) ;;
  *) fail "SBOBINO_PARAKEET_REQUIRE_WORKER_RSS_MONITOR must be 0 or 1" ;;
esac

MODEL_PATH="$SBOBINO_PARAKEET_MODELS_DIR/$MODEL_FILENAME"
if [[ ! -f "$MODEL_PATH" ]]; then
  echo "missing Parakeet model: $MODEL_PATH" >&2
  echo "download manually:" >&2
  echo "  curl -L -o '$MODEL_PATH' '$MODEL_URL'" >&2
  exit 1
fi

if [[ ! -f "$SBOBINO_PARAKEET_MODELS_DIR/$NEMOTRON_STREAMING_Q4_MODEL" ]]; then
  echo "missing Parakeet NVIDIA Nemotron live model in: $SBOBINO_PARAKEET_MODELS_DIR" >&2
  echo "download manually:" >&2
  echo "  curl -L -o '$SBOBINO_PARAKEET_MODELS_DIR/$NEMOTRON_STREAMING_Q4_MODEL' '$NEMOTRON_STREAMING_Q4_URL'" >&2
  exit 1
fi

need_cmd cargo
need_cmd ffmpeg

RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-parakeet-smoke.XXXXXX")
WATCHDOG_STOP_FILE=""
WATCHDOG_PID=""
cleanup() {
  if [[ -n "$WATCHDOG_STOP_FILE" ]]; then
    touch "$WATCHDOG_STOP_FILE" 2>/dev/null || true
  fi
  if [[ -n "$WATCHDOG_PID" ]]; then
    /bin/kill "$WATCHDOG_PID" 2>/dev/null || true
    wait "$WATCHDOG_PID" 2>/dev/null || true
  fi
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

worker_process_group_snapshot() {
  # macOS ps reports RSS in KiB. Sum every member of the worker's isolated
  # process group, not just the worker leader: native decoders can create
  # helper processes and threads while loading the model.
  /bin/ps -axww -o pid=,pgid=,rss=,command= |
    worker_process_group_snapshot_from_ps "$PARAKEET_WORKER"
}

terminate_isolated_worker_group() {
  local worker_pid=$1
  local process_group=$2
  if [[ ! "$worker_pid" =~ ^[1-9][0-9]*$ || ! "$process_group" =~ ^[1-9][0-9]*$ ]]; then
    fail "RSS monitor received an invalid worker PID/process group: pid=$worker_pid group=$process_group"
  fi
  if [[ "$worker_pid" != "$process_group" ]]; then
    fail "RSS monitor refused to signal non-isolated worker PID $worker_pid in process group $process_group"
  fi

  echo "RSS watchdog: terminating Parakeet worker PID $worker_pid process group $process_group" >&2
  /bin/kill -TERM "-$process_group" 2>/dev/null || true
  sleep 0.25
  if /bin/kill -0 "-$process_group" 2>/dev/null; then
    /bin/kill -KILL "-$process_group" 2>/dev/null || true
  fi
}

run_with_worker_rss_watchdog() {
  local label=$1
  shift

  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "$label: macOS worker RSS watchdog unavailable on $(uname -s); running command without external monitor" >&2
    "$@"
    return
  fi

  local stop_file="$RUN_DIR/$label.worker-rss-stop"
  local peak_file="$RUN_DIR/$label.worker-rss-peak-kib"
  local observed_file="$RUN_DIR/$label.worker-rss-observed"
  local breach_file="$RUN_DIR/$label.worker-rss-breach"
  local malformed_file="$RUN_DIR/$label.worker-rss-malformed"
  local worker_limit_kib=$((WORKER_RSS_LIMIT_BYTES / 1024))
  rm -f "$stop_file" "$peak_file" "$observed_file" "$breach_file" "$malformed_file"

  (
    local peak_kib=0
    local observed=0
    while [[ ! -e "$stop_file" ]]; do
      local snapshot
      snapshot=$(worker_process_group_snapshot || true)
      while IFS=$'\t' read -r worker_pid process_group rss_kib extra; do
        [[ -z "$worker_pid$process_group$rss_kib$extra" ]] && continue
        if [[ -n "$extra" ]] || ! valid_worker_snapshot_row "$worker_pid" "$process_group" "$rss_kib"; then
          printf 'worker_pid=%s process_group=%s rss_kib=%s extra=%s\n' \
            "$worker_pid" "$process_group" "$rss_kib" "$extra" >> "$malformed_file"
          continue
        fi
        observed=1
        local normalized_rss_kib=$((10#$rss_kib))
        if (( normalized_rss_kib > peak_kib )); then
          peak_kib=$normalized_rss_kib
        fi
        if (( normalized_rss_kib >= worker_limit_kib )) && [[ ! -e "$breach_file" ]]; then
          local rss_bytes=$((normalized_rss_kib * 1024))
          printf 'worker_pid=%s process_group=%s rss_bytes=%s limit_bytes=%s\n' \
            "$worker_pid" "$process_group" "$rss_bytes" "$WORKER_RSS_LIMIT_BYTES" > "$breach_file"
          terminate_isolated_worker_group "$worker_pid" "$process_group"
        fi
      done <<< "$snapshot"
      sleep 0.25
    done
    printf '%s\n' "$peak_kib" > "$peak_file"
    printf '%s\n' "$observed" > "$observed_file"
  ) &
  local monitor_pid=$!
  WATCHDOG_STOP_FILE=$stop_file
  WATCHDOG_PID=$monitor_pid

  "$@" &
  local command_pid=$!
  local command_status=0
  if wait "$command_pid"; then
    command_status=0
  else
    command_status=$?
  fi
  touch "$stop_file"
  wait "$monitor_pid" || true
  WATCHDOG_STOP_FILE=""
  WATCHDOG_PID=""

  local peak_kib=0
  local observed=0
  [[ -f "$peak_file" ]] && peak_kib=$(<"$peak_file")
  [[ -f "$observed_file" ]] && observed=$(<"$observed_file")
  is_nonnegative_integer "$peak_kib" || fail "$label watchdog produced an invalid peak RSS value: $peak_kib"
  [[ "$observed" == "0" || "$observed" == "1" ]] ||
    fail "$label watchdog produced an invalid observation state: $observed"
  if [[ -s "$malformed_file" ]]; then
    fail "$label watchdog rejected malformed worker snapshot row(s): $(tr '\n' ';' < "$malformed_file")"
  fi
  if (( observed == 1 )); then
    local peak_mib limit_mib
    peak_mib=$(format_mib_from_kib "$peak_kib") ||
      fail "$label watchdog could not format peak RSS: $peak_kib KiB"
    limit_mib=$(format_mib_from_bytes "$WORKER_RSS_LIMIT_BYTES") ||
      fail "$label watchdog could not format RSS limit: $WORKER_RSS_LIMIT_BYTES bytes"
    printf '%s: peak Parakeet worker process-group RSS %s MiB (limit %s MiB)\n' \
      "$label" "$peak_mib" "$limit_mib"
  else
    echo "$label: no long-file Parakeet worker observed (short input or early failure)" >&2
  fi
  if [[ -f "$breach_file" ]]; then
    fail "$label exceeded the Parakeet worker RSS limit: $(<"$breach_file")"
  fi
  if (( REQUIRE_WORKER_RSS_MONITOR == 1 && observed != 1 )); then
    fail "$label required a long-file worker, but none was observed"
  fi
  return "$command_status"
}

WAV_AUDIO="$RUN_DIR/parakeet-smoke.wav"
asr_resolve_source "$ROOT_DIR"
asr_prepare_wav "$ASR_SOURCE_PATH" "$WAV_AUDIO"
WAV_AUDIO="$ASR_NORMALIZED_WAV"
DURATION_SECONDS=$(asr_audio_duration_seconds "$WAV_AUDIO")
asr_print_source_report "$DURATION_SECONDS"

export SBOBINO_PARAKEET_MODEL="$MODEL_FILENAME"
export SBOBINO_PARAKEET_AUDIO="$WAV_AUDIO"

cd "$ROOT_DIR"

echo "running adapter smoke with parakeet-cli=$SBOBINO_PARAKEET_CLI model=$MODEL_FILENAME"
run_with_worker_rss_watchdog adapter_smoke cargo test -p sbobino-infrastructure \
  --test parakeet_cpp_engine_tests \
  parakeet_cpp_real_smoke \
  -- --ignored --nocapture

echo "running service smoke with parakeet-cli=$SBOBINO_PARAKEET_CLI model=$MODEL_FILENAME"
run_with_worker_rss_watchdog service_smoke cargo test -p sbobino-infrastructure \
  --test parakeet_real_service_smoke_tests \
  parakeet_service_real_smoke_persists_metadata \
  -- --ignored --nocapture
