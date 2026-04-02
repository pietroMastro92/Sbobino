#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use sbobino_application::{RealtimeDelta, RealtimeDeltaKind};
use sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine;

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
async fn realtime_collects_preview_and_final_lines_from_stderr() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf '\033[2Kpreview from stderr\n' 1>&2
printf 'final from stderr\n' 1>&2
exit 0
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    engine
        .start(
            "ggml-base.bin",
            "en",
            Arc::new(move |delta: RealtimeDelta| {
                emitted_clone
                    .lock()
                    .expect("emit lock poisoned")
                    .push(delta);
            }),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..40 {
        if !engine.snapshot_text().await.trim().is_empty()
            || !emitted.lock().expect("emit lock poisoned").is_empty()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    let deltas = emitted.lock().expect("emit lock poisoned").clone();

    assert!(
        deltas.iter().any(|delta| {
            matches!(delta.kind, RealtimeDeltaKind::UpdatePreview)
                && delta.text == "preview from stderr"
        }),
        "expected preview delta from stderr, got: {deltas:?}"
    );
    assert!(
        deltas.iter().any(|delta| {
            matches!(delta.kind, RealtimeDeltaKind::AppendFinal)
                && delta.text == "final from stderr"
        }),
        "expected final delta from stderr, got: {deltas:?}"
    );
    assert_eq!(stop_result.transcript.trim(), "final from stderr");
}

#[tokio::test]
async fn realtime_splits_carriage_return_live_updates_like_python_text_mode() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf '\033[2Kafter the war is complete.\r' 1>&2
printf '\033[2Kafter the war is completed.\r' 1>&2
printf 'After the war is completed.\n' 1>&2
exit 0
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    engine
        .start(
            "ggml-base.bin",
            "en",
            Arc::new(move |delta: RealtimeDelta| {
                emitted_clone
                    .lock()
                    .expect("emit lock poisoned")
                    .push(delta);
            }),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..80 {
        if !engine.snapshot_text().await.trim().is_empty()
            || emitted.lock().expect("emit lock poisoned").len() >= 2
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    let deltas = emitted.lock().expect("emit lock poisoned").clone();

    assert!(
        deltas.iter().any(|delta| {
            matches!(delta.kind, RealtimeDeltaKind::UpdatePreview)
                && delta.text == "after the war is complete."
        }),
        "expected first carriage-return preview delta, got: {deltas:?}"
    );
    assert!(
        deltas.iter().any(|delta| {
            matches!(delta.kind, RealtimeDeltaKind::UpdatePreview)
                && delta.text == "after the war is completed."
        }),
        "expected second carriage-return preview delta, got: {deltas:?}"
    );
    assert_eq!(stop_result.transcript.trim(), "After the war is completed.");
}

#[tokio::test]
async fn realtime_keeps_each_finalized_line_in_history() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf 'I always use it for little more sticky product.\n' 1>&2
sleep 0.1
printf 'I always use it for little more sticky product today.\n' 1>&2
exit 0
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    engine
        .start(
            "ggml-base.bin",
            "en",
            Arc::new(move |delta: RealtimeDelta| {
                emitted_clone
                    .lock()
                    .expect("emit lock poisoned")
                    .push(delta);
            }),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..120 {
        let finalized_count = emitted
            .lock()
            .expect("emit lock poisoned")
            .iter()
            .filter(|delta| matches!(delta.kind, RealtimeDeltaKind::AppendFinal))
            .count();
        if finalized_count >= 2
            || engine
                .snapshot_text()
                .await
                .contains("I always use it for little more sticky product today.")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    let deltas = emitted.lock().expect("emit lock poisoned").clone();

    assert_eq!(
        stop_result.transcript.trim(),
        "I always use it for little more sticky product.\nI always use it for little more sticky product today."
    );
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta.kind, RealtimeDeltaKind::AppendFinal))
            .count(),
        2,
        "expected both finalized lines to be preserved, got: {deltas:?}"
    );
}

#[tokio::test]
async fn realtime_filters_whisper_runtime_noise_from_final_output() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf 'ggml_metal_device_init: GPU name: Apple M3\n' 1>&2
printf '[Start speaking]\n' 1>&2
printf 'ciao a tutti\n' 1>&2
exit 0
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    engine
        .start(
            "ggml-base.bin",
            "it",
            Arc::new(move |delta: RealtimeDelta| {
                emitted_clone
                    .lock()
                    .expect("emit lock poisoned")
                    .push(delta);
            }),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..80 {
        if !engine.snapshot_text().await.trim().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    let deltas = emitted.lock().expect("emit lock poisoned").clone();

    assert_eq!(stop_result.transcript.trim(), "ciao a tutti");
    assert_eq!(
        deltas.len(),
        1,
        "expected only the spoken line to be emitted"
    );
    assert!(matches!(deltas[0].kind, RealtimeDeltaKind::AppendFinal));
    assert_eq!(deltas[0].text, "ciao a tutti");
}

#[tokio::test]
async fn realtime_discovers_saved_audio_file_from_session_directory() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
trap 'printf "fake audio bytes" > live-session.wav; exit 0' TERM INT
printf 'memo vocale\n' 1>&2
while true; do
  sleep 1
done
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    engine
        .start("ggml-base.bin", "it", Arc::new(|_delta: RealtimeDelta| {}))
        .await
        .expect("realtime start should succeed");

    for _ in 0..80 {
        if !engine.snapshot_text().await.trim().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    let saved_audio_path = stop_result
        .saved_audio_path
        .expect("expected saved audio path");

    assert_eq!(stop_result.transcript.trim(), "memo vocale");
    assert_eq!(
        saved_audio_path.file_name().and_then(|name| name.to_str()),
        Some("live-session.wav")
    );
    assert!(
        saved_audio_path.exists(),
        "saved audio file should exist at {}",
        saved_audio_path.display()
    );
}

#[tokio::test]
async fn realtime_flushes_the_last_preview_into_the_saved_transcript_when_stopping() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
trap 'exit 0' INT TERM
printf '\033[2KThe last preview should be saved\n' 1>&2
while true; do
  sleep 1
done
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    engine
        .start(
            "ggml-base.bin",
            "en",
            Arc::new(move |delta: RealtimeDelta| {
                emitted_clone
                    .lock()
                    .expect("emit lock poisoned")
                    .push(delta);
            }),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..40 {
        if emitted
            .lock()
            .expect("emit lock poisoned")
            .iter()
            .any(|delta| matches!(delta.kind, RealtimeDeltaKind::UpdatePreview))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(120)).await;

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    assert_eq!(
        stop_result.transcript.trim(),
        "The last preview should be saved"
    );
}

#[tokio::test]
async fn realtime_tracks_startup_diagnostics_from_stderr() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf 'init: found 0 capture devices:\n' 1>&2
printf 'main: audio.init() failed!\n' 1>&2
exit 1
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    engine
        .start("ggml-base.bin", "it", Arc::new(|_delta: RealtimeDelta| {}))
        .await
        .expect("realtime start should succeed before the child exits");

    for _ in 0..20 {
        if !engine.is_running().await {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert!(
        !engine.is_running().await,
        "engine should report stopped after startup failure"
    );

    let diagnostics = engine.snapshot_diagnostics().await.join(" ");
    assert!(
        diagnostics.contains("capture devices"),
        "expected capture-device diagnostic, got: {diagnostics}"
    );
    assert!(
        diagnostics.contains("audio.init() failed"),
        "expected startup failure diagnostic, got: {diagnostics}"
    );
}
