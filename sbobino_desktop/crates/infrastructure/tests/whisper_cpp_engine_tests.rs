#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::WhisperOptions;
use sbobino_infrastructure::adapters::whisper_cpp::WhisperCppEngine;

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";

fn write_executable_script(path: &Path, content: &str) {
    std::fs::write(path, content).expect("failed to write script");

    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .expect("failed to read script metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("failed to chmod script");
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
            "en",
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
            "en",
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
async fn transcribe_ignores_successful_diagnostics_like_no_gpu_found() {
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
echo "whisper_backend_init_gpu: no GPU found" 1>&2
if [ -n "$out" ]; then
  printf "successful transcript\n" > "${out}.txt"
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
            "en",
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("diagnostics on stderr should not fail a successful process");

    assert_eq!(transcript.text, "successful transcript");
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
            "en",
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
async fn transcribe_returns_tail_stderr_on_large_failure() {
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
for i in $(seq 1 160); do
  echo "model-load-line-$i" 1>&2
done
echo "fatal: useful final failure" 1>&2
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
            "en",
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("transcription should fail");

    match error {
        ApplicationError::SpeechToText(message) => {
            let message_lines = message.lines().collect::<Vec<_>>();
            assert!(message.contains("fatal: useful final failure"));
            assert!(message.contains("model-load-line-160"));
            assert!(
                !message_lines.contains(&"model-load-line-1"),
                "expected long stderr to be tailed for UI readability, got: {message}"
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
            "en",
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
    assert!(emitted_lines
        .iter()
        .any(|line| line.contains("Whisper fallback CPU-safe mode")));
}

#[tokio::test]
async fn transcribe_retries_in_cpu_safe_mode_after_repack_runtime_failure() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let retry_args_path = temp.path().join("retry-args.txt");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(
        models_dir.join("ggml-large-v3-turbo-q8_0.bin"),
        b"fake model",
    )
    .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
out=""
cpu_mode=0
no_flash=0
processors=""
while [ $# -gt 0 ]; do
  case "$1" in
    -of) shift; out="$1" ;;
    -ng) cpu_mode=1 ;;
    -nfa) no_flash=1 ;;
    -p) shift; processors="$1" ;;
  esac
  shift
done

if [ "$cpu_mode" -eq 0 ]; then
  for i in $(seq 1 36); do
    echo "repack: repack tensor with q8_0_4x4" 1>&2
  done
  echo "whisper_backend_init_gpu: no GPU found" 1>&2
  echo "system_info: n_threads = 4 / 8 | MTL : EMBED_LIBRARY = 1 | CPU : NEON = 1 | REPACK = 1 |" 1>&2
  echo "whisper_print_progress_callback: progress = 5%" 1>&2
  echo "whisper_print_progress_callback: progress = 20%" 1>&2
  exit 2
fi

printf 'cpu_mode=%s no_flash=%s processors=%s\n' "$cpu_mode" "$no_flash" "$processors" > "{}"
if [ -n "$out" ]; then
  printf "cpu safe retry transcription\n" > "${{out}}.txt"
fi
echo "[00:00:00.000 --> 00:00:01.000] cpu safe retry transcription"
exit 0
"#,
            retry_args_path.to_string_lossy()
        ),
    );

    let engine = WhisperCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();
    let progress_updates: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_updates_clone = progress_updates.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "ggml-large-v3-turbo-q8_0.bin",
            "auto",
            &WhisperOptions {
                processors: 4,
                ..WhisperOptions::default()
            },
            Some(100.0),
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(move |seconds: f32| {
                progress_updates_clone
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("repack runtime failure should retry and complete in CPU-safe mode");

    assert!(transcript.text.contains("cpu safe retry transcription"));
    assert_eq!(
        std::fs::read_to_string(&retry_args_path)
            .expect("retry args should be recorded")
            .trim(),
        "cpu_mode=1 no_flash=1 processors=1"
    );
    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(emitted_lines
        .iter()
        .any(|line| line.contains("Whisper fallback CPU-safe mode")));
    let progress_updates = progress_updates.lock().expect("progress lock poisoned");
    assert!(
        progress_updates.contains(&0.0),
        "retry should reset progress before starting the CPU-safe attempt, got {progress_updates:?}"
    );
    assert!(
        progress_updates
            .iter()
            .rev()
            .any(|seconds| *seconds > 0.0 && *seconds <= 1.0),
        "retry should emit fresh low progress from the CPU-safe attempt, got {progress_updates:?}"
    );
}

#[tokio::test]
async fn long_large_turbo_q8_audio_starts_directly_in_cpu_safe_mode() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let call_count_path = temp.path().join("call-count.txt");
    let args_path = temp.path().join("args.txt");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(
        models_dir.join("ggml-large-v3-turbo-q8_0.bin"),
        b"fake model",
    )
    .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
out=""
cpu_mode=0
no_flash=0
processors=""
count=0
if [ -f "{call_count}" ]; then
  count=$(cat "{call_count}")
fi
count=$((count + 1))
printf "%s\n" "$count" > "{call_count}"

while [ $# -gt 0 ]; do
  case "$1" in
    -of) shift; out="$1" ;;
    -ng) cpu_mode=1 ;;
    -nfa) no_flash=1 ;;
    -p) shift; processors="$1" ;;
  esac
  shift
done

printf 'cpu_mode=%s no_flash=%s processors=%s\n' "$cpu_mode" "$no_flash" "$processors" > "{args}"
if [ "$cpu_mode" -ne 1 ] || [ "$no_flash" -ne 1 ] || [ "$processors" != "1" ]; then
  echo "expected direct CPU-safe mode for long large turbo q8 audio" 1>&2
  exit 7
fi

if [ -n "$out" ]; then
  printf "direct cpu safe long transcription\n" > "${{out}}.txt"
fi
echo "whisper_print_progress_callback: progress = 2%"
echo "[00:00:00.000 --> 00:00:01.000] direct cpu safe long transcription"
exit 0
"#,
            call_count = call_count_path.to_string_lossy(),
            args = args_path.to_string_lossy(),
        ),
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
            "ggml-large-v3-turbo-q8_0.bin",
            "it",
            &WhisperOptions {
                processors: 8,
                ..WhisperOptions::default()
            },
            Some(31.0 * 60.0),
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("long large turbo q8 should use direct CPU-safe mode and complete");

    assert!(transcript
        .text
        .contains("direct cpu safe long transcription"));
    assert_eq!(
        std::fs::read_to_string(&call_count_path)
            .expect("call count should be recorded")
            .trim(),
        "1",
        "direct CPU-safe mode must avoid a failing first attempt"
    );
    assert_eq!(
        std::fs::read_to_string(&args_path)
            .expect("args should be recorded")
            .trim(),
        "cpu_mode=1 no_flash=1 processors=1"
    );
    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(emitted_lines
        .iter()
        .any(|line| line.contains("Whisper CPU-safe mode for long audio")));
    assert!(
        !emitted_lines
            .iter()
            .any(|line| line.contains("Whisper fallback CPU-safe mode")),
        "direct long-audio CPU-safe path should not look like a retry"
    );
}

#[tokio::test]
async fn short_large_turbo_q8_audio_keeps_default_first_attempt() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-cli");
    let models_dir = temp.path().join("models");
    let input_wav = temp.path().join("audio.wav");
    let args_path = temp.path().join("args.txt");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(
        models_dir.join("ggml-large-v3-turbo-q8_0.bin"),
        b"fake model",
    )
    .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
out=""
cpu_mode=0
no_flash=0
while [ $# -gt 0 ]; do
  case "$1" in
    -of) shift; out="$1" ;;
    -ng) cpu_mode=1 ;;
    -nfa) no_flash=1 ;;
  esac
  shift
done

printf 'cpu_mode=%s no_flash=%s\n' "$cpu_mode" "$no_flash" > "{args}"
if [ -n "$out" ]; then
  printf "short default transcription\n" > "${{out}}.txt"
fi
echo "[00:00:00.000 --> 00:00:01.000] short default transcription"
exit 0
"#,
            args = args_path.to_string_lossy(),
        ),
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
            "ggml-large-v3-turbo-q8_0.bin",
            "it",
            &WhisperOptions::default(),
            Some(10.0 * 60.0),
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("short audio should keep default first attempt");

    assert!(transcript.text.contains("short default transcription"));
    assert_eq!(
        std::fs::read_to_string(&args_path)
            .expect("args should be recorded")
            .trim(),
        "cpu_mode=0 no_flash=0"
    );
    let emitted_lines = emitted.lock().expect("emit lock poisoned");
    assert!(
        !emitted_lines
            .iter()
            .any(|line| line.contains("Whisper CPU-safe mode for long audio")),
        "short audio must not be forced into direct CPU-safe mode"
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
  # First attempt: a GPU metal failure triggers the CPU-safe retry.
  echo "ggml_metal_buffer_init: error: failed to allocate buffer" 1>&2
  exit 139
fi

# CPU-safe retry also fails, after spamming identical repack lines and a
# final fatal line. The surfaced error must collapse the repetition.
i=0
while [ "$i" -lt 40 ]; do
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
            "en",
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await;

    let message = match result.expect_err("both attempts must fail") {
        ApplicationError::SpeechToText(message) => message,
        other => panic!("expected SpeechToText error, got {other:?}"),
    };

    assert!(
        message.contains("Whisper retry in CPU-safe mode failed:"),
        "final error must be wrapped with the CPU-safe retry prefix, got: {message}"
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
            "en",
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
async fn transcribe_uses_cli_progress_percent_without_falling_behind_segments() {
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
            "en",
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
        &[68.4],
        "expected progress to advance from whisper-cli percent and not move backwards to earlier segment timing",
    );
}

#[tokio::test]
async fn transcribe_uses_carriage_return_cli_progress_percent() {
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
printf 'whisper_print_progress_callback: progress = 5%%\rwhisper_print_progress_callback: progress = 20%%\r'
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
            "en",
            &WhisperOptions::default(),
            Some(100.0),
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
        &[5.0, 20.0],
        "expected carriage-return progress updates to drive Whisper progress",
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
            "it",
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
            "it",
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
            "en",
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
        lines
            .iter()
            .any(|line| line.contains("\u{001b}[32mconfident\u{001b}[0m")),
        "expected ANSI-colored preview output"
    );
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains("\u{001b}[33mcareful\u{001b}[0m")),
        "expected latest preview snapshot to keep color escapes"
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
  "transcription": [
    {
      "offsets": { "from": 0, "to": 1200 },
      "text": "hello world",
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
            "en",
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
