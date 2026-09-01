#![cfg(unix)]

use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use regex::Regex;
use tempfile::tempdir;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{
    LanguageCode, TranscriptionComputeDevice, TranscriptionLanguagePolicy, WhisperOptions,
};
use sbobino_infrastructure::adapters::parakeet_cpp::ParakeetCppEngine;

const DEFAULT_REAL_SMOKE_MODEL: &str = "tdt-0.6b-v3-q4_k.gguf";

fn transcription_policy(code: &str) -> TranscriptionLanguagePolicy {
    let preferred_language = LanguageCode::from_code(code);
    TranscriptionLanguagePolicy {
        adaptive_detection: preferred_language.is_auto(),
        preferred_language,
    }
}

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

fn write_constant_test_wav(path: &Path, seconds: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create wav");
    for _ in 0..(seconds * 16_000) {
        writer
            .write_sample(1_024_i16)
            .expect("failed to write constant sample");
    }
    writer.finalize().expect("failed to finalize constant wav");
}

fn write_silence_test_wav(path: &Path, seconds: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("failed to create wav");
    for _ in 0..(seconds * 16_000) {
        writer.write_sample(0_i16).expect("failed to write silence");
    }
    writer.finalize().expect("failed to finalize silence wav");
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

#[cfg(target_os = "macos")]
#[test]
fn batch_worker_rejects_malformed_or_oversize_manifest_before_model_load() {
    use std::process::Command;

    let temp = tempdir().expect("failed to create temp dir");
    let fake_include_dir = temp.path().join("fake-include");
    let fake_header = fake_include_dir.join("parakeet_capi.h");
    let fake_capi = temp.path().join("fake_parakeet_capi.cpp");
    let worker_binary = temp.path().join("parakeet-batch-json-test");
    std::fs::create_dir_all(&fake_include_dir).expect("failed to create fake include dir");
    std::fs::write(
        &fake_header,
        r#"#pragma once
struct parakeet_ctx {};
extern "C" {
parakeet_ctx* parakeet_capi_load(const char*);
void parakeet_capi_free(parakeet_ctx*);
void parakeet_capi_set_num_threads(int);
char* parakeet_capi_transcribe_path_json(parakeet_ctx*, const char*, int);
char* parakeet_capi_transcribe_pcm_batch_json_lang(
    parakeet_ctx*, float*, int*, int, int, int, const char*);
const char* parakeet_capi_last_error(parakeet_ctx*);
void parakeet_capi_free_string(char*);
}
"#,
    )
    .expect("failed to write fake parakeet header");
    std::fs::write(
        &fake_capi,
        r#"#include "parakeet_capi.h"
#include <cstdio>
extern "C" parakeet_ctx* parakeet_capi_load(const char*) {
    std::fputs("MODEL_LOAD_SHOULD_NOT_HAPPEN\\n", stderr);
    return nullptr;
}
extern "C" void parakeet_capi_free(parakeet_ctx*) {}
extern "C" void parakeet_capi_set_num_threads(int) {}
extern "C" char* parakeet_capi_transcribe_path_json(parakeet_ctx*, const char*, int) {
    return nullptr;
}
extern "C" char* parakeet_capi_transcribe_pcm_batch_json_lang(
    parakeet_ctx*, float*, int*, int, int, int, const char*) { return nullptr; }
extern "C" const char* parakeet_capi_last_error(parakeet_ctx*) { return "fake"; }
extern "C" void parakeet_capi_free_string(char*) {}
"#,
    )
    .expect("failed to write fake parakeet C API");

    let worker_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("infrastructure crate should be under desktop workspace")
        .join("scripts/parakeet_batch_json.cpp");
    let compile = Command::new("clang++")
        .arg("-std=c++17")
        .arg("-I")
        .arg(&fake_include_dir)
        .arg(&worker_source)
        .arg(&fake_capi)
        .arg("-o")
        .arg(&worker_binary)
        .output()
        .expect("clang++ must be available for the macOS worker build");
    assert!(
        compile.status.success(),
        "fake-CAPI worker build failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let cases = [
        ("nan", "0\\tNaN\\t30\\t0\\t30\\t/tmp/chunk.wav\\n", false),
        (
            "negative_decode_start",
            "0\\t-0.001\\t30\\t0\\t30\\t/tmp/chunk.wav\\n",
            false,
        ),
        (
            "oversize",
            "0\\t0\\t45.001\\t0\\t30\\t/tmp/chunk.wav\\n",
            false,
        ),
        (
            "duplicate_index",
            "0\\t0\\t30\\t0\\t30\\t/tmp/chunk0.wav\\n0\\t25\\t45\\t30\\t45\\t/tmp/chunk1.wav\\n",
            false,
        ),
        (
            "resumed_nonzero",
            "0\\t25\\t55\\t30\\t50\\t/tmp/chunk-resumed.wav\\n",
            true,
        ),
    ];
    for (name, contents, should_reach_model_load) in cases {
        let manifest = temp.path().join(format!("{name}.tsv"));
        std::fs::write(
            &manifest,
            contents.replace("\\t", "\t").replace("\\n", "\n"),
        )
        .expect("failed to write invalid manifest");
        let output = Command::new(&worker_binary)
            .args(["--model", "fake-model.gguf", "--manifest"])
            .arg(&manifest)
            .output()
            .expect("failed to launch fake-CAPI worker");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if should_reach_model_load {
            assert!(
                !output.status.success() && stderr.contains("MODEL_LOAD_SHOULD_NOT_HAPPEN"),
                "{name} resumed manifest should pass validation before model load, got status {:?}, stderr: {stderr}",
                output.status
            );
            assert!(
                !stderr.contains("rejected manifest"),
                "{name} resumed manifest must not be rejected before model load: {stderr}"
            );
        } else {
            assert!(
                !output.status.success() && stderr.contains("rejected manifest"),
                "{name} manifest should fail validation, got status {:?}, stderr: {stderr}",
                output.status
            );
            assert!(
                !stderr.contains("MODEL_LOAD_SHOULD_NOT_HAPPEN"),
                "{name} manifest must be rejected before model load: {stderr}"
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn batch_worker_routes_auto_to_path_json_and_explicit_language_to_batch_api() {
    use std::process::Command;

    let temp = tempdir().expect("failed to create temp dir");
    let fake_include_dir = temp.path().join("fake-include");
    let fake_header = fake_include_dir.join("parakeet_capi.h");
    let fake_capi = temp.path().join("fake_parakeet_capi.cpp");
    let worker_binary = temp.path().join("parakeet-batch-json-routing-test");
    let chunk_path = temp.path().join("chunk.wav");
    let manifest_path = temp.path().join("chunks.tsv");
    std::fs::create_dir_all(&fake_include_dir).expect("failed to create fake include dir");
    write_silence_test_wav(&chunk_path, 1);
    std::fs::write(
        &manifest_path,
        format!("0\t0\t1\t0\t1\t{}\n", chunk_path.display()),
    )
    .expect("failed to write routing manifest");
    std::fs::write(
        &fake_header,
        r#"#pragma once
struct parakeet_ctx {};
extern "C" {
parakeet_ctx* parakeet_capi_load(const char*);
void parakeet_capi_free(parakeet_ctx*);
void parakeet_capi_set_num_threads(int);
char* parakeet_capi_transcribe_path_json(parakeet_ctx*, const char*, int);
char* parakeet_capi_transcribe_pcm_batch_json_lang(
    parakeet_ctx*, float*, int*, int, int, int, const char*);
const char* parakeet_capi_last_error(parakeet_ctx*);
void parakeet_capi_free_string(char*);
}
"#,
    )
    .expect("failed to write fake parakeet header");
    std::fs::write(
        &fake_capi,
        r#"#include "parakeet_capi.h"
#include <cstdio>
#include <cstdlib>
#include <cstring>

static char* copy_json(const char* value) {
    const std::size_t size = std::strlen(value) + 1;
    char* result = static_cast<char*>(std::malloc(size));
    if (result != nullptr) {
        std::memcpy(result, value, size);
    }
    return result;
}

extern "C" parakeet_ctx* parakeet_capi_load(const char*) {
    static parakeet_ctx context;
    return &context;
}
extern "C" void parakeet_capi_free(parakeet_ctx*) {}
extern "C" void parakeet_capi_set_num_threads(int threads) {
    std::fprintf(stderr, "THREAD_CAP=%d\n", threads);
}
extern "C" char* parakeet_capi_transcribe_path_json(
    parakeet_ctx*, const char*, int decoder) {
    std::fprintf(stderr, "PATH_JSON_DECODER=%d\n", decoder);
    return copy_json("{\"text\":\"path\",\"words\":[],\"tokens\":[]}");
}
extern "C" char* parakeet_capi_transcribe_pcm_batch_json_lang(
    parakeet_ctx*, float*, int*, int, int, int decoder, const char*) {
    std::fprintf(stderr, "LANG_BATCH_DECODER=%d\n", decoder);
    return copy_json("[{\"text\":\"lang\",\"words\":[],\"tokens\":[]}]");
}
extern "C" const char* parakeet_capi_last_error(parakeet_ctx*) { return "fake"; }
extern "C" void parakeet_capi_free_string(char* value) { std::free(value); }
"#,
    )
    .expect("failed to write fake parakeet C API");

    let worker_source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("infrastructure crate should be under desktop workspace")
        .join("scripts/parakeet_batch_json.cpp");
    let compile = Command::new("clang++")
        .arg("-std=c++17")
        .arg("-I")
        .arg(&fake_include_dir)
        .arg(&worker_source)
        .arg(&fake_capi)
        .arg("-o")
        .arg(&worker_binary)
        .output()
        .expect("clang++ must be available for the macOS worker build");
    assert!(
        compile.status.success(),
        "fake-CAPI routing worker build failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let auto = Command::new(&worker_binary)
        .args(["--model", "fake-model.gguf", "--manifest"])
        .arg(&manifest_path)
        .args(["--threads", "3"])
        .output()
        .expect("failed to launch auto worker");
    let auto_stdout = String::from_utf8_lossy(&auto.stdout);
    let auto_stderr = String::from_utf8_lossy(&auto.stderr);
    assert!(auto.status.success(), "auto worker failed: {auto_stderr}");
    assert!(
        auto_stdout.contains("\"result\":{\"text\":\"path\""),
        "auto worker must preserve path JSON result, got stdout: {auto_stdout}"
    );
    assert!(auto_stderr.contains("THREAD_CAP=3"));
    assert!(auto_stderr.contains("PATH_JSON_DECODER=0"));
    assert!(!auto_stderr.contains("LANG_BATCH"));

    let explicit = Command::new(&worker_binary)
        .args(["--model", "fake-model.gguf", "--manifest"])
        .arg(&manifest_path)
        .args(["--lang", "it", "--threads", "2"])
        .output()
        .expect("failed to launch explicit-language worker");
    let explicit_stdout = String::from_utf8_lossy(&explicit.stdout);
    let explicit_stderr = String::from_utf8_lossy(&explicit.stderr);
    assert!(
        explicit.status.success(),
        "explicit-language worker failed: {explicit_stderr}"
    );
    assert!(
        explicit_stdout.contains("\"result\":{\"text\":\"lang\""),
        "explicit-language worker must preserve batch result, got stdout: {explicit_stdout}"
    );
    assert!(explicit_stderr.contains("THREAD_CAP=2"));
    assert!(explicit_stderr.contains("LANG_BATCH_DECODER=0"));
    assert!(!explicit_stderr.contains("PATH_JSON"));
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
            &transcription_policy("it"),
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
            &transcription_policy("it"),
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
async fn short_file_transcription_does_not_run_duplicate_preview_inference() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create final model");
    write_test_wav(&input_wav, 12);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *tdt-0.6b-v3-q4_k.gguf*chunk-0000.wav*)
    echo 'duplicate preview inference must not run' 1>&2
    exit 91
    ;;
  *tdt-0.6b-v3-q4_k.gguf*chunk-0001.wav*)
    echo 'duplicate preview inference must not run' 1>&2
    exit 91
    ;;
  *tdt-0.6b-v3-q4_k.gguf*chunk-0002.wav*)
    echo 'duplicate preview inference must not run' 1>&2
    exit 91
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
            &transcription_policy("en"),
            &WhisperOptions::default(),
            Some(12.0),
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
    write_test_wav(&input_wav, 12);

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
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(12.0),
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
while IFS="$(printf '\t')" read -r idx decode_start decode_end commit_start commit_end path; do
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
            &transcription_policy("it"),
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
            &transcription_policy("it"),
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
    assert!(
        transcript
            .segments
            .iter()
            .any(|segment| segment.start_seconds.unwrap_or_default() > 25.0),
        "later 30 s commit windows must retain decoded absolute timing: {:?}",
        transcript
            .segments
            .iter()
            .map(|segment| segment.start_seconds)
            .collect::<Vec<_>>()
    );
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
async fn long_file_worker_keeps_valid_empty_final_chunk_without_cli_fallback() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let cli_invoked = temp.path().join("cli-fallback-invoked");
    let worker_invocations = temp.path().join("worker-invocations");
    let manifest_row_count = temp.path().join("manifest-row-count");
    let output_rows = temp.path().join("worker-output-rows");
    let empty_result_row = temp.path().join("empty-result-row");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_silence_test_wav(&input_wav, 605);

    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
touch "{cli_invoked}"
echo 'CLI fallback must not run for a valid silent worker row' >&2
exit 45
"#,
            cli_invoked = cli_invoked.display()
        ),
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
[ -s "$manifest" ] || exit 46
echo "$$" >> "{worker_invocations}"
wc -l < "$manifest" | tr -d ' ' > "{manifest_row_count}"
last_index=$(tail -n 1 "$manifest" | cut -f1)
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  echo "$idx" >> "{output_rows}"
  if [ "$idx" = "$last_index" ]; then
    echo "$idx" > "{empty_result_row}"
    echo "{{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{{}}}}"
  else
    word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN {{ printf "%.3f", c - d + 1.0 }}')
    word_end=$(awk -v s="$word_start" 'BEGIN {{ printf "%.3f", s + 0.4 }}')
    echo "{{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{{\"text\":\"spoken-$idx\",\"words\":[{{\"w\":\"spoken-$idx\",\"start\":$word_start,\"end\":$word_end}}]}}}}"
  fi
done < "$manifest"
exit 0
"#,
            worker_invocations = worker_invocations.display(),
            manifest_row_count = manifest_row_count.display(),
            output_rows = output_rows.display(),
            empty_result_row = empty_result_row.display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let progress: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let progress_ref = progress.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(605.0),
            Arc::new(|_line: String| {}),
            Arc::new(move |seconds: f32| {
                progress_ref
                    .lock()
                    .expect("progress lock poisoned")
                    .push(seconds);
            }),
        )
        .await
        .expect("a valid silent final worker row must not trigger CLI fallback");

    assert!(
        !cli_invoked.exists(),
        "the direct CLI must not run after a valid empty worker result"
    );
    assert_eq!(
        std::fs::read_to_string(&worker_invocations)
            .expect("worker should record its invocation")
            .lines()
            .count(),
        1,
        "the long file must use one batch worker process"
    );
    let manifest_rows = std::fs::read_to_string(&manifest_row_count)
        .expect("worker should record manifest row count")
        .trim()
        .parse::<usize>()
        .expect("manifest row count should be numeric");
    let output_indices = std::fs::read_to_string(&output_rows)
        .expect("worker should record every output row")
        .lines()
        .map(|line| line.parse::<usize>().expect("worker output index"))
        .collect::<Vec<_>>();
    assert!(manifest_rows > 1, "fixture must contain a final chunk");
    assert_eq!(
        output_indices.len(),
        manifest_rows,
        "no worker row may be dropped"
    );
    assert_eq!(
        output_indices,
        (0..manifest_rows).collect::<Vec<_>>(),
        "worker output rows must stay complete and ordered"
    );
    assert_eq!(
        std::fs::read_to_string(&empty_result_row)
            .expect("worker should mark its silent final row")
            .trim()
            .parse::<usize>()
            .expect("silent row index"),
        manifest_rows - 1,
        "only the final valid worker result is intentionally empty"
    );
    assert!(
        transcript.text.contains("spoken-0"),
        "speech before the silent final chunk must survive: {}",
        transcript.text
    );
    let progress = progress.lock().expect("progress lock poisoned");
    assert!(
        (progress.last().copied().unwrap_or_default() - 605.0).abs() <= 0.02,
        "coverage/progress must reach the final commit end: {progress:?}"
    );
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
            &transcription_policy("it"),
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

        assert!(
            commit_end - commit_start <= 30.02,
            "every commit window must stay within the 30 s initial/target budget: {row}"
        );
        assert!(
            decode_end - decode_start <= 45.02,
            "every serialized decode, including context/tail padding, must stay under 45 s: {row}"
        );
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
    let final_fields = rows
        .last()
        .expect("manifest has final row")
        .split('\t')
        .collect::<Vec<_>>();
    let final_decode_end = final_fields[2].parse::<f32>().expect("final decode_end");
    assert!(
        (final_decode_end - 1002.0).abs() <= 0.02,
        "the final chunk should retain only the bounded 2 s tail pad, got {final_decode_end}"
    );
}

#[tokio::test]
async fn medium_file_worker_streams_progress_before_process_exit() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let first_chunk_sleep_marker = temp.path().join("first-chunk-sleep-complete");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 18);

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
    let saw_progress_before_worker_finished = Arc::new(AtomicBool::new(false));
    let saw_progress_before_worker_finished_ref = saw_progress_before_worker_finished.clone();
    let marker_ref = first_chunk_sleep_marker.clone();
    let progress_marker_ref = first_chunk_sleep_marker.clone();

    let transcript = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(18.0),
            Arc::new(move |line: String| {
                if line.contains("stream0") && !marker_ref.exists() {
                    saw_first_chunk_before_worker_finished_ref.store(true, Ordering::SeqCst);
                }
            }),
            Arc::new(move |seconds: f32| {
                if seconds > 0.0 && !progress_marker_ref.exists() {
                    saw_progress_before_worker_finished_ref.store(true, Ordering::SeqCst);
                }
            }),
        )
        .await
        .expect("long Parakeet transcription should stream worker chunks");

    assert!(transcript.text.contains("stream0"));
    assert!(transcript.text.contains("stream1"));
    assert!(
        saw_first_chunk_before_worker_finished.load(Ordering::SeqCst),
        "first worker chunk should be emitted while the worker is still running"
    );
    assert!(
        saw_progress_before_worker_finished.load(Ordering::SeqCst),
        "a 15-45 second file must report committed progress before the worker exits"
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
    let manifests = (1..=4)
        .map(|attempt| temp.path().join(format!("manifest-attempt-{attempt}.tsv")))
        .collect::<Vec<_>>();

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    // Equal energy makes the silence tie-break choose the requested target
    // edge, so each retry plan proves its actual 30/20/15/10 s commit cap.
    write_constant_test_wav(&input_wav, 1000);

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
if [ "$attempt" -le 3 ]; then
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
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(1000.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("OOM should retry with smaller worker windows");

    assert!(transcript.text.contains("retry0"));
    assert_eq!(
        std::fs::read_to_string(&attempt_file)
            .expect("worker should persist attempt count")
            .trim(),
        "4",
        "the worker must run the initial plan plus all three smaller retry windows"
    );
    for manifest in &manifests {
        assert!(
            manifest.exists(),
            "worker attempt should write {manifest:?}"
        );
    }

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
    let retry_windows = manifests
        .iter()
        .map(|path| {
            max_commit_seconds(
                &std::fs::read_to_string(path)
                    .unwrap_or_else(|error| panic!("failed to read {path:?}: {error}")),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(retry_windows.len(), 4);
    for (actual, expected) in retry_windows.into_iter().zip([30.0, 20.0, 15.0, 10.0]) {
        assert!(
            (actual - expected).abs() <= 0.02,
            "retry planner should generate the {expected}s commit window, got {actual}s"
        );
    }
}

#[tokio::test]
async fn long_file_voiced_empty_retry_uses_commit_only_audio_without_replaying_prefix() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let worker_log = temp.path().join("worker-log.tsv");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join(DEFAULT_REAL_SMOKE_MODEL), b"fake model")
        .expect("failed to create model");
    // Constant non-zero PCM makes the commit-only voiced guard deterministic.
    write_constant_test_wav(&input_wav, 65);

    write_executable_script(
        &script_path,
        "#!/bin/sh\necho 'chunk CLI must not run for a worker retry' >&2\nexit 45\n",
    );
    write_executable_script(
        &worker_path,
        &format!(
            r#"#!/bin/sh
set -eu
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
while IFS="$(printf '\t')" read -r idx decode_start decode_end commit_start commit_end path; do
  [ -n "$idx" ] || continue
  bytes=$(wc -c < "$path" | tr -d '[:space:]')
  duration=$(awk -v bytes="$bytes" 'BEGIN {{ printf "%.3f", (bytes - 44) / 32000.0 }}')
  sample=$(dd if="$path" bs=1 skip=44 count=2 2>/dev/null | od -An -td2 | tr -d '[:space:]')
  printf '%s|%s|%s|%s|%s|%s|%s|%s\n' "$$" "$idx" "$decode_start" "$decode_end" "$commit_start" "$commit_end" "$duration" "$sample" >> "{worker_log}"

  # Simulate Nemotron's context-only empty response. The parent must retain
  # the confirmed first row and retry this exact commit window in isolation.
  if [ "$decode_start" = "25.000" ] && [ "$commit_start" = "30.000" ]; then
    printf '{{"index":%s,"decode_start":%s,"decode_end":%s,"commit_start":%s,"commit_end":%s,"result":{{"text":"","words":[]}}}}\n' \
      "$idx" "$decode_start" "$decode_end" "$commit_start" "$commit_end"
    exit 0
  fi

  label="prefix-$idx"
  if [ "$decode_start" = "30.000" ] && [ "$commit_start" = "30.000" ]; then
    label="retry-commit"
  elif [ "$commit_start" = "60.000" ]; then
    label="tail"
  fi
  word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN {{ printf "%.3f", c - d + 1.0 }}')
  word_end=$(awk -v s="$word_start" 'BEGIN {{ printf "%.3f", s + 0.4 }}')
  printf '{{"index":%s,"decode_start":%s,"decode_end":%s,"commit_start":%s,"commit_end":%s,"result":{{"text":"%s","words":[{{"w":"%s","start":%s,"end":%s}}]}}}}\n' \
    "$idx" "$decode_start" "$decode_end" "$commit_start" "$commit_end" "$label" "$label" "$word_start" "$word_end"
done < "$manifest"
exit 0
"#,
            worker_log = worker_log.display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let emitted = Arc::new(Mutex::new(Vec::<String>::new()));
    let emitted_ref = emitted.clone();
    let transcript = engine
        .transcribe(
            &input_wav,
            DEFAULT_REAL_SMOKE_MODEL,
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(65.0),
            Arc::new(move |delta: String| {
                emitted_ref.lock().expect("delta lock poisoned").push(delta);
            }),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("isolated voiced-empty retry should recover the full file");

    assert!(transcript.text.contains("prefix-0"));
    assert!(transcript.text.contains("retry-commit"));
    assert!(transcript.text.contains("tail"));
    assert_eq!(
        transcript.text.matches("prefix-0").count(),
        1,
        "confirmed prefix must not be replayed by an isolated retry"
    );

    let log = std::fs::read_to_string(&worker_log).expect("worker should inspect every WAV");
    let rows = log.lines().collect::<Vec<_>>();
    assert!(
        rows.iter()
            .any(|row| row.contains("|0|0.000|35.000|0.000|30.000|35.000|1024")),
        "initial confirmed row should contain its 35 s context WAV: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("|1|25.000|65.000|30.000|60.000|40.000|1024")),
        "first context attempt should contain the 40 s overlap WAV: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("|0|30.000|60.000|30.000|60.000|30.000|1024")),
        "isolated retry should expose only the 30 s commit WAV with non-zero PCM: {rows:?}"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.contains("|0|0.000|35.000|0.000|30.000|35.000|1024"))
            .count(),
        1,
        "confirmed prefix row must be present in exactly one worker manifest"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("|0|55.000|67.000|60.000|65.000|12.000|1024")),
        "tail should resume at the confirmed commit edge without replaying prefix: {rows:?}"
    );
    assert!(
        emitted
            .lock()
            .expect("delta lock poisoned")
            .iter()
            .all(|delta| !delta.contains("SBOBINO_PARAKEET")),
        "retry diagnostics must stay technical and out of transcript deltas"
    );
}

#[tokio::test]
async fn long_file_auto_falls_back_to_cpu_worker_after_gpu_retries() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let attempt_file = temp.path().join("auto-attempt-count");
    let device_file = temp.path().join("auto-devices");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_constant_test_wav(&input_wav, 100);

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
echo 'long-file transcription must not invoke the CLI' 1>&2
exit 45
"#,
    );
    write_executable_script(
        &worker_path,
        r#"#!/bin/sh
set -eu
base=$(dirname "$0")
attempt_file="$base/auto-attempt-count"
device_file="$base/auto-devices"
attempt=0
if [ -f "$attempt_file" ]; then attempt=$(cat "$attempt_file"); fi
attempt=$((attempt + 1))
echo "$attempt" > "$attempt_file"
printf '%s\n' "${PARAKEET_DEVICE:-auto}" >> "$device_file"
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -s "$manifest" ] || exit 47
if [ "$attempt" -le 4 ]; then
  echo 'ggml_backend_graph_compute failed: kIOGPUCommandBufferCallbackErrorOutOfMemory' 1>&2
  exit 86
fi
while IFS='	' read -r idx decode_start decode_end commit_start commit_end path; do
  [ -n "$idx" ] || continue
  word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN { printf "%.3f", c - d + 0.1 }')
  word_end=$(awk -v s="$word_start" 'BEGIN { printf "%.3f", s + 0.2 }')
  printf '{"index":%s,"decode_start":%s,"decode_end":%s,"commit_start":%s,"commit_end":%s,"result":{"text":"cpu%s","words":[{"w":"cpu%s","start":%s,"end":%s}]}}\n' "$idx" "$decode_start" "$decode_end" "$commit_start" "$commit_end" "$idx" "$idx" "$word_start" "$word_end"
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
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(100.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("Auto should recover with the CPU worker after GPU retries");

    assert!(transcript.text.contains("cpu0"));
    assert_eq!(
        std::fs::read_to_string(&attempt_file)
            .expect("worker should persist attempt count")
            .trim(),
        "5",
        "Auto should run four GPU attempts followed by one CPU worker attempt"
    );
    let devices = std::fs::read_to_string(&device_file)
        .expect("worker should persist each execution device")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(devices.len(), 5);
    assert!(devices[..4].iter().all(|device| device != "cpu"));
    assert_eq!(devices.last().map(String::as_str), Some("cpu"));
}

#[tokio::test]
async fn long_file_explicit_cpu_rejects_terminal_partial_oom_prefix() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let attempt_file = temp.path().join("attempt-count");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    // Four exhausted retry windows can commit a valid 30 s prefix each. Keep
    // the physical audio longer than that prefix so the final result must
    // remain an error instead of persisting a truncated transcript.
    write_constant_test_wav(&input_wav, 150);

    write_executable_script(
        &script_path,
        "#!/bin/sh\necho 'long-file transcription must not invoke the CLI' 1>&2\nexit 45\n",
    );
    write_executable_script(
        &worker_path,
        &format!(
            r#"#!/bin/sh
set -eu
attempt=0
if [ -f "{attempt_file}" ]; then attempt=$(cat "{attempt_file}"); fi
attempt=$((attempt + 1))
echo "$attempt" > "{attempt_file}"
manifest=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ -s "$manifest" ] || exit 47
# Emit one valid prefix row, then fail as a backend OOM. The parent must retain
# the row for retry accounting but never report it as a completed transcript.
IFS='	' read -r idx decode_start decode_end commit_start commit_end path < "$manifest"
word_start=$(awk -v c="$commit_start" -v d="$decode_start" 'BEGIN {{ printf "%.3f", c - d + 0.1 }}')
word_end=$(awk -v s="$word_start" 'BEGIN {{ printf "%.3f", s + 0.2 }}')
printf '{{"index":%s,"decode_start":%s,"decode_end":%s,"commit_start":%s,"commit_end":%s,"result":{{"text":"prefix%s","words":[{{"w":"prefix%s","start":%s,"end":%s}}]}}}}\n' \
  "$idx" "$decode_start" "$decode_end" "$commit_start" "$commit_end" "$attempt" "$attempt" "$word_start" "$word_end"
echo 'ggml_backend_graph_compute failed: kIOGPUCommandBufferCallbackErrorOutOfMemory' 1>&2
exit 86
"#,
            attempt_file = attempt_file.display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    )
    .with_compute_device(TranscriptionComputeDevice::Cpu);
    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(150.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("explicit CPU must fail closed after terminal partial OOM retries");

    assert!(
        error
            .to_string()
            .contains("kIOGPUCommandBufferCallbackErrorOutOfMemory")
            || error.to_string().contains("out of memory"),
        "the terminal backend error must be retained, got: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&attempt_file)
            .expect("worker should persist retry attempts")
            .trim(),
        "4",
        "explicit CPU must exhaust the four bounded retry windows without an Auto fallback"
    );
}

#[tokio::test]
async fn long_file_parent_rejects_oversize_worker_row_without_cli_fallback() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    write_test_wav(&input_wav, 605);
    let cli_fallback_marker = temp.path().join("cli-fallback-ran");
    write_executable_script(
        &script_path,
        &format!(
            r#"#!/bin/sh
case "$*" in
  *chunk-*)
    touch "{cli_fallback_marker}"
    echo '{{"text":"safe fallback","words":[{{"w":"safe","start":0.1,"end":0.3}},{{"w":"fallback","start":0.3,"end":0.6}}]}}'
    exit 0
    ;;
esac
echo 'unexpected full-input fallback invocation' >&2
exit 45
"#,
            cli_fallback_marker = cli_fallback_marker.display()
        ),
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
[ -s "$manifest" ] || exit 47
echo '{"index":0,"decode_start":0.000,"decode_end":46.000,"commit_start":0.000,"commit_end":30.000,"result":{"text":"unsafe","words":[{"w":"unsafe","start":0.0,"end":0.1}]}}'
exit 0
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(605.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("worker protocol rejection must fail closed without a CLI fallback");

    assert!(
        !cli_fallback_marker.exists(),
        "the rejected worker result must not trigger a full-file or bounded CLI fallback"
    );
    assert!(
        error
            .to_string()
            .contains("invalid decode/commit bounds for chunk 0"),
        "oversize worker output must be rejected before transcript merge, got: {error}"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn long_file_rss_limit_terminates_worker_group_and_reports_peak() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");
    let child_pids = temp.path().join("worker-child-pids.txt");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("tdt-0.6b-v3-q4_k.gguf"), b"fake model")
        .expect("failed to create model");
    // The declared duration selects the long-file path; a tiny physical WAV
    // keeps this lifecycle test independent of real ASR and fast to run.
    write_test_wav(&input_wav, 1);
    write_executable_script(
        &script_path,
        "#!/bin/sh\necho 'CLI fallback must not run after RSS safety termination' >&2\nexit 45\n",
    );
    write_executable_script(
        &worker_path,
        &format!(
            r#"#!/bin/sh
# Do not trip on the shell's startup RSS: first make the helper observable,
# then let it allocate enough memory to cross the explicit test cap.
python3 -c 'import time; reserve = bytearray(32 * 1024 * 1024); time.sleep(30)' &
echo "$!" >> "{child_pids}"
while :; do
  sleep 30
done
"#,
            child_pids = child_pids.display()
        ),
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    )
    .with_worker_rss_limit_override_for_test(8 * 1024 * 1024);
    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-q4_k.gguf",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(600.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("the explicit worker RSS limit must terminate the worker safely");
    let message = error.to_string();
    assert!(
        message.contains("SBOBINO_PARAKEET_MEMORY_LIMIT"),
        "{message}"
    );
    assert!(
        message.contains("peak") && message.contains("limit"),
        "{message}"
    );

    let pids = std::fs::read_to_string(&child_pids)
        .expect("worker should record each spawned helper PID")
        .lines()
        .map(|line| line.parse::<i32>().expect("worker child PID"))
        .collect::<Vec<_>>();
    assert!(
        pids.len() >= 4,
        "every 30/20/15/10 retry must terminate its own worker group, got {pids:?}"
    );
    for pid in pids {
        let mut exited = false;
        for _ in 0..20 {
            let alive = unsafe { libc::kill(pid, 0) == 0 };
            if !alive {
                exited = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            exited,
            "worker helper PID {pid} was orphaned after RSS termination"
        );
    }
}

#[tokio::test]
async fn long_file_worker_passes_explicit_language_and_preserves_nemotron_markers() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("parakeet-cli");
    let worker_path = temp.path().join("parakeet-batch-json");
    let models_dir = temp.path().join("parakeet-models");
    let input_wav = temp.path().join("audio.wav");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(
        models_dir.join("nemotron-3.5-asr-streaming-0.6b-q4_k.gguf"),
        b"fake model",
    )
    .expect("failed to create model");
    write_test_wav(&input_wav, 605);
    write_executable_script(
        &script_path,
        "#!/bin/sh\necho 'chunk CLI must not run when worker succeeds' >&2\nexit 45\n",
    );
    write_executable_script(
        &worker_path,
        r#"#!/bin/sh
manifest=""
lang=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --manifest) manifest="$2"; shift 2 ;;
    --lang) lang="$2"; shift 2 ;;
    *) shift ;;
  esac
done
[ "$lang" = "it" ] || { echo "worker language must be it, got $lang" >&2; exit 46; }
while IFS="$(printf '\t')" read -r idx decode_start decode_end commit_start commit_end path; do
  echo "{\"index\":$idx,\"decode_start\":$decode_start,\"decode_end\":$decode_end,\"commit_start\":$commit_start,\"commit_end\":$commit_end,\"result\":{\"text\":\"<it-IT>Ciao <en-US>Hello\"}}"
done < "$manifest"
"#,
    );

    let engine = ParakeetCppEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let transcript = engine
        .transcribe(
            &input_wav,
            "nemotron-3.5-asr-streaming-0.6b-q4_k.gguf",
            &transcription_policy("it"),
            &WhisperOptions::default(),
            Some(605.0),
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect("long Nemotron worker transcription should succeed");

    assert!(!transcript.text.contains("<it-IT>"));
    assert!(transcript
        .segments
        .iter()
        .any(|segment| segment.language_code.as_deref() == Some("it")));
    assert!(transcript
        .segments
        .iter()
        .any(|segment| segment.language_code.as_deref() == Some("en")));
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
            &transcription_policy("en"),
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
            &transcription_policy("en"),
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
            &transcription_policy("it"),
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
            &transcription_policy("it"),
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
            &transcription_policy("en"),
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
            &transcription_policy("it"),
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
async fn transcribe_rejects_valid_empty_single_file_cli_json() {
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
echo '{}'
exit 0
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
            &transcription_policy("it"),
            &WhisperOptions::default(),
            None,
            Arc::new(|_line: String| {}),
            Arc::new(|_seconds: f32| {}),
        )
        .await
        .expect_err("a valid but empty single-file CLI response must fail");

    assert!(error
        .to_string()
        .contains("parakeet-cli produced empty output"));
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
    let options = WhisperOptions {
        translate_to_english: true,
        ..WhisperOptions::default()
    };

    let error = engine
        .transcribe(
            &input_wav,
            "tdt-0.6b-v3-f16.gguf",
            &transcription_policy("en"),
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
            &transcription_policy("auto"),
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
        emitted
            .last()
            .and_then(|line| line.strip_prefix("\u{001F}REPLACE:")),
        Some(transcript.text.as_str()),
        "final Parakeet delta should carry the final transcript after its protocol prefix"
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
