use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use sbobino_application::{ApplicationError, RealtimeDelta, TranscriptionService};
use sbobino_domain::{
    AppSettings, ArtifactKind, ArtifactSourceOrigin, LanguageCode, ParakeetModel, SpeechModel,
    TimedSegment, TranscriptArtifact, TranscriptionEngine, TranscriptionOutput,
};
use tracing::warn;

use crate::parakeet_realtime::{ParakeetRealtimeEngine, ParakeetRealtimeStopResult};
use crate::realtime_audio::{emit_level_event, start_input_preview, RealtimeInputLevelEvent};
use crate::{error::CommandError, state::AppState};

fn resolve_realtime_engine(
    state: &AppState,
) -> Result<sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine, CommandError> {
    match state.runtime_factory.build_whisper_stream_engine() {
        Ok(engine) => Ok(engine),
        Err(error) => {
            if state.runtime_factory.managed_runtime_required() {
                return Err(CommandError::from(ApplicationError::SpeechToText(error)));
            }

            let settings = state
                .runtime_factory
                .load_settings()
                .map_err(|load_error| CommandError::new("settings", load_error))?;
            let whisper_stream_path = state.runtime_factory.resolve_binary_path(
                &settings.transcription.whisperkit_cli_path,
                "whisper-stream",
            );
            let models_dir = state
                .runtime_factory
                .resolve_models_dir(&settings.transcription.models_dir);
            Ok(
                sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine::new(
                    whisper_stream_path,
                    models_dir,
                ),
            )
        }
    }
}

fn resolve_parakeet_live_engine(state: &AppState) -> Result<ParakeetRealtimeEngine, CommandError> {
    let (lib_path, models_dir) = resolve_parakeet_live_runtime_paths(state)?;
    Ok(ParakeetRealtimeEngine::new(lib_path, models_dir.into()))
}

fn resolve_parakeet_live_runtime_paths(
    state: &AppState,
) -> Result<(PathBuf, String), CommandError> {
    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|load_error| CommandError::new("settings", load_error))?;
    let parakeet_cli_path = state
        .runtime_factory
        .resolve_binary_path(&settings.transcription.parakeet_cli_path, "parakeet-cli");
    eprintln!(
        "[parakeet-live] resolve_parakeet_live_engine: cli_path={}",
        parakeet_cli_path
    );
    let lib_path = resolve_parakeet_live_library_path(&parakeet_cli_path);
    eprintln!("[parakeet-live] resolved lib_path={}", lib_path.display());
    if !lib_path.exists() {
        return Err(CommandError::from(ApplicationError::SpeechToText(format!(
            "Parakeet live library not found at {}. Reinstall the local runtime from Settings > Local Models.",
            lib_path.display()
        ))));
    }
    let models_dir = state
        .runtime_factory
        .resolve_models_dir(&settings.transcription.parakeet_models_dir);
    Ok((lib_path, models_dir))
}

pub(crate) fn resolve_parakeet_live_library_path(parakeet_cli_path: impl AsRef<Path>) -> PathBuf {
    let cli_path = parakeet_cli_path.as_ref();
    let Some(bin_dir) = cli_path.parent() else {
        return PathBuf::from("libparakeet.dylib");
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    if bin_dir.file_name().and_then(|name| name.to_str()) == Some("bin") {
        // Managed install layout: <root>/bin/parakeet-cli -> <root>/lib/libparakeet.dylib
        candidates.push(bin_dir.join("../lib/libparakeet.dylib"));
        if let Some(parent) = bin_dir.parent() {
            candidates.push(parent.join("lib/libparakeet.dylib"));
            candidates.push(parent.join("libparakeet.dylib"));
            candidates.push(parent.join("../lib/libparakeet.dylib"));
        }
    } else {
        candidates.push(bin_dir.join("libparakeet.dylib"));
        if let Some(parent) = bin_dir.parent() {
            candidates.push(parent.join("lib/libparakeet.dylib"));
            candidates.push(parent.join("libparakeet.dylib"));
        }
    }
    if let Some(parent) = cli_path.parent() {
        candidates.push(parent.join("libparakeet.dylib"));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("lib/libparakeet.dylib"));
        }
    }
    let mut seen: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if !seen.iter().any(|existing| existing == &candidate) {
            seen.push(candidate);
        }
    }
    for candidate in &seen {
        if candidate.exists() {
            return candidate
                .canonicalize()
                .unwrap_or_else(|_| candidate.clone());
        }
    }
    // Fall back to the first candidate so the loader can produce a clear error
    // message that points to the expected path.
    seen.into_iter()
        .next()
        .unwrap_or_else(|| bin_dir.join("libparakeet.dylib"))
}

pub(crate) fn parakeet_live_target_lang(language: LanguageCode) -> &'static str {
    match language {
        LanguageCode::Auto => "auto",
        LanguageCode::En => "en",
        LanguageCode::It => "it",
        LanguageCode::Fr => "fr",
        LanguageCode::De => "de",
        LanguageCode::Es => "es",
        LanguageCode::Pt => "pt",
        LanguageCode::Zh => "zh",
        // Nemotron's locale dictionary uses ja-JP, not bare ja.
        LanguageCode::Ja => "ja-JP",
    }
}

pub(crate) fn select_parakeet_live_model(
    models_dir: &Path,
    requested: ParakeetModel,
    _language: LanguageCode,
) -> Result<ParakeetModel, CommandError> {
    let live_candidates = [
        ParakeetModel::Nemotron35AsrStreaming06bQ4,
        ParakeetModel::Nemotron35AsrStreaming06bQ8,
        ParakeetModel::Nemotron35AsrStreaming06bF16,
    ];

    if requested.is_multilingual_streaming() && models_dir.join(requested.gguf_filename()).exists()
    {
        return Ok(requested);
    }

    for candidate in live_candidates {
        if models_dir.join(candidate.gguf_filename()).exists() {
            return Ok(candidate);
        }
    }

    Err(CommandError::from(ApplicationError::SpeechToText(format!(
        "Parakeet live requires the NVIDIA Nemotron live model in {}. Install it from Settings > Local Models.",
        models_dir.display()
    ))))
}

#[derive(Debug, Deserialize)]
pub struct StartRealtimePayload {
    #[serde(default)]
    pub engine: Option<TranscriptionEngine>,
    pub model: Option<SpeechModel>,
    #[serde(default)]
    pub parakeet_model: Option<ParakeetModel>,
    pub language: Option<LanguageCode>,
    pub resume_artifact_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartRealtimeResponse {
    pub started: bool,
}

#[derive(Debug, Deserialize)]
pub struct StopRealtimePayload {
    pub save: Option<bool>,
    pub title: Option<String>,
    pub elapsed_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StopRealtimeResponse {
    pub saved: bool,
    pub artifact: Option<TranscriptArtifact>,
}

struct RealtimeStopResult {
    transcript: String,
    segments: Vec<TimedSegment>,
    saved_audio_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStatusEvent {
    pub state: String,
    pub message: String,
}

async fn stop_realtime_preview(
    app: &tauri::AppHandle,
    state: &AppState,
    final_state: &str,
    message: &str,
) {
    if let Some(preview) = state.realtime.preview.lock().await.take() {
        preview.stop(app, final_state, message);
    }
}

async fn start_realtime_preview(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), CommandError> {
    stop_realtime_preview(app, state, "idle", "Microphone preview reset.").await;
    let preview = start_input_preview(app)
        .map_err(|error| CommandError::from(ApplicationError::SpeechToText(error.message)))?;
    *state.realtime.preview.lock().await = Some(preview);
    Ok(())
}

async fn apply_realtime_speaker_diarization(
    state: &AppState,
    settings: &AppSettings,
    audio_path: Option<&Path>,
    segments: &mut Vec<TimedSegment>,
    metadata: &mut BTreeMap<String, String>,
) {
    if !settings.transcription.speaker_diarization.enabled {
        return;
    }

    let Some(audio_path) = audio_path else {
        metadata.insert(
            "speaker_diarization_status".to_string(),
            "skipped_no_audio".to_string(),
        );
        return;
    };

    if segments.is_empty() {
        metadata.insert(
            "speaker_diarization_status".to_string(),
            "skipped_no_segments".to_string(),
        );
        return;
    }

    let diarizer = match state.runtime_factory.build_speaker_diarizer(settings) {
        Ok(Some(diarizer)) => diarizer,
        Ok(None) => return,
        Err(error) => {
            metadata.insert(
                "speaker_diarization_status".to_string(),
                "failed".to_string(),
            );
            metadata.insert("speaker_diarization_error".to_string(), error.clone());
            warn!("speaker diarization skipped for realtime session: {error}");
            return;
        }
    };

    match diarizer.diarize(audio_path).await {
        Ok(turns) => {
            metadata.insert(
                "speaker_diarization_status".to_string(),
                "completed".to_string(),
            );
            if !turns.is_empty() {
                *segments = TranscriptionService::assign_speakers_to_segments(segments, &turns);
            }
        }
        Err(error) => {
            metadata.insert(
                "speaker_diarization_status".to_string(),
                "failed".to_string(),
            );
            metadata.insert("speaker_diarization_error".to_string(), error.to_string());
            warn!("speaker diarization failed for realtime session: {error}");
        }
    }
}

#[tauri::command]
pub async fn start_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<StartRealtimePayload>,
) -> Result<StartRealtimeResponse, CommandError> {
    eprintln!("[realtime-start] command received payload={payload:?}");
    let payload = payload.unwrap_or(StartRealtimePayload {
        engine: None,
        model: None,
        parakeet_model: None,
        language: None,
        resume_artifact_id: None,
    });

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let default_model = settings.transcription.model;
    let default_language = settings.transcription.language;
    let engine_kind = payload
        .engine
        .unwrap_or_else(|| settings.transcription.engine.clone());
    let model = payload.model.unwrap_or(default_model);
    let language = payload.language.unwrap_or(default_language);

    if let Some(id) = &payload.resume_artifact_id {
        let artifact = state
            .artifact_service
            .get(id)
            .await
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::new("not_found", "realtime session not found"))?;

        *state.realtime.session_name.lock().await = Some(artifact.title.clone());
    } else {
        *state.realtime.session_name.lock().await = None;
    }

    *state.realtime.language_code.lock().await = language.as_whisper_code().to_string();
    *state.realtime.active_engine.lock().await = engine_kind.clone();

    let app_handle = app.clone();
    let emit_delta = Arc::new(move |delta: RealtimeDelta| {
        let _ = app_handle.emit("realtime://delta", delta);
    });
    let app_handle = app.clone();
    let emit_input_level = Arc::new(move |event: RealtimeInputLevelEvent| {
        let _ = app_handle.emit("realtime://input_level", event);
    });
    let mut running_message = "Live listening".to_string();

    eprintln!(
        "[realtime-start] selected engine={engine_kind:?} model={model:?} language={language:?}"
    );
    match engine_kind {
        TranscriptionEngine::WhisperCpp => {
            let engine = resolve_realtime_engine(&state)?;
            {
                let mut current_engine = state.realtime.engine.lock().await;
                *current_engine = engine.clone();
            }
            if let Some(id) = &payload.resume_artifact_id {
                if let Some(artifact) = state
                    .artifact_service
                    .get(id)
                    .await
                    .map_err(CommandError::from)?
                {
                    engine.seed_buffer(&artifact.raw_transcript).await;
                }
            } else {
                engine.reset().await;
            }
            *state.realtime.model_filename.lock().await = Some(model.ggml_filename().to_string());

            start_realtime_preview(&app, &state).await?;

            if let Err(error) = engine
                .start(
                    model.ggml_filename(),
                    language.as_whisper_code(),
                    emit_delta,
                )
                .await
            {
                stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
                return Err(CommandError::from(error));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            eprintln!("[realtime-start] parakeet branch entered");
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
            let parakeet_engine = resolve_parakeet_live_engine(&state)?;
            let models_dir = state
                .runtime_factory
                .resolve_models_dir(&settings.transcription.parakeet_models_dir);
            let requested_model = payload
                .parakeet_model
                .unwrap_or(settings.transcription.parakeet_model);
            let live_model = select_parakeet_live_model(
                std::path::Path::new(&models_dir),
                requested_model.clone(),
                language.clone(),
            )?;
            eprintln!(
                "[realtime-start] parakeet live model requested={requested_model:?} selected={live_model:?} models_dir={models_dir}"
            );
            running_message = "Live listening".to_string();
            if let Some(id) = &payload.resume_artifact_id {
                if let Some(artifact) = state
                    .artifact_service
                    .get(id)
                    .await
                    .map_err(CommandError::from)?
                {
                    parakeet_engine.seed_buffer(&artifact.raw_transcript).await;
                }
            } else {
                parakeet_engine.reset().await;
            }
            *state.realtime.model_filename.lock().await =
                Some(live_model.gguf_filename().to_string());
            {
                let mut current_engine = state.realtime.parakeet_engine.lock().await;
                *current_engine = Some(parakeet_engine.clone());
            }

            eprintln!("[realtime-start] starting parakeet engine");
            if let Err(error) = parakeet_engine
                .start(
                    live_model.gguf_filename(),
                    parakeet_live_target_lang(language.clone()),
                    emit_delta,
                    emit_input_level.clone(),
                )
                .await
            {
                eprintln!("[realtime-start] parakeet engine start failed: {error}");
                emit_level_event(&app, "idle", 0.0, error.to_string());
                return Err(CommandError::from(error));
            }
            eprintln!("[realtime-start] parakeet engine start returned ok");
        }
    }

    sleep(Duration::from_millis(350)).await;
    let running = match engine_kind {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            engine.is_running().await
        }
        TranscriptionEngine::ParakeetCpp => {
            let engine = state.realtime.parakeet_engine.lock().await.clone();
            match engine {
                Some(engine) => engine.is_running().await,
                None => false,
            }
        }
    };
    eprintln!("[realtime-start] post-start running={running}");
    if !running {
        match engine_kind {
            TranscriptionEngine::WhisperCpp => {
                stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
            }
            TranscriptionEngine::ParakeetCpp => {
                emit_level_event(&app, "idle", 0.0, "Microphone preview stopped.");
            }
        }
        let diagnostics = match engine_kind {
            TranscriptionEngine::WhisperCpp => {
                let engine = state.realtime.engine.lock().await.clone();
                engine.snapshot_diagnostics().await
            }
            TranscriptionEngine::ParakeetCpp => {
                let engine = state.realtime.parakeet_engine.lock().await.clone();
                match engine {
                    Some(engine) => engine.snapshot_diagnostics().await,
                    None => Vec::new(),
                }
            }
        };
        let detail = if diagnostics.is_empty() {
            "Realtime transcription stopped immediately. Verify microphone access and that at least one audio input device is available.".to_string()
        } else {
            diagnostics.join(" ")
        };
        return Err(CommandError::from(ApplicationError::SpeechToText(detail)));
    }

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "running".to_string(),
            message: running_message,
        },
    );

    eprintln!("[realtime-start] command completed started=true");
    Ok(StartRealtimeResponse { started: true })
}

#[tauri::command]
pub async fn pause_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    match state.realtime.active_engine.lock().await.clone() {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            engine.pause().await.map_err(CommandError::from)?;
            stop_realtime_preview(&app, &state, "paused", "Microphone preview paused.").await;
        }
        TranscriptionEngine::ParakeetCpp => {
            if let Some(engine) = state.realtime.parakeet_engine.lock().await.clone() {
                engine.pause().await.map_err(CommandError::from)?;
            }
            emit_level_event(&app, "paused", 0.0, "Microphone preview paused.");
        }
    }

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "paused".to_string(),
            message: "Live paused".to_string(),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn resume_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    match state.realtime.active_engine.lock().await.clone() {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            start_realtime_preview(&app, &state).await?;
            engine.resume().await.map_err(CommandError::from)?;
        }
        TranscriptionEngine::ParakeetCpp => {
            if let Some(engine) = state.realtime.parakeet_engine.lock().await.clone() {
                engine.resume().await.map_err(CommandError::from)?;
            }
            emit_level_event(&app, "running", 0.0, "Microphone preview resumed.");
        }
    }

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "running".to_string(),
            message: "Live resumed".to_string(),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn stop_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<StopRealtimePayload>,
) -> Result<StopRealtimeResponse, CommandError> {
    let payload = payload.unwrap_or(StopRealtimePayload {
        save: Some(true),
        title: None,
        elapsed_seconds: None,
    });
    let save = payload.save.unwrap_or(true);

    let active_engine = state.realtime.active_engine.lock().await.clone();
    let mut stop_result = match active_engine {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            let result = engine.stop().await.map_err(CommandError::from)?;
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
            RealtimeStopResult {
                transcript: result.transcript,
                segments: Vec::new(),
                saved_audio_path: result.saved_audio_path,
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            let engine = state
                .realtime
                .parakeet_engine
                .lock()
                .await
                .clone()
                .ok_or_else(|| CommandError::new("realtime", "Parakeet live is not running"))?;
            let result: ParakeetRealtimeStopResult =
                engine.stop().await.map_err(CommandError::from)?;
            emit_level_event(&app, "idle", 0.0, "Microphone preview stopped.");
            RealtimeStopResult {
                transcript: result.transcript,
                segments: result.segments,
                saved_audio_path: result.saved_audio_path,
            }
        }
    };

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "stopped".to_string(),
            message: "Live stopped".to_string(),
        },
    );

    if !save || stop_result.transcript.trim().is_empty() {
        return Ok(StopRealtimeResponse {
            saved: false,
            artifact: None,
        });
    }

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let session_title = state
        .realtime
        .session_name
        .lock()
        .await
        .clone()
        .or_else(|| {
            payload
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
        })
        .unwrap_or_else(|| format!("live_{}", Utc::now().format("%d%m%Y_%H%M%S")));

    let language_code = state.realtime.language_code.lock().await.clone();
    let model_filename = state
        .realtime
        .model_filename
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| settings.transcription.model.ggml_filename().to_string());

    let optimized = String::new();
    let summary = String::new();
    let faqs = String::new();

    let mut metadata = BTreeMap::new();
    metadata.insert("kind".to_string(), "realtime".to_string());
    metadata.insert("language".to_string(), language_code.clone());
    metadata.insert("model".to_string(), model_filename.clone());
    if let Some(elapsed_seconds) = payload.elapsed_seconds {
        metadata.insert("duration_seconds".to_string(), elapsed_seconds.to_string());
    }
    metadata.insert(
        "audio_saved".to_string(),
        if stop_result.saved_audio_path.is_some() {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );

    apply_realtime_speaker_diarization(
        &state,
        &settings,
        stop_result.saved_audio_path.as_deref(),
        &mut stop_result.segments,
        &mut metadata,
    )
    .await;

    if !stop_result.segments.is_empty() {
        metadata.insert(
            "timeline_v2".to_string(),
            TranscriptionOutput {
                text: stop_result.transcript.clone(),
                segments: stop_result.segments.clone(),
            }
            .timeline_v2_metadata_json(),
        );
    }

    let source_label = stop_result
        .saved_audio_path
        .as_ref()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{session_title}.wav"));

    let mut artifact = TranscriptArtifact::new(
        Uuid::new_v4().to_string(),
        session_title.clone(),
        ArtifactKind::Realtime,
        source_label,
        ArtifactSourceOrigin::Realtime,
        stop_result.transcript,
        optimized,
        summary,
        faqs,
        metadata,
    )
    .map_err(|e| CommandError::new("validation", e.to_string()))?;
    artifact.audio_available = stop_result.saved_audio_path.is_some();
    artifact.audio_duration_seconds = payload.elapsed_seconds.map(|value| value as f32);
    artifact.processing_engine = Some(match active_engine {
        TranscriptionEngine::WhisperCpp => "whisper_stream".to_string(),
        TranscriptionEngine::ParakeetCpp => "parakeet_cpp".to_string(),
    });
    artifact.processing_model = Some(model_filename.clone());
    artifact.processing_language = Some(state.realtime.language_code.lock().await.clone());
    if let Some(path) = stop_result.saved_audio_path.as_ref() {
        artifact.set_source_external_path(path.to_string_lossy().to_string());
    }

    state
        .artifact_service
        .save(&artifact)
        .await
        .map_err(CommandError::from)?;

    let _ = app.emit("realtime://saved", artifact.clone());

    Ok(StopRealtimeResponse {
        saved: true,
        artifact: Some(artifact),
    })
}

#[tauri::command]
pub async fn list_realtime_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<TranscriptArtifact>, CommandError> {
    state
        .artifact_service
        .list(sbobino_application::ArtifactQuery {
            kind: Some(ArtifactKind::Realtime),
            query: None,
            limit: Some(100),
            offset: Some(0),
        })
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn load_realtime_session(
    state: State<'_, AppState>,
    payload: crate::commands::artifacts::GetArtifactPayload,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?;

    if let Some(item) = &artifact {
        let engine = state.realtime.engine.lock().await.clone();
        engine.seed_buffer(&item.raw_transcript).await;
        *state.realtime.session_name.lock().await = Some(item.title.clone());
    }

    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parakeet_live_uses_installed_multilingual_model_when_tdt_is_selected() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path()
                .join(ParakeetModel::Nemotron35AsrStreaming06bQ4.gguf_filename()),
            b"model",
        )
        .expect("write multilingual live model");

        let selected =
            select_parakeet_live_model(temp.path(), ParakeetModel::Tdt06bV3Q4, LanguageCode::En)
                .expect("selected");

        assert_eq!(selected, ParakeetModel::Nemotron35AsrStreaming06bQ4);
    }

    #[test]
    fn parakeet_live_fails_closed_without_realtime_model() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path().join(ParakeetModel::Tdt06bV3Q4.gguf_filename()),
            b"model",
        )
        .expect("write tdt model");

        let error =
            select_parakeet_live_model(temp.path(), ParakeetModel::Tdt06bV3Q4, LanguageCode::It)
                .expect_err("tdt-only live should fail");

        assert!(
            error
                .message
                .contains("requires the NVIDIA Nemotron live model"),
            "{:?}",
            error
        );
    }

    #[test]
    fn parakeet_live_uses_nemotron_for_non_english_or_auto_language() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path()
                .join(ParakeetModel::Nemotron35AsrStreaming06bQ4.gguf_filename()),
            b"model",
        )
        .expect("write nemotron model");
        std::fs::write(
            temp.path()
                .join(ParakeetModel::RealtimeEou120mV1F16.gguf_filename()),
            b"model",
        )
        .expect("write english eou model");

        let selected =
            select_parakeet_live_model(temp.path(), ParakeetModel::Tdt06bV3Q4, LanguageCode::It)
                .expect("selected");
        assert_eq!(selected, ParakeetModel::Nemotron35AsrStreaming06bQ4);

        let auto_selected =
            select_parakeet_live_model(temp.path(), ParakeetModel::Tdt06bV3Q4, LanguageCode::Auto)
                .expect("auto selected");
        assert_eq!(auto_selected, ParakeetModel::Nemotron35AsrStreaming06bQ4);
    }
    #[test]
    fn parakeet_live_library_resolution_supports_managed_runtime_layout() {
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("runtime/bin");
        let lib_dir = temp.path().join("runtime/lib");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        std::fs::create_dir_all(&lib_dir).expect("lib dir");
        let cli = bin_dir.join("parakeet-cli");
        let lib = lib_dir.join("libparakeet.dylib");
        std::fs::write(&cli, b"cli").expect("cli");
        std::fs::write(&lib, b"lib").expect("lib");

        assert_eq!(
            resolve_parakeet_live_library_path(&cli),
            lib.canonicalize().expect("canonical lib")
        );
    }

    #[test]
    fn parakeet_live_library_resolution_supports_dev_sidecar_layout() {
        let temp = tempfile::tempdir().expect("temp dir");
        let binaries_dir = temp.path().join("apps/desktop/src-tauri/binaries");
        let lib_dir = temp.path().join("apps/desktop/src-tauri/lib");
        std::fs::create_dir_all(&binaries_dir).expect("binaries dir");
        std::fs::create_dir_all(&lib_dir).expect("lib dir");
        let cli = binaries_dir.join("parakeet-cli-aarch64-apple-darwin");
        let lib = lib_dir.join("libparakeet.dylib");
        std::fs::write(&cli, b"cli").expect("cli");
        std::fs::write(&lib, b"lib").expect("lib");

        assert_eq!(
            resolve_parakeet_live_library_path(&cli),
            lib.canonicalize().expect("canonical lib")
        );
    }

    #[test]
    fn parakeet_live_library_resolution_supports_direct_sibling_layout() {
        let temp = tempfile::tempdir().expect("temp dir");
        let cli_dir = temp.path().join("dev-runtime");
        std::fs::create_dir_all(&cli_dir).expect("cli dir");
        let cli = cli_dir.join("parakeet-cli");
        let lib = cli_dir.join("libparakeet.dylib");
        std::fs::write(&cli, b"cli").expect("cli");
        std::fs::write(&lib, b"lib").expect("lib");

        assert_eq!(
            resolve_parakeet_live_library_path(&cli),
            lib.canonicalize().expect("canonical lib")
        );
    }

    #[test]
    fn parakeet_live_library_resolution_returns_expected_missing_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("runtime/bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let cli = bin_dir.join("parakeet-cli");
        std::fs::write(&cli, b"cli").expect("cli");

        let resolved = resolve_parakeet_live_library_path(&cli);

        assert!(
            resolved.ends_with("lib/libparakeet.dylib"),
            "{}",
            resolved.display()
        );
        assert!(!resolved.exists());
    }
}
