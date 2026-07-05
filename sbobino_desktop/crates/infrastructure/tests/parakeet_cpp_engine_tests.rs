#![cfg(unix)]

use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

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

fn write_test_wav(path: &Path, seconds: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create wav");
    for index in 0..(seconds * 16_000) {
        let value = ((index as f32 * 0.02).sin() * i16::MAX as f32 * 0.2) as i16;
        writer.write_sample(value).expect("failed to write sample");
    }
    writer.finalize().expect("failed to finalize wav");
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
async fn transcribe_uses_realtime_json_chunks_for_progressive_preview() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create final model");
    write_test_wav(&input_wav, 18);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *tdt-0.6b-v3-q4_k.gguf*chunk-0000.wav*)
    echo '{"text":"first chunk<EOU>","words":[{"w":"first","start":0.1,"end":0.3},{"w":"chunk","start":0.3,"end":0.6}]}'
    exit 0
    ;;
  *tdt-0.6b-v3-q4_k.gguf*chunk-0001.wav*)
    echo '{"text":"second chunk<EOU>","words":[{"w":"second","start":0.1,"end":0.3},{"w":"chunk","start":0.3,"end":0.6}]}'
    exit 0
    ;;
  *tdt-0.6b-v3-q4_k.gguf*chunk-0002.wav*)
    echo '{"text":"third chunk<EOU>","words":[{"w":"third","start":0.1,"end":0.3},{"w":"chunk","start":0.3,"end":0.6}]}'
    exit 0
    ;;
esac
echo '{"text":"first chunk second chunk third chunk","words":[{"w":"first","start":0.0,"end":0.2},{"w":"chunk","start":0.2,"end":0.4},{"w":"second","start":8.0,"end":8.2},{"w":"chunk","start":8.2,"end":8.4},{"w":"third","start":16.0,"end":16.2},{"w":"chunk","start":16.2,"end":16.4}]}'
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
            "tdt-0.6b-v3-q4_k.gguf",
            "en",
            &WhisperOptions::default(),
            Some(18.0),
            Arc::new(move |line: String| {
                emitted_ref.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(transcript.text, "first chunk second chunk third chunk");
    let emitted = emitted.lock().expect("emit lock poisoned");
    let preview_deltas = emitted
        .iter()
        .filter(|line| line.starts_with("\u{001F}REPLACE:"))
        .collect::<Vec<_>>();
    assert!(
        preview_deltas.len() >= 2,
        "expected progressive preview deltas before final output, got {emitted:?}"
    );
    assert!(
        preview_deltas[0].contains("first chunk") && !preview_deltas[0].contains("second chunk"),
        "first preview should contain only the first chunk, got {:?}",
        preview_deltas[0]
    );
    assert!(
        preview_deltas
            .iter()
            .any(|line| line.contains("first chunk") && line.contains("second chunk")),
        "expected cumulative chunk preview, got {preview_deltas:?}"
    );
}

#[tokio::test]
async fn non_english_progressive_preview_uses_final_tdt_model_not_english_eou() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create final model");
    std::fs::write(
        models_dir.join("realtime_eou_120m-v1-f16.gguf"),
        b"fake english realtime model",
    )
    .expect("failed to create realtime model");
    write_test_wav(&input_wav, 18);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *realtime_eou_120m-v1-f16.gguf*)
    echo 'English-only realtime model must not be used for Italian preview' 1>&2
    exit 43
    ;;
  *tdt-0.6b-v3-q4_k.gguf*chunk-0000.wav*)
    echo '{"text":"primo blocco","words":[{"w":"primo","start":0.1,"end":0.3},{"w":"blocco","start":0.3,"end":0.6}]}'
    exit 0
    ;;
  *tdt-0.6b-v3-q4_k.gguf*chunk-0001.wav*)
    echo '{"text":"secondo blocco","words":[{"w":"secondo","start":0.1,"end":0.3},{"w":"blocco","start":0.3,"end":0.6}]}'
    exit 0
    ;;
esac
echo '{"text":"primo blocco secondo blocco","words":[{"w":"primo","start":0.0,"end":0.2},{"w":"blocco","start":0.2,"end":0.4},{"w":"secondo","start":8.0,"end":8.2},{"w":"blocco","start":8.2,"end":8.4}]}'
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
            "tdt-0.6b-v3-q4_k.gguf",
            "it",
            &WhisperOptions::default(),
            Some(18.0),
            Arc::new(move |line: String| {
                emitted_ref.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("Italian transcription should use the multilingual TDT model");

    assert_eq!(transcript.text, "primo blocco secondo blocco");
    let emitted = emitted.lock().expect("emit lock poisoned");
    assert!(
        emitted
            .iter()
            .any(|line| line.starts_with("\u{001F}REPLACE:") && line.contains("primo blocco")),
        "expected Italian TDT preview deltas, got {emitted:?}"
    );
}

#[tokio::test]
async fn long_file_merge_keeps_boundary_word_only_present_in_next_chunk() {
    // Regression: the chunked merge used to drop a word whose timestamp fell in
    // the overlap zone even when the previous chunk never transcribed it
    // (Parakeet routinely under-transcribes the tail of a clip). The result was
    // whole words — sometimes whole sentences near the ~5min chunk boundary —
    // silently disappearing from the transcript. The merge must never lose a
    // word that only one chunk produced, regardless of where its timestamp lands.
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 605);

    write_executable_script(
        &script_path,
        "#!/bin/sh\necho 'chunk CLI must not run when worker succeeds' >&2\nexit 45\n",
    );
    // Each chunk emits a word at its very start (the overlap region). For the
    // first chunk that is genuine audio near t=0. For every later chunk the word
    // lands inside the 2s overlap with the previous chunk. The previous chunk
    // did NOT transcribe this word (it under-transcribed its tail), so the only
    // copy is the one the next chunk produces — and the positional dedup drops
    // it because its timestamp is <= committed_until.
    write_executable_script(
        &worker_path,
        r#"#!/bin/sh
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  # Word sits exactly at the chunk's commit start. For every later chunk that is
  # inside the pre-context decoded by the previous chunk, but the previous chunk
  # did not emit it. It must survive because it is unique.
  local_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN { printf "%.3f", c - d }')
  local_end=$(awk -v s="$local_start" 'BEGIN { printf "%.3f", s + 0.4 }')
  echo "{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{\"text\":\"parola$idx\",\"words\":[{\"w\":\"parola$idx\",\"start\":$local_start,\"end\":$local_end}]}}"
done < "$manifest"
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
            "tdt-0.6b-v3-q4_k.gguf",
            "it",
            &WhisperOptions::default(),
            Some(605.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("chunked transcription should merge worker output");

    assert!(
        transcript.text.contains("parola1"),
        "the boundary word only present in the second chunk must survive the merge, got: {:?}",
        transcript.text
    );
}

#[tokio::test]
async fn long_file_transcription_uses_worker_chunks_and_progressive_deltas() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 605);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *audio.wav*)
    echo 'full input must not be used for long Parakeet files' 1>&2
    exit 44
    ;;
  *chunk-*)
    echo '{"text":"preview chunk","words":[{"w":"preview","start":0.1,"end":0.3},{"w":"chunk","start":0.3,"end":0.6}]}'
    exit 0
    ;;
esac
echo 'unexpected parakeet-cli invocation' 1>&2
exit 45
"#,
    );
    write_executable_script(
        &worker_path,
        r#"#!/bin/sh
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN { printf "%.3f", c - d + 3.1 }')
  word_mid=$(awk -v s="$word_start" 'BEGIN { printf "%.3f", s + 0.2 }')
  word_last=$(awk -v s="$word_start" 'BEGIN { printf "%.3f", s + 0.4 }')
  word_end=$(awk -v s="$word_start" 'BEGIN { printf "%.3f", s + 0.6 }')
  echo "{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{\"text\":\"worker chunk $idx\",\"words\":[{\"w\":\"worker\",\"start\":$word_start,\"end\":$word_mid},{\"w\":\"chunk\",\"start\":$word_mid,\"end\":$word_last},{\"w\":\"$idx\",\"start\":$word_last,\"end\":$word_end}]}}"
done < "$manifest"
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_ref = emitted.clone();
    let progress: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_ref = progress.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            "it",
            &WhisperOptions::default(),
            Some(605.0),
            Arc::new(move |line: String| {
                emitted_ref.lock().expect("emit lock poisoned").push(line);
            }),
            Arc::new(move |seconds: f32| {
                progress_ref
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("long Parakeet transcription should use chunked worker path");

    assert!(transcript.text.contains("worker chunk 0"));
    assert!(transcript.text.contains("worker chunk 1"));
    assert!(transcript.segments.len() >= 2);
    assert!(transcript.segments[1].start_seconds.unwrap_or_default() > 100.0);
    let emitted = emitted.lock().expect("emit lock poisoned");
    assert!(
        emitted
            .iter()
            .any(|line| line.starts_with("\u{001F}REPLACE:") && line.contains("worker chunk 1")),
        "expected cumulative long-file deltas, got {emitted:?}"
    );
    let progress = progress.lock().expect("progress lock poisoned");
    assert!(progress.iter().any(|seconds| *seconds > 300.0));
}

#[tokio::test]
async fn long_file_manifest_uses_contiguous_commit_windows_with_decode_context() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let manifest_copy = temp.path().join("manifest-copy.tsv");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 1000);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *chunk-*)
    echo '{"text":"preview","words":[{"w":"preview","start":0.1,"end":0.3}]}'
    exit 0
    ;;
esac
echo 'full input must not be used for long Parakeet files' 1>&2
exit 44
"#,
    );
    write_executable_script(
        &worker_path,
        &format!(
            r#"#!/bin/sh
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
cp "$manifest" "{manifest_copy}"
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN {{ printf "%.3f", c - d + 1.0 }}')
  word_end=$(awk -v s="$word_start" 'BEGIN {{ printf "%.3f", s + 0.4 }}')
  echo "{{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{{\"text\":\"chunk$idx\",\"words\":[{{\"w\":\"chunk$idx\",\"start\":$word_start,\"end\":$word_end}}]}}}}"
done < "$manifest"
exit 0
"#,
            manifest_copy = manifest_copy.display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            "it",
            &WhisperOptions::default(),
            Some(1000.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("long Parakeet transcription should use worker manifest");

    assert!(transcript.text.contains("chunk0"));
    assert!(
        manifest_copy.exists(),
        "fake worker should copy the generated manifest"
    );

    let manifest = std::fs::read_to_string(&manifest_copy).expect("failed to read manifest copy");
    let mut previous_commit_end = 0.0_f32;
    let rows = manifest
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    assert!(
        rows.len() >= 3,
        "expected multiple long-audio windows: {rows:?}"
    );

    for (index, row) in rows.iter().enumerate() {
        let fields = row.split('\t').collect::<Vec<_>>();
        assert_eq!(
            fields.len(),
            6,
            "manifest row should have 6 TSV fields: {row}"
        );
        let decode_start = fields[1].parse::<f32>().expect("decode_start");
        let decode_end = fields[2].parse::<f32>().expect("decode_end");
        let commit_start = fields[3].parse::<f32>().expect("commit_start");
        let commit_end = fields[4].parse::<f32>().expect("commit_end");

        if index == 0 {
            assert!(
                commit_end - commit_start <= 145.0,
                "first long-file chunk should be fast-start sized: {row}"
            );
        }
        assert!(
            (commit_start - previous_commit_end).abs() <= 0.02,
            "commit windows must be contiguous: previous={previous_commit_end}, row={row}"
        );
        assert!(decode_start <= commit_start + 0.02, "row={row}");
        assert!(decode_end >= commit_end - 0.02, "row={row}");
        if index > 0 {
            assert!(
                decode_start < commit_start - 1.0,
                "non-first windows need pre-context: {row}"
            );
        }
        if index + 1 < rows.len() {
            assert!(
                decode_end > commit_end + 1.0,
                "non-final windows need post-context: {row}"
            );
        }
        previous_commit_end = commit_end;
    }
    assert!(
        (previous_commit_end - 1000.0).abs() <= 0.02,
        "final commit should reach the full audio duration, got {previous_commit_end}"
    );
}

#[tokio::test]
async fn long_file_worker_streams_first_chunk_before_process_exit() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let first_chunk_sleep_marker = temp.path().join("first-chunk-sleep-complete");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 700);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo 'full input must not be used for long Parakeet files' 1>&2
exit 44
"#,
    );
    write_executable_script(
        &worker_path,
        &format!(
            r#"#!/bin/sh
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
count=0
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN {{ printf "%.3f", c - d + 1.0 }}')
  word_end=$(awk -v s="$word_start" 'BEGIN {{ printf "%.3f", s + 0.4 }}')
  echo "{{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{{\"text\":\"stream$idx\",\"words\":[{{\"w\":\"stream$idx\",\"start\":$word_start,\"end\":$word_end}}]}}}}"
  if [ "$count" -eq 0 ]; then
    sleep 1
    touch "{first_chunk_sleep_marker}"
  fi
  count=$((count + 1))
done < "$manifest"
exit 0
"#,
            first_chunk_sleep_marker = first_chunk_sleep_marker.display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let saw_first_chunk_before_worker_finished = Arc::new(AtomicBool::new(false));
    let saw_first_chunk_before_worker_finished_ref = saw_first_chunk_before_worker_finished.clone();
    let marker_ref = first_chunk_sleep_marker.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            "it",
            &WhisperOptions::default(),
            Some(700.0),
            Arc::new(move |line: String| {
                if line.contains("stream0") && !marker_ref.exists() {
                    saw_first_chunk_before_worker_finished_ref.store(true, Ordering::SeqCst);
                }
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("long Parakeet transcription should stream worker chunks");

    assert!(transcript.text.contains("stream0"));
    assert!(transcript.text.contains("stream1"));
    assert!(
        saw_first_chunk_before_worker_finished.load(Ordering::SeqCst),
        "first worker chunk should be emitted while the worker is still running"
    );
}

#[tokio::test]
async fn long_file_worker_oom_retries_with_smaller_coverage_windows() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let attempt_file = temp.path().join("attempt-count");
    let first_manifest = temp.path().join("manifest-attempt-1.tsv");
    let second_manifest = temp.path().join("manifest-attempt-2.tsv");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 1000);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *chunk-*)
    echo '{"text":"preview","words":[{"w":"preview","start":0.1,"end":0.3}]}'
    exit 0
    ;;
esac
echo 'chunk CLI must not run after worker OOM retry' 1>&2
exit 45
"#,
    );
    write_executable_script(
        &worker_path,
        &format!(
            r#"#!/bin/sh
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
attempt=0
if [ -f "{attempt_file}" ]; then
  attempt=$(cat "{attempt_file}")
fi
attempt=$((attempt + 1))
echo "$attempt" > "{attempt_file}"
cp "$manifest" "{temp}/manifest-attempt-$attempt.tsv"
if [ "$attempt" -eq 1 ]; then
  echo 'ggml_backend_graph_compute failed: kIOGPUCommandBufferCallbackErrorOutOfMemory' 1>&2
  exit 86
fi
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN {{ printf "%.3f", c - d + 1.0 }}')
  word_end=$(awk -v s="$word_start" 'BEGIN {{ printf "%.3f", s + 0.4 }}')
  echo "{{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{{\"text\":\"retry$idx\",\"words\":[{{\"w\":\"retry$idx\",\"start\":$word_start,\"end\":$word_end}}]}}}}"
done < "$manifest"
exit 0
"#,
            attempt_file = attempt_file.display(),
            temp = temp.path().display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            "it",
            &WhisperOptions::default(),
            Some(1000.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("OOM should retry with smaller worker windows");

    assert!(transcript.text.contains("retry0"));
    assert!(first_manifest.exists(), "first worker attempt should run");
    assert!(second_manifest.exists(), "second worker attempt should run");

    let first_manifest =
        std::fs::read_to_string(first_manifest).expect("failed to read first manifest");
    let second_manifest =
        std::fs::read_to_string(second_manifest).expect("failed to read second manifest");
    let max_commit_seconds = |manifest: &str| {
        manifest
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|row| {
                let fields = row.split('\t').collect::<Vec<_>>();
                fields[4].parse::<f32>().unwrap() - fields[3].parse::<f32>().unwrap()
            })
            .fold(0.0_f32, f32::max)
    };
    let first_commit_seconds = max_commit_seconds(&first_manifest);
    let second_commit_seconds = max_commit_seconds(&second_manifest);
    assert!(
        second_commit_seconds < first_commit_seconds,
        "retry should regenerate a smaller commit plan: first max={first_commit_seconds}, second max={second_commit_seconds}"
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
async fn transcribe_does_not_require_separate_realtime_preview_model() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-f16.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 1);
    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo '{"text":"ok","segments":[{"text":"ok","start":0.0,"end":0.5}],"words":[{"word":"ok","start":0.0,"end":0.5}]}'
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
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("selected final model should also be usable for preview");

    assert_eq!(transcript.text, "ok");
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
