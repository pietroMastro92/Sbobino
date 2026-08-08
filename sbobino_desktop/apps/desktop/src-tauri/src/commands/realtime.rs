use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use sbobino_application::{ApplicationError, RealtimeDelta};
use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, JobProgress, JobStage, LanguageCode, ParakeetModel,
    SpeechModel, TimedSegment, TranscriptArtifact, TranscriptionEngine, TranscriptionOutput,
};

use crate::commands::transcription::{JobFailedEvent, JobProgressEvent};
use crate::parakeet_realtime::ParakeetRealtimeEngine;
use crate::realtime_audio::{emit_level_event, start_input_preview, RealtimeInputLevelEvent};
use crate::{error::CommandError, state::AppState};

const REALTIME_INPUT_PATH: &str = "realtime://microphone";
const REALTIME_SOURCE_LABEL: &str = "Live microphone";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParakeetLiveLibraryPlatform {
    MacOs,
    Windows,
}

impl ParakeetLiveLibraryPlatform {
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::MacOs
        }
    }

    fn library_filename(self) -> &'static str {
        match self {
            Self::MacOs => "libparakeet.dylib",
            Self::Windows => "parakeet.dll",
        }
    }
}

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
    resolve_parakeet_live_library_path_for_platform(
        parakeet_cli_path,
        ParakeetLiveLibraryPlatform::current(),
    )
}

fn resolve_parakeet_live_library_path_for_platform(
    parakeet_cli_path: impl AsRef<Path>,
    platform: ParakeetLiveLibraryPlatform,
) -> PathBuf {
    let cli_path = parakeet_cli_path.as_ref();
    let Some(bin_dir) = cli_path.parent() else {
        return PathBuf::from(platform.library_filename());
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    match platform {
        ParakeetLiveLibraryPlatform::Windows => {
            // Windows packages the DLL both beside the CLI and under runtime/lib.
            candidates.push(bin_dir.join("parakeet.dll"));
            if bin_dir.file_name().and_then(|name| name.to_str()) == Some("bin") {
                candidates.push(bin_dir.join("../lib/parakeet.dll"));
                if let Some(parent) = bin_dir.parent() {
                    candidates.push(parent.join("lib/parakeet.dll"));
                    candidates.push(parent.join("parakeet.dll"));
                    candidates.push(parent.join("../lib/parakeet.dll"));
                }
            } else if let Some(parent) = bin_dir.parent() {
                candidates.push(parent.join("lib/parakeet.dll"));
                candidates.push(parent.join("parakeet.dll"));
            }
        }
        ParakeetLiveLibraryPlatform::MacOs => {
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
        }
    }
    if let Some(parent) = cli_path.parent() {
        candidates.push(parent.join(platform.library_filename()));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("lib").join(platform.library_filename()));
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
        .unwrap_or_else(|| bin_dir.join(platform.library_filename()))
}

pub(crate) fn parakeet_live_target_lang(language: LanguageCode) -> &'static str {
    let _preferred_language = language;
    "auto"
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
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartRealtimeResponse {
    pub started: bool,
    pub job_id: String,
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
    pub queued: bool,
    pub job_id: Option<String>,
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

#[derive(Debug, Clone)]
struct RealtimeArtifactInput {
    job_id: String,
    session_title: String,
    language_code: String,
    model_filename: String,
    processing_engine: String,
    elapsed_seconds: Option<u64>,
    transcript: String,
    segments: Vec<TimedSegment>,
    saved_audio_path: Option<PathBuf>,
}

fn clean_optional_title(title: Option<String>) -> Option<String> {
    title
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn fallback_live_title() -> String {
    format!("live_{}", Utc::now().format("%d%m%Y_%H%M%S"))
}

fn realtime_job_progress(
    job_id: &str,
    stage: JobStage,
    message: impl Into<String>,
    percentage: u8,
    current_seconds: Option<f32>,
    total_seconds: Option<f32>,
) -> JobProgress {
    JobProgress {
        job_id: job_id.to_string(),
        stage,
        message: message.into(),
        percentage,
        current_seconds,
        total_seconds,
    }
}

#[allow(clippy::too_many_arguments)]
fn realtime_progress_event(
    progress: JobProgress,
    title: Option<String>,
    model: SpeechModel,
    language: LanguageCode,
) -> JobProgressEvent {
    JobProgressEvent {
        progress,
        input_path: REALTIME_INPUT_PATH.to_string(),
        title,
        source_origin: ArtifactSourceOrigin::Realtime,
        source_label: Some(REALTIME_SOURCE_LABEL.to_string()),
        source_folder: None,
        model,
        language,
        preset: None,
        workspace_id: None,
    }
}

fn emit_realtime_progress(
    app: &tauri::AppHandle,
    progress: JobProgress,
    title: Option<String>,
    model: SpeechModel,
    language: LanguageCode,
) {
    let _ = app.emit(
        "transcription://progress",
        realtime_progress_event(progress, title, model, language),
    );
}

fn build_realtime_artifact(
    input: RealtimeArtifactInput,
) -> Result<TranscriptArtifact, ApplicationError> {
    let transcription_output = TranscriptionOutput {
        text: input.transcript.clone(),
        segments: input.segments.clone(),
    };
    let processing_language = transcription_output.processing_language_code();
    let mut metadata = BTreeMap::new();
    metadata.insert("kind".to_string(), "realtime".to_string());
    metadata.insert("language".to_string(), processing_language.clone());
    metadata.insert(
        "preferred_language".to_string(),
        input.language_code.clone(),
    );
    metadata.insert("language_detection_version".to_string(), "1".to_string());
    metadata.insert(
        "detected_languages".to_string(),
        transcription_output.detected_languages_json(),
    );
    metadata.insert("model".to_string(), input.model_filename.clone());
    if let Some(elapsed_seconds) = input.elapsed_seconds {
        metadata.insert("duration_seconds".to_string(), elapsed_seconds.to_string());
    }
    metadata.insert(
        "audio_saved".to_string(),
        if input.saved_audio_path.is_some() {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );
    if !input.segments.is_empty() {
        metadata.insert(
            "timeline_v2".to_string(),
            transcription_output.timeline_v2_metadata_json(),
        );
    }

    let source_label = input
        .saved_audio_path
        .as_ref()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{}.wav", input.session_title));

    let mut artifact = TranscriptArtifact::new(
        input.job_id,
        input.session_title,
        ArtifactKind::Realtime,
        source_label,
        ArtifactSourceOrigin::Realtime,
        input.transcript,
        String::new(),
        String::new(),
        String::new(),
        metadata,
    )
    .map_err(|e| ApplicationError::Validation(e.to_string()))?;
    artifact.audio_available = input.saved_audio_path.is_some();
    artifact.audio_duration_seconds = input.elapsed_seconds.map(|value| value as f32);
    artifact.processing_engine = Some(input.processing_engine);
    artifact.processing_model = Some(input.model_filename);
    artifact.processing_language = Some(processing_language);
    if let Some(path) = input.saved_audio_path.as_ref() {
        artifact.set_source_external_path(path.to_string_lossy().to_string());
    }

    Ok(artifact)
}

#[cfg(test)]
mod artifact_tests {
    use super::*;

    #[test]
    fn build_realtime_artifact_preserves_live_job_metadata() {
        let artifact = build_realtime_artifact(RealtimeArtifactInput {
            job_id: "live-job-1".to_string(),
            session_title: "Daily standup".to_string(),
            language_code: "it".to_string(),
            model_filename: "ggml-large-v3-turbo.bin".to_string(),
            processing_engine: "whisper_stream".to_string(),
            elapsed_seconds: Some(3725),
            transcript: "Ciao a tutti.".to_string(),
            segments: Vec::new(),
            saved_audio_path: Some(PathBuf::from("/tmp/sbobino-live/session.wav")),
        })
        .expect("realtime artifact should be valid");

        assert_eq!(artifact.job_id, "live-job-1");
        assert_eq!(artifact.title, "Daily standup");
        assert_eq!(artifact.kind, ArtifactKind::Realtime);
        assert_eq!(artifact.source_origin, ArtifactSourceOrigin::Realtime);
        assert_eq!(artifact.source_label, "session.wav");
        assert_eq!(artifact.raw_transcript, "Ciao a tutti.");
        assert_eq!(
            artifact.processing_engine.as_deref(),
            Some("whisper_stream")
        );
        assert_eq!(
            artifact.processing_model.as_deref(),
            Some("ggml-large-v3-turbo.bin")
        );
        assert_eq!(artifact.processing_language.as_deref(), Some("und"));
        assert_eq!(
            artifact
                .metadata
                .get("preferred_language")
                .map(String::as_str),
            Some("it")
        );
        assert_eq!(
            artifact.metadata.get("language").map(String::as_str),
            Some("und")
        );
        assert_eq!(artifact.audio_duration_seconds, Some(3725.0));
        assert_eq!(
            artifact
                .metadata
                .get("duration_seconds")
                .map(String::as_str),
            Some("3725")
        );
        assert_eq!(
            artifact.metadata.get("audio_saved").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn build_realtime_artifact_rejects_empty_live_transcript() {
        let error = build_realtime_artifact(RealtimeArtifactInput {
            job_id: "live-job-2".to_string(),
            session_title: "Empty live".to_string(),
            language_code: "en".to_string(),
            model_filename: "ggml-base.bin".to_string(),
            processing_engine: "whisper_stream".to_string(),
            elapsed_seconds: None,
            transcript: "  ".to_string(),
            segments: Vec::new(),
            saved_audio_path: None,
        })
        .expect_err("empty live transcript should not produce an artifact");

        assert!(error.to_string().contains("transcript"));
    }

    #[test]
    fn build_realtime_artifact_persists_parakeet_timeline_metadata() {
        let artifact = build_realtime_artifact(RealtimeArtifactInput {
            job_id: "live-job-3".to_string(),
            session_title: "Parakeet live".to_string(),
            language_code: "it".to_string(),
            model_filename: "nemotron-3.5-asr-streaming-0.6b-q8.gguf".to_string(),
            processing_engine: "parakeet_cpp".to_string(),
            elapsed_seconds: Some(42),
            transcript: "Segmento uno.".to_string(),
            segments: vec![TimedSegment {
                text: "Segmento uno.".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(2.5),
                speaker_id: None,
                speaker_label: None,
                language_code: None,
                language_confidence: None,
                words: Vec::new(),
            }],
            saved_audio_path: None,
        })
        .expect("parakeet realtime artifact should be valid");

        assert_eq!(artifact.processing_engine.as_deref(), Some("parakeet_cpp"));
        assert!(
            artifact.metadata.contains_key("timeline_v2"),
            "{:?}",
            artifact.metadata
        );
    }
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

async fn clear_active_realtime_metadata(state: &AppState) {
    *state.realtime.active_job_id.lock().await = None;
    *state.realtime.session_name.lock().await = None;
    *state.realtime.model_filename.lock().await = None;
    *state.realtime.model.lock().await = None;
    *state.realtime.language.lock().await = None;
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
        title: None,
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
    let job_id = Uuid::new_v4().to_string();
    let requested_title = clean_optional_title(payload.title.clone());
    let mut session_title = requested_title.clone().unwrap_or_else(fallback_live_title);
    let mut resume_transcript: Option<String> = None;

    if let Some(id) = &payload.resume_artifact_id {
        let artifact = state
            .artifact_service
            .get(id)
            .await
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::new("not_found", "realtime session not found"))?;

        session_title = requested_title.unwrap_or_else(|| artifact.title.clone());
        resume_transcript = Some(artifact.raw_transcript);
    }

    *state.realtime.active_job_id.lock().await = Some(job_id.clone());
    *state.realtime.session_name.lock().await = Some(session_title.clone());
    // Persist the user's preference separately from the engine's runtime flag.
    // Both live engines are started in automatic detection mode.
    *state.realtime.language_code.lock().await = language.as_code().to_string();
    *state.realtime.active_engine.lock().await = engine_kind.clone();
    *state.realtime.model.lock().await = Some(model.clone());
    *state.realtime.language.lock().await = Some(language.clone());

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
    match engine_kind.clone() {
        TranscriptionEngine::WhisperCpp => {
            let engine = match resolve_realtime_engine(&state) {
                Ok(engine) => engine,
                Err(error) => {
                    clear_active_realtime_metadata(&state).await;
                    return Err(error);
                }
            };
            {
                let mut current_engine = state.realtime.engine.lock().await;
                *current_engine = engine.clone();
            }
            *state.realtime.parakeet_engine.lock().await = None;
            if let Some(transcript) = resume_transcript.as_deref() {
                engine.seed_buffer(transcript).await;
            } else {
                engine.reset().await;
            }
            *state.realtime.model_filename.lock().await = Some(model.ggml_filename().to_string());

            if let Err(error) = start_realtime_preview(&app, &state).await {
                clear_active_realtime_metadata(&state).await;
                return Err(error);
            }

            if let Err(error) = engine
                .start(
                    model.ggml_filename(),
                    language.as_whisper_code(),
                    emit_delta,
                )
                .await
            {
                clear_active_realtime_metadata(&state).await;
                stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
                return Err(CommandError::from(error));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            eprintln!("[realtime-start] parakeet branch entered");
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
            let parakeet_engine = match resolve_parakeet_live_engine(&state) {
                Ok(engine) => engine,
                Err(error) => {
                    clear_active_realtime_metadata(&state).await;
                    return Err(error);
                }
            };
            let models_dir = state
                .runtime_factory
                .resolve_models_dir(&settings.transcription.parakeet_models_dir);
            let requested_model = payload
                .parakeet_model
                .unwrap_or(settings.transcription.parakeet_model);
            let live_model = match select_parakeet_live_model(
                std::path::Path::new(&models_dir),
                requested_model.clone(),
                language.clone(),
            ) {
                Ok(model) => model,
                Err(error) => {
                    clear_active_realtime_metadata(&state).await;
                    return Err(error);
                }
            };
            eprintln!(
                "[realtime-start] parakeet live model requested={requested_model:?} selected={live_model:?} models_dir={models_dir}"
            );
            running_message = "Live listening".to_string();
            if let Some(transcript) = resume_transcript.as_deref() {
                parakeet_engine.seed_buffer(transcript).await;
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
                clear_active_realtime_metadata(&state).await;
                emit_level_event(&app, "idle", 0.0, error.to_string());
                return Err(CommandError::from(error));
            }
            eprintln!("[realtime-start] parakeet engine start returned ok");
        }
    }

    sleep(Duration::from_millis(350)).await;
    let running = match engine_kind.clone() {
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
        clear_active_realtime_metadata(&state).await;
        match engine_kind.clone() {
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

    emit_realtime_progress(
        &app,
        realtime_job_progress(
            &job_id,
            JobStage::Transcribing,
            "Live listening",
            0,
            Some(0.0),
            None,
        ),
        Some(session_title),
        model,
        language,
    );

    Ok(StartRealtimeResponse {
        started: true,
        job_id,
    })
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

enum RealtimeEngineHandle {
    Whisper(sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine),
    Parakeet(ParakeetRealtimeEngine),
}

impl RealtimeEngineHandle {
    async fn stop(self) -> Result<RealtimeStopResult, ApplicationError> {
        match self {
            Self::Whisper(engine) => {
                let result = engine.stop().await?;
                Ok(RealtimeStopResult {
                    transcript: result.transcript,
                    segments: result.segments,
                    saved_audio_path: result.saved_audio_path,
                })
            }
            Self::Parakeet(engine) => {
                let result = engine.stop().await?;
                Ok(RealtimeStopResult {
                    transcript: result.transcript,
                    segments: result.segments,
                    saved_audio_path: result.saved_audio_path,
                })
            }
        }
    }
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

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;
    let active_engine = state.realtime.active_engine.lock().await.clone();
    let engine = match active_engine.clone() {
        TranscriptionEngine::WhisperCpp => {
            RealtimeEngineHandle::Whisper(state.realtime.engine.lock().await.clone())
        }
        TranscriptionEngine::ParakeetCpp => {
            let engine = state
                .realtime
                .parakeet_engine
                .lock()
                .await
                .clone()
                .ok_or_else(|| CommandError::new("realtime", "Parakeet live is not running"))?;
            RealtimeEngineHandle::Parakeet(engine)
        }
    };
    let processing_engine = match active_engine.clone() {
        TranscriptionEngine::WhisperCpp => "whisper_stream".to_string(),
        TranscriptionEngine::ParakeetCpp => "parakeet_cpp".to_string(),
    };
    let job_id = state
        .realtime
        .active_job_id
        .lock()
        .await
        .take()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing_session_title = state.realtime.session_name.lock().await.take();
    let session_title = clean_optional_title(payload.title.clone())
        .or(existing_session_title)
        .unwrap_or_else(fallback_live_title);
    let language = state
        .realtime
        .language
        .lock()
        .await
        .clone()
        .unwrap_or(settings.transcription.language);
    let model = state
        .realtime
        .model
        .lock()
        .await
        .clone()
        .unwrap_or(settings.transcription.model);
    let language_code = state.realtime.language_code.lock().await.clone();
    let model_filename = state
        .realtime
        .model_filename
        .lock()
        .await
        .take()
        .unwrap_or_else(|| model.ggml_filename().to_string());
    *state.realtime.model.lock().await = None;
    *state.realtime.language.lock().await = None;

    match active_engine {
        TranscriptionEngine::WhisperCpp => {
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
        }
        TranscriptionEngine::ParakeetCpp => {
            *state.realtime.parakeet_engine.lock().await = None;
            emit_level_event(&app, "idle", 0.0, "Microphone preview stopped.");
        }
    }

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "stopped".to_string(),
            message: "Live stopped".to_string(),
        },
    );

    emit_realtime_progress(
        &app,
        realtime_job_progress(
            &job_id,
            if save {
                JobStage::Persisting
            } else {
                JobStage::Cancelled
            },
            if save {
                "Saving live transcription"
            } else {
                "Live transcription discarded"
            },
            if save { 90 } else { 100 },
            payload.elapsed_seconds.map(|value| value as f32),
            None,
        ),
        Some(session_title.clone()),
        model.clone(),
        language.clone(),
    );

    let app_handle = app.clone();
    let artifact_service = state.artifact_service.clone();
    let elapsed_seconds = payload.elapsed_seconds;
    let job_id_for_task = job_id.clone();
    tauri::async_runtime::spawn(async move {
        let stop_result = match engine.stop().await {
            Ok(result) => result,
            Err(error) => {
                let _ = app_handle.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: job_id_for_task.clone(),
                        message: error.to_string(),
                    },
                );
                return;
            }
        };

        if !save {
            emit_realtime_progress(
                &app_handle,
                realtime_job_progress(
                    &job_id_for_task,
                    JobStage::Cancelled,
                    "Live transcription discarded",
                    100,
                    elapsed_seconds.map(|value| value as f32),
                    None,
                ),
                Some(session_title),
                model,
                language,
            );
            return;
        }

        if stop_result.transcript.trim().is_empty() {
            emit_realtime_progress(
                &app_handle,
                realtime_job_progress(
                    &job_id_for_task,
                    JobStage::Cancelled,
                    "Live transcription stopped with no captured speech",
                    100,
                    elapsed_seconds.map(|value| value as f32),
                    None,
                ),
                Some(session_title),
                model,
                language,
            );
            return;
        }

        let artifact = match build_realtime_artifact(RealtimeArtifactInput {
            job_id: job_id_for_task.clone(),
            session_title: session_title.clone(),
            language_code,
            model_filename,
            processing_engine,
            elapsed_seconds,
            transcript: stop_result.transcript,
            segments: stop_result.segments,
            saved_audio_path: stop_result.saved_audio_path,
        }) {
            Ok(artifact) => artifact,
            Err(error) => {
                let _ = app_handle.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: job_id_for_task.clone(),
                        message: error.to_string(),
                    },
                );
                return;
            }
        };

        match artifact_service.save(&artifact).await {
            Ok(()) => {
                emit_realtime_progress(
                    &app_handle,
                    realtime_job_progress(
                        &artifact.job_id,
                        JobStage::Completed,
                        "Live transcription saved",
                        100,
                        None,
                        None,
                    ),
                    Some(artifact.title.clone()),
                    model,
                    language,
                );
                let _ = app_handle.emit("transcription://completed", artifact.clone());
                let _ = app_handle.emit("realtime://saved", artifact);
            }
            Err(error) => {
                let _ = app_handle.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: job_id_for_task,
                        message: error.to_string(),
                    },
                );
            }
        }
    });

    Ok(StopRealtimeResponse {
        saved: false,
        queued: true,
        job_id: Some(job_id),
        artifact: None,
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
            resolve_parakeet_live_library_path_for_platform(
                &cli,
                ParakeetLiveLibraryPlatform::MacOs,
            ),
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
            resolve_parakeet_live_library_path_for_platform(
                &cli,
                ParakeetLiveLibraryPlatform::MacOs,
            ),
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
            resolve_parakeet_live_library_path_for_platform(
                &cli,
                ParakeetLiveLibraryPlatform::MacOs,
            ),
            lib.canonicalize().expect("canonical lib")
        );
    }

    #[test]
    fn parakeet_live_library_resolution_supports_windows_packaged_runtime_layout() {
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("runtime/bin");
        let lib_dir = temp.path().join("runtime/lib");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        std::fs::create_dir_all(&lib_dir).expect("lib dir");
        let cli = bin_dir.join("parakeet-cli.exe");
        let bin_lib = bin_dir.join("parakeet.dll");
        std::fs::write(&cli, b"cli").expect("cli");
        std::fs::write(&bin_lib, b"lib").expect("bin lib");

        assert_eq!(
            resolve_parakeet_live_library_path_for_platform(
                &cli,
                ParakeetLiveLibraryPlatform::Windows,
            ),
            bin_lib.canonicalize().expect("canonical bin lib")
        );

        std::fs::remove_file(&bin_lib).expect("remove bin lib");
        let lib = lib_dir.join("parakeet.dll");
        std::fs::write(&lib, b"lib").expect("runtime lib");
        assert_eq!(
            resolve_parakeet_live_library_path_for_platform(
                &cli,
                ParakeetLiveLibraryPlatform::Windows,
            ),
            lib.canonicalize().expect("canonical runtime lib")
        );
    }

    #[test]
    fn parakeet_live_library_resolution_returns_expected_missing_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("runtime/bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        let cli = bin_dir.join("parakeet-cli");
        std::fs::write(&cli, b"cli").expect("cli");

        let resolved = resolve_parakeet_live_library_path_for_platform(
            &cli,
            ParakeetLiveLibraryPlatform::MacOs,
        );

        assert!(
            resolved.ends_with("lib/libparakeet.dylib"),
            "{}",
            resolved.display()
        );
        assert!(!resolved.exists());

        let windows_resolved = resolve_parakeet_live_library_path_for_platform(
            &cli,
            ParakeetLiveLibraryPlatform::Windows,
        );
        assert!(
            windows_resolved.ends_with("parakeet.dll"),
            "{}",
            windows_resolved.display()
        );
        assert!(!windows_resolved.exists());
    }
}
