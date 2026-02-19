#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::WhisperOptions;
use sbobino_infrastructure::adapters::whisper_cpp::WhisperCppEngine;

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
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.contains("first line"));
    assert!(transcript.contains("second line"));
    assert!(transcript.contains("third line"));

    let lines = emitted.lock().expect("emit lock poisoned").clone();
    assert!(lines.iter().any(|line| line == "first line"));
    assert!(lines.iter().any(|line| line == "second line"));
    assert!(lines.iter().any(|line| line.contains("third line")));
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
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.contains("final line from txt"));
    assert!(transcript.contains("another final line"));
    assert!(
        !transcript.contains("partial stdout line"),
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
            "en",
            &WhisperOptions::default(),
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
async fn transcribe_keeps_repeated_lines_from_stream() {
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
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    let transcript_lines: Vec<&str> = transcript.lines().collect();
    assert_eq!(transcript_lines.len(), 3, "expected all streamed lines");
    assert_eq!(transcript_lines[0], "repeated line");
    assert_eq!(transcript_lines[1], "repeated line");
    assert_eq!(transcript_lines[2], "final line");

    let lines = emitted.lock().expect("emit lock poisoned").clone();
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.as_str() == "repeated line")
            .count(),
        2
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
max_context=""
threads=""
processors=""
temp=""
entropy=""
logprob=""
word=""
best_of=""
beam_size=""

while [ $# -gt 0 ]; do
  case "$1" in
    -of) shift; out="$1" ;;
    -tr) translate=1 ;;
    -sow) split_on_word=1 ;;
    -mc) shift; max_context="$1" ;;
    -t) shift; threads="$1" ;;
    -p) shift; processors="$1" ;;
    -tp) shift; temp="$1" ;;
    -et) shift; entropy="$1" ;;
    -lpt) shift; logprob="$1" ;;
    -wt) shift; word="$1" ;;
    -bo) shift; best_of="$1" ;;
    -bs) shift; beam_size="$1" ;;
  esac
  shift
done

if [ -n "$out" ]; then
  printf "tr=%s sow=%s mc=%s t=%s p=%s tp=%s et=%s lpt=%s wt=%s bo=%s bs=%s\n" \
    "$translate" "$split_on_word" "$max_context" "$threads" "$processors" \
    "$temp" "$entropy" "$logprob" "$word" "$best_of" "$beam_size" > "${out}.txt"
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
                entropy_threshold: 2.2,
                logprob_threshold: -0.9,
                word_threshold: 0.2,
                best_of: 7,
                beam_size: 1,
                threads: 6,
                processors: 2,
            },
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(transcript.contains("tr=1"));
    assert!(transcript.contains("sow=1"));
    assert!(transcript.contains("mc=0"));
    assert!(transcript.contains("t=6"));
    assert!(transcript.contains("p=2"));
    assert!(transcript.contains("tp=0.35"));
    assert!(transcript.contains("et=2.2"));
    assert!(transcript.contains("lpt=-0.9"));
    assert!(transcript.contains("wt=0.2"));
    assert!(transcript.contains("bo=7"));
    assert!(transcript.contains("bs="));
}
