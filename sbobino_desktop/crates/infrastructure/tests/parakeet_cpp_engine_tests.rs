#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use regex::Regex;
use tempfile::tempdir;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::WhisperOptions;
use sbobino_infrastructure::adapters::parakeet_cpp::ParakeetCppEngine;

const DEFAULT_REAL_SMOKE_MODEL: &str = "tdt-0.6b-v3-q4_k.gguf";

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must be set for the Parakeet real smoke test");
    })
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
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

fn assert_valid_real_parakeet_output(transcript: &sbobino_domain::TranscriptionOutput) {
    assert!(
        !transcript.text.trim().is_empty(),
        "Parakeet real smoke produced an empty transcript"
    );
    assert!(
        !transcript.segments.is_empty(),
        "Parakeet real smoke produced no segments"
    );

    let mut has_word_timestamp = false;
    for segment in &transcript.segments {
        assert!(
            !segment.text.trim().is_empty(),
            "Parakeet real smoke produced an empty segment"
        );
        if let Some(start) = segment.start_seconds {
            assert!(start.is_finite(), "segment start timestamp is not finite");
        }
        if let Some(end) = segment.end_seconds {
            assert!(end.is_finite(), "segment end timestamp is not finite");
        }
        for word in &segment.words {
            assert!(
                !word.text.trim().is_empty(),
                "Parakeet real smoke produced an empty word"
            );
            if let Some(start) = word.start_seconds {
                assert!(start.is_finite(), "word start timestamp is not finite");
                has_word_timestamp = true;
            }
            if let Some(end) = word.end_seconds {
                assert!(end.is_finite(), "word end timestamp is not finite");
                has_word_timestamp = true;
            }
            if let Some(confidence) = word.confidence {
                assert!(
                    confidence.is_finite() && (0.0..=1.0).contains(&confidence),
                    "word confidence must be finite and between 0 and 1"
                );
            }
        }
    }

    assert!(
        has_word_timestamp,
        "Parakeet real smoke produced no word-level timestamps"
    );
}

#[tokio::test]
async fn transcribe_parses_json_words_and_emits_text() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *--stream*)
    echo '[00:00:00.000 --> 00:00:00.500] hello'
    echo '[00:00:00.500 --> 00:00:01.000] world'
    exit 0
    ;;
esac
echo 'loading model'
echo '{"text":"hello world","words":[{"w":"hello","start":0.12,"end":0.44,"conf":0.91},{"w":"world","start":0.44,"end":0.8,"conf":0.88}],"tokens":[]}'
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "it",
            &WhisperOptions::default(),
            Some(1.0),
            Arc::new(move |line: String| {
                emitted_clone.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.text, "hello world");
    assert_eq!(transcript.segments[0].words.len(), 2);
    assert_eq!(transcript.segments[0].words[0].text, "hello");
    assert_eq!(transcript.segments[0].words[0].start_seconds, Some(0.12));
    assert_eq!(transcript.segments[0].words[0].confidence, Some(0.91));
    let emitted = emitted.lock().expect("emit lock poisoned");
    let preview_deltas = emitted
        .iter()
        .filter(|line| line.starts_with("\u{001F}REPLACE:"))
        .collect::<Vec<_>>();
    assert!(
        preview_deltas.len() >= 2,
        "expected progressive preview deltas plus final transcript, got {emitted:?}"
    );
    assert!(preview_deltas[0].contains("REPLACE:hello"));
    assert!(
        preview_deltas
            .iter()
            .any(|line| line.contains("hello") && line.contains("world")),
        "expected a cumulative preview containing the final words, got {preview_deltas:?}"
    );
    assert_eq!(emitted.last().map(String::as_str), Some("hello world"));
}

#[tokio::test]
async fn transcribe_accepts_progressive_preview_from_stderr() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *--stream*)
    echo 'pk::Backend using GPU device Metal' 1>&2
    echo '[00:00:00.000 --> 00:00:00.500] ciao' 1>&2
    echo '[00:00:00.500 --> 00:00:01.000] mondo' 1>&2
    exit 0
    ;;
esac
echo '{"text":"ciao mondo","words":[{"w":"ciao","start":0.0,"end":0.4},{"w":"mondo","start":0.4,"end":0.9}]}'
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_ref = emitted.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "it",
            &WhisperOptions::default(),
            Some(1.0),
            Arc::new(move |line: String| {
                emitted_ref.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.text, "ciao mondo");
    let emitted = emitted.lock().expect("emit lock poisoned");
    assert!(
        emitted
            .iter()
            .any(|line| line.starts_with("\u{001F}REPLACE:") && line.contains("ciao")),
        "expected stderr preview delta, got {emitted:?}"
    );
}

#[tokio::test]
async fn transcribe_splits_root_words_into_multiple_timed_segments() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *--stream*)
    echo '[00:00:00.000 --> 00:00:01.000] preview'
    exit 0
    ;;
esac
echo '{"text":"one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen","words":[{"w":"one","start":0.0,"end":1.0},{"w":"two","start":1.0,"end":2.0},{"w":"three","start":2.0,"end":3.0},{"w":"four","start":3.0,"end":4.0},{"w":"five","start":4.0,"end":5.0},{"w":"six","start":5.0,"end":6.0},{"w":"seven","start":6.0,"end":7.0},{"w":"eight","start":7.0,"end":8.0},{"w":"nine","start":8.0,"end":9.0},{"w":"ten","start":9.0,"end":10.0},{"w":"eleven","start":10.0,"end":11.0},{"w":"twelve","start":11.0,"end":12.0},{"w":"thirteen","start":12.0,"end":13.0},{"w":"fourteen","start":13.0,"end":14.0},{"w":"fifteen","start":14.0,"end":15.0}]}'
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "en",
            &WhisperOptions::default(),
            Some(15.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert!(
        transcript.segments.len() >= 2,
        "expected root words to be split into multiple segments, got {:?}",
        transcript.segments
    );
    assert_eq!(transcript.segments[0].start_seconds, Some(0.0));
    assert_eq!(
        transcript
            .segments
            .last()
            .and_then(|segment| segment.end_seconds),
        Some(15.0)
    );
}

#[tokio::test]
async fn transcribe_prefers_flat_words_when_segments_have_no_words() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *--stream*)
    echo '[00:00:00.000 --> 00:00:01.000] preview'
    exit 0
    ;;
esac
echo '{"text":"first sentence. second sentence.","segments":[{"text":"first sentence. second sentence.","start":0.0,"end":4.0}],"words":[{"w":"first","start":0.0,"end":0.4},{"w":"sentence.","start":0.4,"end":1.0},{"w":"second","start":3.0,"end":3.5},{"w":"sentence.","start":3.5,"end":4.0}]}'
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "en",
            &WhisperOptions::default(),
            Some(4.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.segments.len(), 2);
    assert_eq!(transcript.segments[0].text, "first sentence.");
    assert_eq!(transcript.segments[0].words.len(), 2);
    assert_eq!(transcript.segments[1].text, "second sentence.");
    assert_eq!(transcript.segments[1].start_seconds, Some(3.0));
}

#[tokio::test]
async fn transcribe_parses_segment_json_with_words() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *--stream*)
    echo '[00:00:00.000 --> 00:00:00.500] ciao.'
    echo '[00:00:00.500 --> 00:00:01.200] mondo'
    exit 0
    ;;
esac
echo '{"text":"ciao. mondo","segments":[{"text":"ciao.","start":0.0,"end":0.5,"words":[{"text":"ciao","start":0.0,"end":0.5,"confidence":0.92}]},{"text":"mondo","start":0.5,"end":1.2,"words":[{"text":"mondo","start":0.5,"end":1.2,"confidence":0.89}]}]}'
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "it",
            &WhisperOptions::default(),
            Some(1.2),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.text, "ciao. mondo");
    assert_eq!(transcript.segments.len(), 2);
    assert_eq!(transcript.segments[0].text, "ciao.");
    assert_eq!(transcript.segments[0].start_seconds, Some(0.0));
    assert_eq!(transcript.segments[0].end_seconds, Some(0.5));
    assert_eq!(transcript.segments[0].words[0].confidence, Some(0.92));
    assert_eq!(transcript.segments[1].text, "mondo");
}

#[tokio::test]
async fn transcribe_rejects_missing_model_before_starting_cli() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");
    write_executable_script(
        &script_path,
        "#!/bin/sh\necho cli should not run >&2\nexit 99\n",
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "it",
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("missing model should fail");

    assert!(error.to_string().contains("Parakeet model file not found"));
    assert!(!error.to_string().contains("cli should not run"));
}

#[tokio::test]
async fn transcribe_rejects_missing_realtime_preview_model_before_starting_cli() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");
    write_executable_script(
        &script_path,
        "#!/bin/sh\necho cli should not run >&2\nexit 99\n",
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "it",
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("missing realtime preview model should fail");

    assert!(error
        .to_string()
        .contains("Parakeet progressive preview requires"));
    assert!(!error.to_string().contains("cli should not run"));
}

#[tokio::test]
async fn transcribe_rejects_successful_cli_without_json() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");
    write_executable_script(
        &script_path,
        "#!/bin/sh\ncase \"$*\" in *--stream*) echo '[00:00:00.000 --> 00:00:01.000] preview'; exit 0;; esac\necho 'loaded model but no json'\nexit 0\n",
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "it",
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("missing json should fail");

    assert!(error.to_string().contains("produced no JSON output"));
}

#[tokio::test]
async fn transcribe_returns_stderr_on_failure() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake realtime model",
    )
    .expect("failed to create preview model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo "fatal: unsupported model" 1>&2
exit 2
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
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
            assert!(message.contains("fatal: unsupported model"));
        }
        other => panic!("unexpected error variant: {other}"),
    }
}

#[tokio::test]
async fn transcribe_rejects_translate_to_english() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    std::fs::write(&input_wav, b"RIFF....WAVE").expect("failed to create input wav");
    write_executable_script(&script_path, "#!/bin/sh\nexit 0\n");

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let mut options = WhisperOptions::default();
    options.translate_to_english = true;

    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            "en",
            &options,
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("translation should be rejected");

    assert!(error.to_string().contains("translate-to-English"));
}

#[tokio::test]
#[ignore = "requires real parakeet-cli, GGUF model, and spoken audio env vars"]
async fn parakeet_cpp_real_smoke() {
    let cli_path = required_env("SBOBINO_PARAKEET_CLI");
    let models_dir = required_env("SBOBINO_PARAKEET_MODELS_DIR");
    let audio_path = required_env("SBOBINO_PARAKEET_AUDIO");
    let model_filename =
        optional_env("SBOBINO_PARAKEET_MODEL").unwrap_or_else(|| DEFAULT_REAL_SMOKE_MODEL.into());

    assert!(
        Path::new(&cli_path).is_file(),
        "SBOBINO_PARAKEET_CLI must point to an existing file"
    );
    assert!(
        Path::new(&models_dir).is_dir(),
        "SBOBINO_PARAKEET_MODELS_DIR must point to an existing directory"
    );
    assert!(
        Path::new(&models_dir).join(&model_filename).is_file(),
        "Parakeet model file must exist in SBOBINO_PARAKEET_MODELS_DIR"
    );
    assert!(
        Path::new(&audio_path).is_file(),
        "SBOBINO_PARAKEET_AUDIO must point to an existing audio file"
    );

    let engine = ParakeetCppEngine::new(cli_path, models_dir);
    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_ref = emitted.clone();
    let transcript = engine
        .transcribe(
            Path::new(&audio_path),
            &model_filename,
            "auto",
            &WhisperOptions::default(),
            None,
            Arc::new(move |line: String| {
                let mut emitted = emitted_ref.lock().expect("emit lock poisoned");
                if emitted.len() < 5 {
                    eprintln!("parakeet_partial={line}");
                }
                emitted.push(line);
            }),
            Arc::new(|seconds: f32| {
                eprintln!("parakeet_progress_seconds={seconds:.3}");
            }),
        )
        .await
        .expect("real Parakeet transcription should succeed");

    eprintln!("parakeet_text={}", transcript.text);
    eprintln!("parakeet_segments={}", transcript.segments.len());
    assert_valid_real_parakeet_output(&transcript);
    let min_segments = optional_env("SBOBINO_PARAKEET_MIN_SEGMENTS")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    assert!(
        transcript.segments.len() >= min_segments,
        "expected at least {min_segments} Parakeet segments, got {}",
        transcript.segments.len()
    );
    let emitted = emitted.lock().expect("emit lock poisoned");
    let preview_delta_count = emitted
        .iter()
        .filter(|line| line.starts_with("\u{001F}REPLACE:"))
        .count();
    let min_preview_deltas = optional_env("SBOBINO_PARAKEET_MIN_PREVIEW_DELTAS")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1);
    assert!(
        preview_delta_count >= min_preview_deltas,
        "expected at least {min_preview_deltas} progressive Parakeet preview deltas, got {preview_delta_count}"
    );
    assert_eq!(
        emitted.last().map(String::as_str),
        Some(transcript.text.as_str()),
        "final Parakeet delta should match the final transcript"
    );

    if let Some(pattern) = optional_env("SBOBINO_PARAKEET_EXPECTED_REGEX") {
        let regex = Regex::new(&pattern).expect("SBOBINO_PARAKEET_EXPECTED_REGEX is invalid");
        assert!(
            regex.is_match(&transcript.text),
            "Parakeet transcript did not match expected regex '{pattern}': {}",
            transcript.text
        );
    }
}
