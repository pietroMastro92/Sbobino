#![cfg(unix)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::tempdir;

use sbobino_application::{RealtimeDelta, RealtimeDeltaKind};
use sbobino_infrastructure::adapters::whisper_stream::{
    WhisperStreamEngine, WhisperStreamTelemetry,
};

const STREAM_SETTLE_ATTEMPTS: usize = 320;
const STREAM_SETTLE_DELAY: Duration = Duration::from_millis(25);

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
async fn realtime_start_waits_for_post_warmup_capture_marker() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");
    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf 'whisper_init: warming up\n' 1>&2
sleep 0.25
printf '[Start speaking]\n'
sleep 1
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let started = Instant::now();
    engine
        .start(
            "ggml-base.bin",
            "auto",
            Arc::new(|_delta: RealtimeDelta| {}),
        )
        .await
        .expect("start should resolve after the capture-ready marker");
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "start returned before the delayed capture-ready marker"
    );
    assert!(engine.is_running().await);
    let _ = engine
        .stop()
        .await
        .expect("ready session should stop cleanly");
}

#[tokio::test]
async fn realtime_start_reports_exit_before_capture_ready() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");
    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_executable_script(
        &script_path,
        "#!/bin/sh\nprintf 'whisper_init: failed before capture\\n' 1>&2\nexit 9\n",
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let error = engine
        .start(
            "ggml-base.bin",
            "auto",
            Arc::new(|_delta: RealtimeDelta| {}),
        )
        .await
        .expect_err("pre-marker exit must fail the start command");
    assert!(
        error
            .to_string()
            .contains("before microphone capture became ready")
            || error.to_string().contains("failed before capture")
            || error.to_string().contains("status 9"),
        "unexpected startup error: {error}"
    );
}

#[tokio::test]
async fn realtime_start_rejects_exact_backend_warmup_failure() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");
    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_executable_script(
        &script_path,
        "#!/bin/sh\nprintf '%s: failed to warm up the live transcription backend\\n' \"$0\" 1>&2\nexit 6\n",
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let error = engine
        .start(
            "ggml-base.bin",
            "auto",
            Arc::new(|_delta: RealtimeDelta| {}),
        )
        .await
        .expect_err("warmup failure must never be accepted as transcript readiness");
    assert!(
        error
            .to_string()
            .contains("failed to warm up the live transcription backend"),
        "unexpected warmup error: {error}"
    );
    assert!(!engine.is_running().await);
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

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if !engine.snapshot_text().await.trim().is_empty()
            || !emitted.lock().expect("emit lock poisoned").is_empty()
        {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
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
async fn realtime_emits_diagnostic_telemetry_with_first_preview_and_finalization() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_executable_script(
        &script_path,
        r#"#!/bin/sh
case "$*" in
  *"-l en"*) ;;
  *) echo "expected explicit -l en, got: $*" 1>&2; exit 41 ;;
esac
printf '\033[2Kpreview telemetry\n' 1>&2
sleep 0.08
printf 'final telemetry\n' 1>&2
sleep 0.08
exit 0
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let telemetry: Arc<Mutex<Vec<WhisperStreamTelemetry>>> = Arc::new(Mutex::new(Vec::new()));
    let telemetry_clone = telemetry.clone();
    engine
        .start_with_telemetry(
            "ggml-base.bin",
            "en",
            Arc::new(|_delta: RealtimeDelta| {}),
            Some(Arc::new(move |event| {
                telemetry_clone
                    .lock()
                    .expect("telemetry lock poisoned")
                    .push(event);
            })),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if !engine.is_running().await {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
    }
    let _ = engine.stop().await.expect("realtime stop should succeed");

    let events = telemetry.lock().expect("telemetry lock poisoned").clone();
    assert!(
        events.len() >= 2,
        "expected preview and final telemetry: {events:?}"
    );
    let first_preview = events
        .iter()
        .find(|event| event.first_preview_ms.is_some())
        .expect("first preview timing should be populated");
    assert!(first_preview.captured_seconds.is_finite());
    assert!(first_preview.processed_seconds.is_finite());
    assert!(first_preview.backlog_seconds >= 0.0);
    assert!(first_preview.inference_ms.is_some());
    let final_event = events
        .last()
        .expect("final telemetry event should be emitted");
    assert!(final_event.finalization_ms.is_some());
    assert!(final_event.finalization_ms.unwrap_or_default() >= 0.0);
}

#[tokio::test]
async fn realtime_backlog_failure_emits_immediate_degraded_telemetry() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");
    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");
    write_executable_script(
        &script_path,
        r#"#!/bin/sh
printf 'SBOBINO_WHISPER_LIVE_METRIC captured_seconds=3.000 processed_seconds=0.500 backlog_seconds=2.500 inference_ms=2500.000 dropped_samples=0\n' 1>&2
printf 'SBOBINO_WHISPER_LIVE_BACKLOG exceeded captured=48000 inferred=8000 buffered=40000 dropped=0\n' 1>&2
exit 7
"#,
    );

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );
    let telemetry: Arc<Mutex<Vec<WhisperStreamTelemetry>>> = Arc::new(Mutex::new(Vec::new()));
    let telemetry_clone = telemetry.clone();
    engine
        .start_with_telemetry(
            "ggml-base.bin",
            "auto",
            Arc::new(|_delta: RealtimeDelta| {}),
            Some(Arc::new(move |event| {
                telemetry_clone
                    .lock()
                    .expect("telemetry lock poisoned")
                    .push(event);
            })),
        )
        .await
        .expect("realtime start should succeed before the simulated backlog");

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if telemetry
            .lock()
            .expect("telemetry lock poisoned")
            .iter()
            .any(|event| event.backlog_seconds > 2.0)
        {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
    }
    assert!(
        telemetry
            .lock()
            .expect("telemetry lock poisoned")
            .iter()
            .any(|event| event.backlog_seconds > 2.0),
        "the UI telemetry sink must be notified before Stop"
    );
    let error = engine
        .stop()
        .await
        .expect_err("backlog failure must remain terminal");
    assert!(error.to_string().contains("could not keep up"));
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

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if !engine.snapshot_text().await.trim().is_empty()
            || emitted.lock().expect("emit lock poisoned").len() >= 2
        {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
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

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
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
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
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

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if !engine.snapshot_text().await.trim().is_empty() {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
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

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if !engine.snapshot_text().await.trim().is_empty() {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
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

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if emitted
            .lock()
            .expect("emit lock poisoned")
            .iter()
            .any(|delta| matches!(delta.kind, RealtimeDeltaKind::UpdatePreview))
        {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

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

    let error = engine
        .start("ggml-base.bin", "it", Arc::new(|_delta: RealtimeDelta| {}))
        .await
        .expect_err("fatal pre-capture diagnostics must fail start immediately");
    assert!(error.to_string().contains("capture devices"));

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

#[tokio::test]
async fn realtime_start_uses_managed_runtime_bin_dir_on_path() {
    let temp = tempdir().expect("failed to create temp dir");
    let bin_dir = temp.path().join("bin");
    let script_path = bin_dir.join("whisper-stream");
    let models_dir = temp.path().join("models");

    std::fs::create_dir_all(&bin_dir).expect("failed to create bin dir");
    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    let script = r#"#!/bin/sh
if [ "${PATH#"$PWD"}" = "$PATH" ]; then
  :
fi
case "$PATH" in
  "__BIN_DIR__":*) ;;
  *)
    printf 'path-missing\n' 1>&2
    exit 1
    ;;
esac
printf 'managed env ok\n' 1>&2
sleep 0.2
exit 0
"#
    .replace("__BIN_DIR__", &bin_dir.to_string_lossy());
    write_executable_script(&script_path, &script);

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    engine
        .start("ggml-base.bin", "en", Arc::new(|_delta: RealtimeDelta| {}))
        .await
        .expect("realtime start should succeed");

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if !engine.snapshot_text().await.trim().is_empty() {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    assert!(
        stop_result.transcript.contains("managed env ok"),
        "expected managed runtime env confirmation, got: {}",
        stop_result.transcript
    );
}

#[tokio::test]
async fn realtime_start_passes_auto_language_to_avoid_english_default() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");
    let args_path = temp.path().join("args.txt");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    let script = r#"#!/bin/sh
printf '%s\n' "$@" > "__ARGS_PATH__"
printf 'auto language ok\n' 1>&2
sleep 0.2
exit 0
"#
    .replace("__ARGS_PATH__", &args_path.to_string_lossy());
    write_executable_script(&script_path, &script);

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    engine
        .start(
            "ggml-base.bin",
            "auto",
            Arc::new(|_delta: RealtimeDelta| {}),
        )
        .await
        .expect("realtime start should succeed");

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if args_path.exists() {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    assert!(
        stop_result.transcript.contains("auto language ok"),
        "expected fake realtime transcript, got: {}",
        stop_result.transcript
    );

    let args = std::fs::read_to_string(&args_path).expect("expected captured args");
    let args = args.lines().collect::<Vec<_>>();
    assert!(
        args.windows(2).any(|pair| matches!(pair, ["-l", "auto"])),
        "expected whisper-stream to receive '-l auto', got: {args:?}"
    );
}

#[tokio::test]
async fn realtime_start_passes_explicit_language_when_preference_is_selected() {
    let temp = tempdir().expect("failed to create temp dir");
    let script_path = temp.path().join("whisper-stream");
    let models_dir = temp.path().join("models");
    let args_path = temp.path().join("args.txt");

    std::fs::create_dir_all(&models_dir).expect("failed to create models dir");
    std::fs::write(models_dir.join("ggml-base.bin"), b"fake model")
        .expect("failed to create model");

    let script = r#"#!/bin/sh
printf '%s\n' "$@" > "__ARGS_PATH__"
printf 'italian language ok\n' 1>&2
sleep 0.2
exit 0
"#
    .replace("__ARGS_PATH__", &args_path.to_string_lossy());
    write_executable_script(&script_path, &script);

    let engine = WhisperStreamEngine::new(
        script_path.to_string_lossy().to_string(),
        models_dir.to_string_lossy().to_string(),
    );

    engine
        .start("ggml-base.bin", "it", Arc::new(|_delta: RealtimeDelta| {}))
        .await
        .expect("realtime start should succeed");

    for _ in 0..STREAM_SETTLE_ATTEMPTS {
        if args_path.exists() {
            break;
        }
        tokio::time::sleep(STREAM_SETTLE_DELAY).await;
    }

    let stop_result = engine.stop().await.expect("realtime stop should succeed");
    assert!(
        stop_result.transcript.contains("italian language ok"),
        "expected fake realtime transcript, got: {}",
        stop_result.transcript
    );

    let args = std::fs::read_to_string(&args_path).expect("expected captured args");
    let args = args.lines().collect::<Vec<_>>();
    assert!(
        args.windows(2).any(|pair| matches!(pair, ["-l", "it"])),
        "expected whisper-stream to receive '-l it', got: {args:?}"
    );
}
