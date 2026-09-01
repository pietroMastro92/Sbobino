#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{LanguageCode, TranscriptionLanguagePolicy, WhisperOptions};
use sbobino_infrastructure::adapters::whisper_cpp::WhisperCppEngine;

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";

fn transcription_policy(code: &str) -> TranscriptionLanguagePolicy {
    let preferred_language = LanguageCode::from_code(code);
    TranscriptionLanguagePolicy {
        adaptive_detection: preferred_language.is_auto(),
        preferred_language,
    }
}

fn write_executable_script(path: &Path, content: &str) {
    std::fs::write(path, content).expect("failed to write script");

    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .expect("failed to read script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("failed to chmod script");
}

fn write_speech_wav(path: &Path, seconds: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create wav");
    for index in 0..(seconds * 16_000) {
        let sample = ((index as f32 * 0.03).sin() * 12_000.0) as i16;
        writer
            .write_sample(sample)
            .expect("failed to write wav sample");
    }
    writer.finalize().expect("failed to finalize wav");
}

#[tokio::test]
async fn transcribe_collects_lines_from_stdout_and_stderr() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *"-l en"*) ;;
  *) echo "expected explicit -l en, got: $*" 1>&2; exit 41 ;;
esac
echo "init: loading model"
echo "[00:00:00.000 --> 00:00:01.000] first line"
echo "[00:00:01.000 --> 00:00:02.000] second line" 1>&2
echo "- third line"
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.text.contains("first line"));
    assert!(transcript.text.contains("second line"));

    let lines = emitted.lock().expect("emit lock poisoned").clone();
    assert!(lines
        .iter()
        .any(|line| line == &format!("{DELTA_REPLACE_PREFIX}first line")));
    assert!(lines
        .iter()
        .any(|line| line.contains("first line\nsecond line")));
}

#[tokio::test]
async fn transcribe_prefers_generated_txt_output_when_available() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  fi
  shift
done
echo "[00:00:00.000 --> 00:00:01.000] partial stdout line"
if [ -n "$out" ]; then
  printf "final line from txt\nanother final line\n" > "${out}.txt"
fi
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.text.contains("final line from txt"));
    assert!(transcript.text.contains("another final line"));
    assert!(
        !transcript.text.contains("partial stdout line"),
        "expected file output to override noisy stream output"
    );
}

#[tokio::test]
async fn transcribe_returns_stderr_on_failure() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo "fatal: runtime crash" 1>&2
exit 2
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let error = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("transcription should fail");

    match error {
        ApplicationError::SpeechToText(message) => {
            assert!(
                message.contains("fatal: runtime crash"),
                "expected stderr details in error, got: {message}"
            );
        }
        other => panic!("unexpected error variant: {other}"),
    }
}

#[tokio::test]
async fn transcribe_retries_in_cpu_mode_after_gpu_failure() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
cpu_mode=0
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  elif [ "$1" = "-ng" ]; then
    cpu_mode=1
  fi
  shift
done

if [ "$cpu_mode" -eq 0 ]; then
  echo "ggml_metal_buffer_init: error: failed to allocate buffer, size = 2.94 MiB" 1>&2
  exit 139
fi

if [ -n "$out" ]; then
  printf "cpu fallback transcription\n" > "${out}.txt"
fi
echo "[00:00:00.000 --> 00:00:01.000] cpu fallback transcription"
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed after cpu fallback");

    assert!(transcript.text.contains("cpu fallback transcription"));
    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(
        !emitted_lines
            .iter()
            .any(|line| line.contains("Whisper fallback") || line.contains("CPU-safe mode")),
        "backend continuation must stay invisible in transcript deltas: {emitted_lines:?}"
    );
}

#[tokio::test]
async fn transcribe_retries_in_cpu_safe_mode_after_repack_runtime_failure() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
cpu_mode=0
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  elif [ "$1" = "-ng" ]; then
    cpu_mode=1
  fi
  shift
done

if [ "$cpu_mode" -eq 0 ]; then
  # Whisper prints noisy repack/runtime diagnostics on stderr before failing
  # with a non-zero exit on long audio when the GPU backend can't initialize.
  i=0
  while [ "$i" -lt 5 ]; do
    echo "repack tensor with q8_0_4x4" 1>&2
    i=$((i + 1))
  done
  echo "REPACK = 1" 1>&2
  echo "whisper_backend_init_gpu: no GPU found" 1>&2
  echo "flash attention is not supported on this device" 1>&2
  exit 1
fi

if [ -n "$out" ]; then
  printf "cpu safe transcript\n" > "${out}.txt"
fi
echo "[00:00:00.000 --> 00:00:01.000] cpu safe transcript"
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed after cpu safe retry");

    assert!(
        transcript.text.contains("cpu safe transcript"),
        "transcript should come from the cpu safe retry: {:?}",
        transcript.text
    );

    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(
        !emitted_lines
            .iter()
            .any(|line| line.contains("Whisper fallback") || line.contains("CPU-safe mode")),
        "backend continuation must stay invisible in transcript deltas: {:?}",
        emitted_lines
    );
}

#[tokio::test]
async fn nonzero_batch_preserves_written_outputs_and_recovers_only_missing_chunk() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let invocations = temp.path().join("invocations.log");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_speech_wav(&input_wav, 40);

    let invocations_path = invocations.to_string_lossy().to_string();
    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
args="$*"
input_count=0
of_count=0
cpu=0
out=""
first_out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-f" ]; then input_count=$((input_count + 1)); fi
  if [ "$1" = "-ng" ]; then cpu=1; fi
  if [ "$1" = "-of" ]; then of_count=$((of_count + 1)); shift; out="$1"; if [ -z "$first_out" ]; then first_out="$out"; fi; fi
  shift
done
echo "inputs=$of_count cpu=$cpu" >> "{invocations}"
if [ "$of_count" -gt 1 ]; then
  # Simulate a batch that saved its first output before the process failed.
  printf "batch first\n" > "${{first_out}}.txt"
  echo "ggml_metal_buffer_init: error: failed to allocate buffer" 1>&2
  exit 139
fi
printf "cpu isolated\n" > "${{out}}.txt"
exit 0
"#,
            invocations = invocations_path
        ),
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();
    let progress: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_clone = progress.clone();
    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            Some(40.0),
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(move |seconds: f32| {
                progress_clone
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("isolated missing-chunk recovery should succeed");

    assert!(transcript.text.contains("batch first"));
    assert!(transcript.text.contains("cpu isolated"));
    assert!(
        transcript
            .segments
            .windows(2)
            .all(|pair| pair[1].start_seconds.unwrap_or_default() + 0.001
                >= pair[0].start_seconds.unwrap_or_default()),
        "isolated recovery must preserve monotonic segment timestamps: {:?}",
        transcript.segments
    );
    let progress_values = progress.lock().expect("progress lock poisoned").clone();
    assert!(
        progress_values
            .windows(2)
            .all(|pair| pair[1] + 0.001 >= pair[0]),
        "isolated recovery must not reset engine progress: {:?}",
        progress_values
    );
    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(
        !emitted_lines.iter().any(|line| {
            line.contains("fallback") || line.contains("CPU-safe") || line.contains("retry")
        }),
        "backend continuation must stay invisible in transcript deltas: {emitted_lines:?}"
    );
    let invocation_lines =
        std::fs::read_to_string(&invocations).expect("script should record invocations");
    assert!(invocation_lines.contains("inputs=2 cpu=0"));
    assert!(invocation_lines.contains("inputs=1 cpu=0"));
    assert!(!invocation_lines.contains("inputs=1 cpu=1"));
    assert_eq!(
        invocation_lines
            .lines()
            .filter(|line| line.contains("inputs=2"))
            .count(),
        1,
        "the successful first output must not trigger a full-group CPU replay"
    );
}

#[tokio::test]
async fn gpu_partial_batch_fallback_retries_only_persistently_missing_chunks() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let invocations = temp.path().join("invocations.log");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_speech_wav(&input_wav, 40);

    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
inputs=""
outputs=""
input_count=0
output_count=0
cpu=0
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-f" ]; then
    shift
    input_count=$((input_count + 1))
    inputs="$inputs $1"
  elif [ "$1" = "-of" ]; then
    shift
    output_count=$((output_count + 1))
    outputs="$outputs $1"
  elif [ "$1" = "-ng" ]; then
    cpu=1
  fi
  shift
done
echo "cpu=$cpu inputs=$input_count outputs=$output_count inputs:$inputs" >> "{invocations}"

first_output=""
for out in $outputs; do
  if [ -z "$first_output" ]; then first_output="$out"; fi
done

if [ "$cpu" -eq 0 ]; then
  if [ "$input_count" -gt 1 ]; then
    # GPU decodes and persists the first chunk, then fails before the later
    # chunk.  That artifact is a confirmed checkpoint and must survive.
    printf "gpu confirmed\n" > "${{first_output}}.txt"
    echo "ggml_metal_buffer_init: error: failed to allocate buffer" 1>&2
    exit 139
  fi
  # GPU recovery for the missing chunk is persistently incomplete.  It exits
  # cleanly but leaves no artifact, forcing the CPU continuation.
  exit 0
fi

# CPU continuation should receive only the missing chunk and complete it.
printf "cpu recovered\n" > "${{first_output}}.txt"
exit 0
"#,
            invocations = invocations.to_string_lossy(),
        ),
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("auto"),
            &WhisperOptions::default(),
            Some(40.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("CPU continuation should complete the missing chunk");

    assert_eq!(transcript.text, "gpu confirmed\ncpu recovered");
    assert_eq!(
        transcript
            .text
            .lines()
            .filter(|line| *line == "gpu confirmed")
            .count(),
        1,
        "confirmed GPU text must not be duplicated by CPU fallback"
    );
    assert!(transcript.segments.windows(2).all(|pair| {
        pair[1].start_seconds.unwrap_or_default() + 0.001 >= pair[0].end_seconds.unwrap_or_default()
    }));

    let invocation_lines = std::fs::read_to_string(&invocations)
        .expect("script should record GPU and CPU invocations")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(
        invocation_lines.len(),
        3,
        "expected GPU batch, GPU recovery, CPU recovery"
    );
    assert!(invocation_lines[0].contains("cpu=0 inputs=2 outputs=2"));
    assert!(invocation_lines[1].contains("cpu=0 inputs=1 outputs=1"));
    assert!(invocation_lines[2].contains("cpu=1 inputs=1 outputs=1"));
    let first_inputs = invocation_lines[0]
        .split_once("inputs: ")
        .expect("first invocation should log input paths")
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let cpu_inputs = invocation_lines[2]
        .split_once("inputs: ")
        .expect("CPU invocation should log input paths")
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    assert_eq!(first_inputs.len(), 2);
    assert_eq!(cpu_inputs.len(), 1);
    assert_ne!(
        cpu_inputs[0], first_inputs[0],
        "CPU fallback must not replay the confirmed first GPU input: {invocation_lines:?}"
    );
}

#[tokio::test]
async fn batch_recovery_retries_all_missing_chunks_once_without_individual_invocations() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let invocations = temp.path().join("invocations.log");
    let state_file = temp.path().join("state");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_speech_wav(&input_wav, 42);
    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
outputs=""
of_count=0
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    of_count=$((of_count + 1))
    outputs="$outputs $1"
  fi
  shift
done
echo "$of_count" >> "{invocations}"
attempt=1
if [ -f "{state}" ]; then attempt=$(cat "{state}"); attempt=$((attempt + 1)); fi
echo "$attempt" > "{state}"
if [ "$attempt" -eq 1 ]; then
  # Simulate a successful process that produced no durable outputs. The
  # service must recover all unconfirmed inputs together.
  exit 0
fi
index=0
for out in $outputs; do
  index=$((index + 1))
  printf '{{"transcription":[{{"text":"recovered chunk %s","offsets":{{"from":0,"to":1000}}}}]}}' "$index" > "${{out}}.json"
done
exit 0
"#,
            invocations = invocations.to_string_lossy(),
            state = state_file.to_string_lossy(),
        ),
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("auto"),
            &WhisperOptions::default(),
            Some(42.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("grouped recovery should succeed");

    assert!(transcript.text.contains("recovered chunk 1"));
    let invocation_log =
        std::fs::read_to_string(&invocations).expect("recovery script should record invocations");
    let invocation_lines = invocation_log
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(
        invocation_lines.len(),
        2,
        "expected initial + one recovery batch"
    );
    assert_eq!(invocation_lines[0], invocation_lines[1]);
    let chunk_count = invocation_lines[0]
        .parse::<usize>()
        .expect("invocation should record input count");
    assert!(chunk_count >= 2, "fixture should produce multiple chunks");
}

#[tokio::test]
async fn adaptive_batch_cli_progress_is_intermediate_and_grounded_in_chunk_outputs() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    // Continuous voiced audio creates several adaptive chunks. The fake CLI
    // behaves like whisper.cpp's multi-input mode: it reports callback
    // percentages while each durable JSON artifact is written.
    write_speech_wav(&input_wav, 270);
    write_executable_script(
        &script_path,
        r#"#!/bin/sh
outputs=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    outputs="$outputs $1"
  fi
  shift
done

index=0
for out in $outputs; do
  index=$((index + 1))
  printf '{"transcription":[{"text":"chunk %s","offsets":{"from":0,"to":1000}}]}' "$index" > "${out}.json"
  echo "whisper_print_progress_callback: progress = 25%" 1>&2
  echo "whisper_print_progress_callback: progress = 100%" 1>&2
  # Leave enough time for the service to observe grounded progress before the
  # next chunk is confirmed; this is intentionally not an elapsed-time pulse.
  sleep 0.02
done
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let progress: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_clone = progress.clone();
    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("auto"),
            &WhisperOptions::default(),
            Some(270.0),
            Arc::new(|_line: String| {}),
            Arc::new(move |seconds: f32| {
                progress_clone
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("fake adaptive batch should succeed");

    let progress = progress.lock().expect("progress lock poisoned").clone();
    assert!(
        progress.len() >= 2,
        "expected intermediate batch progress: {progress:?}"
    );
    assert!(
        progress.first().copied().unwrap_or_default() < 20.0,
        "the first input must not claim the whole batch completed: {progress:?}"
    );
    assert!(
        progress
            .iter()
            .any(|seconds| *seconds > 20.0 && *seconds < 41.9),
        "a later input should advance the per-input cursor: {progress:?}"
    );
    assert!(
        progress.windows(2).all(|pair| pair[1] + 0.001 >= pair[0]),
        "batch progress must stay monotonic: {progress:?}"
    );
    assert!(
        progress
            .iter()
            .any(|seconds| *seconds > 0.1 && *seconds < 269.9),
        "progress must include a grounded intermediate value, got {progress:?}"
    );
    assert!(
        progress
            .iter()
            .any(|seconds| *seconds > 225.0 && *seconds < 269.9),
        "a later process group must advance after the first group without claiming total completion: {progress:?}"
    );
    assert!(transcript.text.contains("chunk 1"));
    assert!(transcript.text.contains("chunk 2"));
    assert!(transcript.segments.windows(2).all(|pair| {
        pair[1].start_seconds.unwrap_or_default() + 0.001 >= pair[0].end_seconds.unwrap_or_default()
    }));
}

#[tokio::test]
async fn cpu_safe_retry_forces_single_processor_and_no_gpu_flags() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let args_log = temp.path().join("args.log");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    let args_log_for_script = args_log.to_string_lossy().to_string();
    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
# Record every invocation's args so the test can inspect the retry attempt.
echo "INVOCATION $@ " >> "{args_log}"

out=""
cpu_mode=0
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  elif [ "$1" = "-ng" ]; then
    cpu_mode=1
  fi
  shift
done

if [ "$cpu_mode" -eq 0 ]; then
  echo "ggml_metal_buffer_init: error: failed to allocate buffer" 1>&2
  exit 139
fi

if [ -n "$out" ]; then
  printf "cpu single processor transcript\n" > "${{out}}.txt"
fi
exit 0
"#,
            args_log = args_log_for_script
        ),
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let options = WhisperOptions {
        processors: 4,
        ..WhisperOptions::default()
    };

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &options,
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed after cpu safe retry");

    assert!(
        transcript.text.contains("cpu single processor transcript"),
        "transcript should come from the cpu safe retry: {:?}",
        transcript.text
    );

    // The retry invocation (the one carrying -ng) must also carry -p 1, i.e.
    // processors overridden to single-thread even though the user asked for 4.
    let logged = std::fs::read_to_string(&args_log).expect("failed to read args log");
    let retry_invocations: Vec<&str> = logged.lines().filter(|line| line.contains("-ng")).collect();
    assert!(
        !retry_invocations.is_empty(),
        "expected at least one cpu-safe retry invocation, got log: {logged}"
    );
    let retry_args = retry_invocations.last().expect("retry invocation present");
    assert!(
        retry_args
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair[0] == "-p" && pair[1] == "1"),
        "cpu-safe retry must pass `-p 1` (processors overridden to 1), got: {retry_args}"
    );
}

#[tokio::test]
async fn no_gpu_found_with_successful_exit_stays_success_without_retry() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  fi
  shift
done

# whisper.cpp prints "no GPU found" during init when no Metal device is
# available, but on CPU it still completes successfully (exit 0). This must
# NOT be treated as a retry-worthy failure.
echo "whisper_backend_init_gpu: no GPU found" 1>&2
echo "ggml_backend_init: CPU backend ready" 1>&2

if [ -n "$out" ]; then
  printf "successful cpu run\n" > "${out}.txt"
fi
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("successful cpu run must not trigger a retry");

    assert!(
        transcript.text.contains("successful cpu run"),
        "transcript should be produced on the first attempt: {:?}",
        transcript.text
    );

    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(
        !emitted_lines
            .iter()
            .any(|line| line.contains("CPU-safe mode")),
        "no CPU-safe fallback notice should be emitted for a successful run: {:?}",
        emitted_lines
    );
}

#[tokio::test]
async fn cpu_safe_retry_keeps_engine_progress_monotonic() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
cpu_mode=0
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  elif [ "$1" = "-ng" ]; then
    cpu_mode=1
  fi
  shift
done

if [ "$cpu_mode" -eq 0 ]; then
  # First (GPU) attempt: emit a few segments that advance progress, then crash.
  echo "[00:00:10.000 --> 00:00:12.000] gpu segment one"
  echo "[00:00:12.000 --> 00:00:14.000] gpu segment two"
  echo "ggml_metal_buffer_init: error: failed to allocate buffer" 1>&2
  exit 139
fi

# CPU-safe retry: segments restart from low timestamps.
if [ -n "$out" ]; then
  printf "cpu retry transcript\n" > "${out}.txt"
fi
echo "[00:00:00.000 --> 00:00:01.000] cpu retry transcript"
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let progress: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_clone = progress.clone();

    engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(move |seconds: f32| {
                progress_clone
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("transcription should succeed after cpu safe retry");

    let progress_values = progress.lock().expect("progress lock poisoned").clone();
    assert!(
        progress_values
            .windows(2)
            .all(|pair| pair[1] + 0.001 >= pair[0]),
        "engine progress must stay monotonic across the CPU continuation: {:?}",
        progress_values
    );
}

#[tokio::test]
async fn cpu_safe_retry_failure_wraps_error_and_dedups_repeated_stderr() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
cpu_mode=0
while [ $# -gt 0 ]; do
  if [ "$1" = "-ng" ]; then
    cpu_mode=1
  fi
  shift
done

if [ "$cpu_mode" -eq 0 ]; then
  # First attempt crashes with a signal.
  exit 139
fi

# CPU-safe retry also fails, after spamming 50 identical repack lines and a
# final fatal line. The surfaced error must collapse the repetition.
i=0
while [ "$i" -lt 50 ]; do
  echo "repack tensor with q8_0_4x4" 1>&2
  i=$((i + 1))
done
echo "ggml_metal: fatal device error" 1>&2
exit 1
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let result = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await;

    let error = result.expect_err("both attempts must fail");
    let message = match error {
        ApplicationError::SpeechToText(message) => message,
        other => panic!("expected SpeechToText error, got {other:?}"),
    };

    assert!(
        message.starts_with("Whisper transcription failed after a backend retry:"),
        "final error must remain user-facing without retry mechanics, got: {message}"
    );

    let repack_count = message.matches("repack tensor with q8_0_4x4").count();
    assert!(
        repack_count <= 3,
        "repeated stderr lines must be collapsed in the surfaced error, found {repack_count} occurrences in: {message}"
    );

    assert!(
        message.contains("ggml_metal: fatal device error"),
        "the final fatal stderr line must survive dedup in the error, got: {message}"
    );
}

#[tokio::test]
async fn transcribe_collapses_consecutive_repeated_lines_in_final_output() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo "[00:00:00.000 --> 00:00:01.000] repeated line"
echo "[00:00:01.000 --> 00:00:02.000] repeated line"
echo "[00:00:02.000 --> 00:00:03.000] final line"
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    let transcript_lines: Vec<&str> = transcript.text.lines().collect();
    assert_eq!(
        transcript_lines.len(),
        2,
        "expected duplicate lines to collapse"
    );
    assert_eq!(transcript_lines[0], "repeated line");
    assert_eq!(transcript_lines[1], "final line");

    let lines = emitted.lock().expect("emit lock poisoned").clone();
    assert!(lines
        .iter()
        .any(|line| line == &format!("{DELTA_REPLACE_PREFIX}repeated line")));
    assert!(lines
        .iter()
        .any(|line| line.contains("repeated line\nrepeated line")));
    assert!(lines.last().is_some_and(|line| line.contains("final line")));
}

#[tokio::test]
async fn transcribe_ignores_cli_progress_percent_without_segments() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo "whisper_print_progress_callback: progress = 57%"
echo "[00:00:00.000 --> 00:00:04.000] first segment"
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let progress_updates: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_updates_clone = progress_updates.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            Some(120.0),
            Arc::new(|_line: String| {}),
            Arc::new(move |seconds: f32| {
                progress_updates_clone
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.text.contains("first segment"));
    assert_eq!(
        progress_updates
            .lock()
            .expect("progress lock poisoned")
            .as_slice(),
        &[4.0],
        "expected progress to advance only from finalized segment timing",
    );
}

#[tokio::test]
async fn transcribe_emits_progress_from_comma_decimal_timestamps() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf '[00:00:00,000 --> 00:00:02,500] first segment\n'
printf '[00:00:02,500 --> 00:00:04,000] second segment\n'
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let progress_updates: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_updates_clone = progress_updates.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(10.0),
            Arc::new(|_line: String| {}),
            Arc::new(move |seconds: f32| {
                progress_updates_clone
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.text.contains("first segment"));
    assert!(transcript.text.contains("second segment"));
    assert_eq!(
        progress_updates
            .lock()
            .expect("progress lock poisoned")
            .as_slice(),
        &[2.5, 4.0],
        "expected progress to advance from comma-formatted segment timestamps",
    );
}

#[tokio::test]
async fn transcribe_passes_whisper_options_to_cli() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
translate=0
split_on_word=0
print_colors=0
json_full=0
max_context=""
threads=""
processors=""
temp=""
entropy=""
logprob=""
word=""
best_of=""
beam_size=""
temp_inc=""
no_speech=""
prompt=""

while [ $# -gt 0 ]; do
  case "$1" in
    -of) shift; out="$1" ;;
    -tr) translate=1 ;;
    -sow) split_on_word=1 ;;
    -pc) print_colors=1 ;;
    -ojf) json_full=1 ;;
    -mc) shift; max_context="$1" ;;
    -t) shift; threads="$1" ;;
    -p) shift; processors="$1" ;;
    -tp) shift; temp="$1" ;;
    -tpi) shift; temp_inc="$1" ;;
    -et) shift; entropy="$1" ;;
    -lpt) shift; logprob="$1" ;;
    -nth) shift; no_speech="$1" ;;
    -wt) shift; word="$1" ;;
    -bo) shift; best_of="$1" ;;
    -bs) shift; beam_size="$1" ;;
    --prompt) shift; prompt="$1" ;;
  esac
  shift
done

if [ -n "$out" ]; then
  printf "tr=%s sow=%s pc=%s ojf=%s mc=%s t=%s p=%s tp=%s tpi=%s et=%s lpt=%s nth=%s wt=%s bo=%s bs=%s prompt=%s\n" \
    "$translate" "$split_on_word" "$print_colors" "$json_full" "$max_context" "$threads" "$processors" \
    "$temp" "$temp_inc" "$entropy" "$logprob" "$no_speech" "$word" "$best_of" "$beam_size" "$prompt" > "${out}.txt"
fi
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("it"),
            &WhisperOptions {
                translate_to_english: true,
                no_context: true,
                split_on_word: true,
                temperature: 0.35,
                temperature_increment_on_fallback: 0.15,
                entropy_threshold: 2.2,
                logprob_threshold: -0.9,
                no_speech_threshold: 0.74,
                word_threshold: 0.2,
                best_of: 7,
                beam_size: 1,
                threads: 6,
                processors: 2,
                prompt: Some("Meeting about launch".to_string()),
                ..WhisperOptions::default()
            },
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.text.contains("tr=1"));
    assert!(transcript.text.contains("sow=1"));
    assert!(transcript.text.contains("pc=1"));
    assert!(transcript.text.contains("ojf=1"));
    assert!(transcript.text.contains("mc=0"));
    assert!(transcript.text.contains("t=6"));
    assert!(transcript.text.contains("p=2"));
    assert!(transcript.text.contains("tp=0.35"));
    assert!(transcript.text.contains("tpi=0.15"));
    assert!(transcript.text.contains("et=2.2"));
    assert!(transcript.text.contains("lpt=-0.9"));
    assert!(transcript.text.contains("nth=0.74"));
    assert!(transcript.text.contains("wt=0.2"));
    assert!(transcript.text.contains("bo=7"));
    assert!(transcript.text.contains("bs="));
    assert!(transcript.text.contains("prompt=Meeting about launch"));
}

#[tokio::test]
async fn transcribe_emits_colorized_preview_without_polluting_final_text() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf '[00:00:00.000 --> 00:00:01.000] \033[32mconfident\033[0m\n'
printf '[00:00:01.000 --> 00:00:02.000] \033[33mcareful\033[0m\n'
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.text, "confident\ncareful");

    let lines = emitted.lock().expect("emit lock poisoned").clone();
    assert!(
        lines.iter().all(|line| !line.contains('\u{001b}')),
        "ANSI escapes must never enter preview deltas: {lines:?}"
    );
    assert!(
        lines.last().is_some_and(|line| line.contains("careful")),
        "expected latest preview snapshot to contain plain text"
    );
}

#[tokio::test]
async fn transcribe_prefers_json_full_tokens_for_confidence_and_word_offsets() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  fi
  shift
done

if [ -n "$out" ]; then
  printf "hello world\n" > "${out}.txt"
  cat > "${out}.json" <<'EOF'
{
  "result": {
    "language": "it",
    "probability": 0.77,
    "confidence": 0.88
  },
  "transcription": [
    {
      "offsets": { "from": 0, "to": 1200 },
      "text": "hello world",
      "probability": 0.66,
      "confidence": 0.55,
      "tokens": [
        {
          "text": "hello",
          "offsets": { "from": 0, "to": 500 },
          "p": 0.91
        },
        {
          "text": "world",
          "offsets": { "from": 600, "to": 1200 },
          "p": 0.42
        }
      ]
    }
  ]
}
EOF
fi
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.text, "hello world");
    assert_eq!(transcript.segments.len(), 1);

    let segment = &transcript.segments[0];
    assert_eq!(segment.text, "hello world");
    assert_eq!(segment.start_seconds, Some(0.0));
    assert_eq!(segment.end_seconds, Some(1.2));
    assert_eq!(segment.language_code.as_deref(), Some("it"));
    assert_eq!(segment.language_confidence, None);
    assert_eq!(segment.words.len(), 2);
    assert_eq!(segment.words[0].text, "hello");
    assert_eq!(segment.words[0].start_seconds, Some(0.0));
    assert_eq!(segment.words[0].end_seconds, Some(0.5));
    assert_eq!(segment.words[0].confidence, Some(0.91));
    assert_eq!(segment.words[1].text, "world");
    assert_eq!(segment.words[1].start_seconds, Some(0.6));
    assert_eq!(segment.words[1].end_seconds, Some(1.2));
    assert_eq!(segment.words[1].confidence, Some(0.42));
}

#[tokio::test]
async fn transcribe_preserves_explicit_language_probability_without_token_aliases() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-of" ]; then
    shift
    out="$1"
  fi
  shift
done

if [ -n "$out" ]; then
  printf "ciao mondo\n" > "${out}.txt"
  cat > "${out}.json" <<'EOF'
{
  "result": {
    "language": "it",
    "language_probability": 0.73,
    "probability": 0.99,
    "confidence": 0.88
  },
  "transcription": [
    {
      "offsets": { "from": 0, "to": 1000 },
      "text": "ciao mondo",
      "probability": 0.21,
      "confidence": 0.12,
      "tokens": [
        {
          "text": "ciao",
          "offsets": { "from": 0, "to": 450 },
          "p": 0.31
        },
        {
          "text": "mondo",
          "offsets": { "from": 500, "to": 1000 },
          "p": 0.41
        }
      ]
    }
  ]
}
EOF
fi
exit 0
"#,
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-base.bin",
            &transcription_policy("en"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    let segment = transcript
        .segments
        .first()
        .expect("explicit language fixture should produce one segment");
    assert_eq!(segment.language_code.as_deref(), Some("it"));
    assert_eq!(segment.language_confidence, Some(0.73));
    assert_eq!(segment.words[0].confidence, Some(0.31));
    assert_eq!(segment.words[1].confidence, Some(0.41));
}
