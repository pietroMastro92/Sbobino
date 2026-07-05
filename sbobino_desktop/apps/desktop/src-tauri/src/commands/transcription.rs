use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use sbobino_application::{
    ApplicationError, AudioTranscoder, DiarizationProgress, RunTranscriptionRequest,
    TranscriptionService,
};
use sbobino_domain::{
    AppSettings, ArtifactSourceOrigin, JobProgress, JobStage, LanguageCode, ParakeetModel,
    SpeechModel, TimedSegment, TimedWord, TranscriptArtifact, TranscriptionEngine,
    TranscriptionOutput, WhisperOptions,
};
use sbobino_infrastructure::adapters::ffmpeg::FfmpegAdapter;

use crate::{
    commands::automatic_import::{
        record_automatic_import_failure, record_automatic_import_success,
        IMPORT_FOLDER_METADATA_KEY, IMPORT_PRESET_METADATA_KEY, IMPORT_SOURCE_LABEL_METADATA_KEY,
        IMPORT_WORKSPACE_METADATA_KEY,
    },
    commands::prepared_transcript::parse_timeline_document,
    error::CommandError,
    state::{AppState, ArtifactPostProcessingTask, TranscriptionTask},
};

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";

#[derive(Debug, Clone, Deserialize)]
pub struct StartTranscriptionPayload {
    pub input_path: String,
    pub engine: TranscriptionEngine,
    pub language: LanguageCode,
    pub model: SpeechModel,
    #[serde(default)]
    pub parakeet_model: ParakeetModel,
    pub enable_ai: bool,
    #[serde(default)]
    pub whisper_options: WhisperOptions,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub source_origin: Option<ArtifactSourceOrigin>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub source_fingerprint_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartTranscriptionResponse {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobProgressEvent {
    #[serde(flatten)]
    pub progress: JobProgress,
    pub input_path: String,
    pub title: Option<String>,
    pub source_origin: ArtifactSourceOrigin,
    pub source_label: Option<String>,
    pub source_folder: Option<String>,
    pub model: SpeechModel,
    pub language: LanguageCode,
    pub preset: Option<String>,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobFailedEvent {
    pub job_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionDeltaEvent {
    pub job_id: String,
    pub text: String,
    pub sequence: u64,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct CancelTranscriptionPayload {
    pub job_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CancelArtifactPostProcessingPayload {
    pub artifact_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactPostProcessingEvent {
    pub artifact_id: String,
    pub kind: String,
    pub status: String,
    pub stage: String,
    pub message: String,
    pub phase: Option<String>,
    pub percentage: Option<u8>,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub artifact: Option<TranscriptArtifact>,
}

#[tauri::command]
pub async fn start_transcription(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: StartTranscriptionPayload,
) -> Result<StartTranscriptionResponse, CommandError> {
    spawn_transcription_job(app, state.inner().clone(), payload).await
}

pub(crate) async fn spawn_transcription_job(
    app: tauri::AppHandle,
    state: AppState,
    payload: StartTranscriptionPayload,
) -> Result<StartTranscriptionResponse, CommandError> {
    let job_id = Uuid::new_v4().to_string();

    let postprocessing_settings = state.runtime_factory.load_settings().ok();
    let diarization_requested = postprocessing_settings
        .as_ref()
        .map(|settings| settings.transcription.speaker_diarization.enabled)
        .unwrap_or(false);

    let mut request = RunTranscriptionRequest {
        job_id: job_id.clone(),
        input_path: payload.input_path,
        engine: payload.engine,
        language: payload.language,
        model: payload.model,
        parakeet_model: payload.parakeet_model,
        enable_ai: payload.enable_ai,
        whisper_options: payload.whisper_options,
        title: payload.title,
        parent_id: payload.parent_id,
        source_origin: payload
            .source_origin
            .unwrap_or(ArtifactSourceOrigin::Imported),
        metadata: payload.metadata,
        source_fingerprint_json: payload.source_fingerprint_json,
    };
    if diarization_requested {
        request
            .metadata
            .entry("speaker_diarization_status".to_string())
            .or_insert_with(|| "queued".to_string());
        request
            .metadata
            .entry("speaker_diarization_progress".to_string())
            .or_insert_with(|| "0".to_string());
        request
            .metadata
            .entry("speaker_diarization_phase".to_string())
            .or_insert_with(|| "queued".to_string());
    }

    let runtime_factory = state.runtime_factory.clone();
    let app_handle = app.clone();
    let delta_app_handle = app.clone();
    let task_job_id = job_id.clone();
    let delta_job_id = job_id.clone();
    let cleanup_job_id = job_id.clone();
    let delta_sequence = Arc::new(AtomicU64::new(0));
    let cancellation_token = CancellationToken::new();
    let task_cancellation_token = cancellation_token.clone();
    let tasks = state.transcription_tasks.clone();
    let transcription_gate = state.transcription_gate.clone();
    let automatic_import_metadata = request.metadata.clone();
    let automatic_import_state = state.clone();
    let postprocessing_state = state.clone();
    let postprocessing_input_path = request.input_path.clone();
    let progress_input_path = request.input_path.clone();
    let progress_title = request.title.clone();
    let progress_source_origin = request.source_origin.clone();
    let progress_source_label = request
        .metadata
        .get(IMPORT_SOURCE_LABEL_METADATA_KEY)
        .cloned();
    let progress_source_folder = request.metadata.get(IMPORT_FOLDER_METADATA_KEY).cloned();
    let progress_model = request.model.clone();
    let progress_language = request.language.clone();
    let progress_preset = request.metadata.get(IMPORT_PRESET_METADATA_KEY).cloned();
    let progress_workspace_id = request.metadata.get(IMPORT_WORKSPACE_METADATA_KEY).cloned();

    tauri::async_runtime::spawn(async move {
        let progress_input_path = progress_input_path.clone();
        let progress_title = progress_title.clone();
        let progress_source_origin = progress_source_origin.clone();
        let progress_source_label = progress_source_label.clone();
        let progress_source_folder = progress_source_folder.clone();
        let progress_model = progress_model.clone();
        let progress_language = progress_language.clone();
        let progress_preset = progress_preset.clone();
        let progress_workspace_id = progress_workspace_id.clone();
        let emit_progress = Arc::new(move |progress: JobProgress| {
            let _ = app_handle.emit(
                "transcription://progress",
                JobProgressEvent {
                    progress,
                    input_path: progress_input_path.clone(),
                    title: progress_title.clone(),
                    source_origin: progress_source_origin.clone(),
                    source_label: progress_source_label.clone(),
                    source_folder: progress_source_folder.clone(),
                    model: progress_model.clone(),
                    language: progress_language.clone(),
                    preset: progress_preset.clone(),
                    workspace_id: progress_workspace_id.clone(),
                },
            );
        });

        // Emit an immediate PreparingAudio event so the job appears in the
        // transcription queue without waiting for the runtime gate or the
        // service's own first emit (which only fires after build_service() and
        // the transcode setup). This covers every entry point — manual start,
        // queued promotion and automatic import — so non-WAV inputs are no
        // longer invisible until their ffmpeg conversion completes.
        emit_progress(JobProgress {
            job_id: task_job_id.clone(),
            stage: JobStage::PreparingAudio,
            message: "Preparing transcription".to_string(),
            percentage: 0,
            current_seconds: None,
            total_seconds: None,
            phase: None,
            progress_kind: sbobino_domain::ProgressKind::Actual,
            attempt: 1,
            effective_model: None,
        });
        let delta_sequence = delta_sequence.clone();
        let emit_delta = Arc::new(move |text: String| {
            let (mode, normalized_text) =
                if let Some(snapshot) = text.strip_prefix(DELTA_REPLACE_PREFIX) {
                    ("replace".to_string(), snapshot.to_string())
                } else {
                    ("append".to_string(), text)
                };
            let sequence = delta_sequence.fetch_add(1, Ordering::Relaxed);
            let _ = delta_app_handle.emit(
                "transcription://delta",
                TranscriptionDeltaEvent {
                    job_id: delta_job_id.clone(),
                    text: normalized_text,
                    sequence,
                    mode,
                },
            );
        });

        // Serialize heavy work. If another job is already running, emit a
        // Queued progress event so the UI can show "Waiting" and the user
        // can still cancel. Dropping `_permit` at end of this block releases
        // the gate for the next queued job.
        let _permit = match transcription_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                emit_progress(JobProgress {
                    job_id: task_job_id.clone(),
                    stage: JobStage::Queued,
                    message: "Waiting for previous transcription to finish".to_string(),
                    percentage: 0,
                    current_seconds: None,
                    total_seconds: None,
                    phase: None,
                    progress_kind: sbobino_domain::ProgressKind::Actual,
                    attempt: 1,
                    effective_model: None,
                });
                tokio::select! {
                    biased;
                    _ = task_cancellation_token.cancelled() => {
                        let _ = app.emit(
                            "transcription://failed",
                            JobFailedEvent {
                                job_id: task_job_id.clone(),
                                message: "Transcription cancelled".to_string(),
                            },
                        );
                        let mut registry = tasks.lock().await;
                        registry.remove(&cleanup_job_id);
                        return;
                    }
                    permit = transcription_gate.clone().acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => {
                            let _ = app.emit(
                                "transcription://failed",
                                JobFailedEvent {
                                    job_id: task_job_id.clone(),
                                    message: "Transcription gate closed unexpectedly".to_string(),
                                },
                            );
                            let mut registry = tasks.lock().await;
                            registry.remove(&cleanup_job_id);
                            return;
                        }
                    },
                }
            }
        };

        if task_cancellation_token.is_cancelled() {
            let _ = app.emit(
                "transcription://failed",
                JobFailedEvent {
                    job_id: task_job_id.clone(),
                    message: "Transcription cancelled".to_string(),
                },
            );
            let mut registry = tasks.lock().await;
            registry.remove(&cleanup_job_id);
            return;
        }

        let transcription_service = match runtime_factory.build_service() {
            Ok(service) => service,
            Err(error) => {
                let _ = app.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: task_job_id.clone(),
                        message: format!("Transcription runtime unavailable: {error}"),
                    },
                );
                let mut registry = tasks.lock().await;
                registry.remove(&cleanup_job_id);
                return;
            }
        };

        match transcription_service
            .run_file_transcription(request, emit_progress, emit_delta, task_cancellation_token)
            .await
        {
            Ok(artifact) => {
                let _ = record_automatic_import_success(
                    &automatic_import_state,
                    &automatic_import_metadata,
                )
                .await;
                let postprocess_artifact = artifact.clone();
                let _ = app.emit("transcription://completed", artifact);
                if diarization_requested {
                    if let Some(settings) = postprocessing_settings.clone() {
                        tauri::async_runtime::spawn(run_file_diarization_background(
                            app.clone(),
                            postprocessing_state.clone(),
                            settings,
                            postprocess_artifact,
                            PathBuf::from(postprocessing_input_path.clone()),
                        ));
                    }
                }
            }
            Err(ApplicationError::Cancelled) => {
                let _ = app.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: task_job_id.clone(),
                        message: "Transcription cancelled".to_string(),
                    },
                );
            }
            Err(error) => {
                let _ = record_automatic_import_failure(
                    &automatic_import_state,
                    &automatic_import_metadata,
                    &error.to_string(),
                )
                .await;
                let _ = app.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: task_job_id,
                        message: error.to_string(),
                    },
                );
            }
        }

        let mut registry = tasks.lock().await;
        registry.remove(&cleanup_job_id);
    });

    let mut registry = state.transcription_tasks.lock().await;
    registry.insert(
        job_id.clone(),
        TranscriptionTask {
            cancel_token: cancellation_token,
        },
    );

    Ok(StartTranscriptionResponse { job_id })
}

#[tauri::command]
pub async fn cancel_transcription(
    state: State<'_, AppState>,
    payload: CancelTranscriptionPayload,
) -> Result<(), CommandError> {
    let task = {
        let mut registry = state.transcription_tasks.lock().await;
        registry.remove(&payload.job_id)
    };

    if let Some(task) = task {
        task.cancel_token.cancel();
    }

    Ok(())
}

#[tauri::command]
pub async fn cancel_artifact_postprocessing(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: CancelArtifactPostProcessingPayload,
) -> Result<(), CommandError> {
    let task = {
        let mut registry = state.postprocessing_tasks.lock().await;
        registry.remove(&payload.artifact_id)
    };

    if let Some(task) = task {
        task.cancel_token.cancel();
    }

    let updated = state
        .artifact_service
        .update_diarization_result(&payload.artifact_id, None, "interrupted", None)
        .await
        .map_err(|error| CommandError::new("artifact_postprocess", error.to_string()))?;
    emit_artifact_postprocess(
        &app,
        &payload.artifact_id,
        "diarization",
        "interrupted",
        "interrupted",
        "Speaker detection stopped.",
        None,
        Some(0),
        updated,
    );

    Ok(())
}

fn emit_artifact_postprocess(
    app: &tauri::AppHandle,
    artifact_id: &str,
    kind: &str,
    status: &str,
    stage: &str,
    message: &str,
    progress: Option<DiarizationProgress>,
    percentage_override: Option<u8>,
    artifact: Option<TranscriptArtifact>,
) {
    let (phase, percentage, completed, total) = if let Some(progress) = progress {
        (
            Some(progress.phase),
            Some(progress.percentage),
            progress.completed,
            progress.total,
        )
    } else {
        (
            Some(stage.to_string()),
            percentage_override,
            None::<u64>,
            None::<u64>,
        )
    };
    let _ = app.emit(
        "artifact://postprocess",
        ArtifactPostProcessingEvent {
            artifact_id: artifact_id.to_string(),
            kind: kind.to_string(),
            status: status.to_string(),
            stage: stage.to_string(),
            message: message.to_string(),
            phase,
            percentage,
            completed,
            total,
            artifact,
        },
    );
}

async fn run_file_diarization_background(
    app: tauri::AppHandle,
    state: AppState,
    settings: AppSettings,
    artifact: TranscriptArtifact,
    input_path: PathBuf,
) {
    let artifact_id = artifact.id.clone();
    let cancel_token = CancellationToken::new();
    {
        let mut registry = state.postprocessing_tasks.lock().await;
        registry.insert(
            artifact_id.clone(),
            ArtifactPostProcessingTask {
                cancel_token: cancel_token.clone(),
            },
        );
    }

    let result = run_file_diarization_background_inner(
        &app,
        &state,
        &settings,
        artifact,
        &input_path,
        &cancel_token,
    )
    .await;

    {
        let mut registry = state.postprocessing_tasks.lock().await;
        registry.remove(&artifact_id);
    }

    if let Err(error) = result {
        let (status, stage, message, error_message) = match error {
            ApplicationError::Cancelled => (
                "interrupted",
                "interrupted",
                "Speaker detection stopped.",
                None,
            ),
            other => (
                "failed",
                "failed",
                "Speaker detection failed; the transcript remains available.",
                Some(other.to_string()),
            ),
        };
        let updated = state
            .artifact_service
            .update_diarization_result(&artifact_id, None, status, error_message.as_deref())
            .await
            .ok()
            .flatten();
        emit_artifact_postprocess(
            &app,
            &artifact_id,
            "diarization",
            status,
            stage,
            message,
            None,
            Some(0),
            updated,
        );
    }
}

async fn run_file_diarization_background_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    settings: &AppSettings,
    artifact: TranscriptArtifact,
    input_path: &Path,
    cancel_token: &CancellationToken,
) -> Result<(), ApplicationError> {
    let artifact_id = artifact.id.clone();
    emit_artifact_postprocess(
        app,
        &artifact_id,
        "diarization",
        "queued",
        "queued",
        "Speaker detection is queued.",
        None,
        Some(0),
        None,
    );

    let permit = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return Err(ApplicationError::Cancelled),
        permit = state.transcription_gate.clone().acquire_owned() => permit.map_err(|_| {
            ApplicationError::SpeakerDiarization("post-processing gate closed unexpectedly".to_string())
        })?,
    };

    if cancel_token.is_cancelled() {
        drop(permit);
        return Err(ApplicationError::Cancelled);
    }

    let _ = state
        .artifact_service
        .update_metadata_entry(&artifact_id, "speaker_diarization_status", Some("running"))
        .await?;
    let _ = state
        .artifact_service
        .update_metadata_entry(&artifact_id, "speaker_diarization_progress", Some("0"))
        .await?;
    let _ = state
        .artifact_service
        .update_metadata_entry(&artifact_id, "speaker_diarization_phase", Some("preparing"))
        .await?;
    emit_artifact_postprocess(
        app,
        &artifact_id,
        "diarization",
        "running",
        "preparing",
        "Preparing speaker detection.",
        None,
        Some(0),
        None,
    );

    let wav_path = std::env::temp_dir().join(format!(
        "sbobino-diarization-{}-{}.wav",
        artifact_id,
        Uuid::new_v4()
    ));
    let ffmpeg_path = state
        .runtime_factory
        .resolve_binary_path(&settings.transcription.ffmpeg_path, "ffmpeg");
    let transcoder = FfmpegAdapter::new(ffmpeg_path);
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            drop(permit);
            return Err(ApplicationError::Cancelled);
        }
        result = transcoder.to_wav_mono_16k(input_path, &wav_path) => {
            result?;
        }
    }

    let segments = segments_from_artifact_timeline(&artifact);
    if segments.is_empty() {
        let updated = state
            .artifact_service
            .update_diarization_result(&artifact_id, None, "completed", None)
            .await?;
        emit_artifact_postprocess(
            app,
            &artifact_id,
            "diarization",
            "completed",
            "completed",
            "Speaker detection completed.",
            None,
            Some(100),
            updated,
        );
        let _ = tokio::fs::remove_file(&wav_path).await;
        drop(permit);
        return Ok(());
    }

    let diarizer = state
        .runtime_factory
        .build_speaker_diarizer(settings)
        .map_err(ApplicationError::SpeakerDiarization)?
        .ok_or_else(|| {
            ApplicationError::SpeakerDiarization(
                "speaker diarization runtime is unavailable".to_string(),
            )
        })?;

    let progress_app = app.clone();
    let progress_artifact_id = artifact_id.clone();
    let progress = Arc::new(move |progress: DiarizationProgress| {
        let message = progress.message.clone();
        emit_artifact_postprocess(
            &progress_app,
            &progress_artifact_id,
            "diarization",
            "running",
            "diarizing",
            &message,
            Some(progress),
            None,
            None,
        );
    });

    let turns = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            let _ = tokio::fs::remove_file(&wav_path).await;
            drop(permit);
            return Err(ApplicationError::Cancelled);
        }
        result = diarizer.diarize(&wav_path, progress) => result?,
    };

    let assigned = if turns.is_empty() {
        segments
    } else {
        TranscriptionService::assign_speakers_to_segments(&segments, &turns)
    };
    let timeline = TranscriptionOutput {
        text: assigned
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        segments: assigned,
        effective_model: None,
    }
    .timeline_v2_metadata_json();

    let updated = state
        .artifact_service
        .update_diarization_result(&artifact_id, Some(&timeline), "completed", None)
        .await?;
    emit_artifact_postprocess(
        app,
        &artifact_id,
        "diarization",
        "completed",
        "completed",
        "Speaker detection completed.",
        None,
        Some(100),
        updated,
    );

    let _ = tokio::fs::remove_file(&wav_path).await;
    drop(permit);
    Ok(())
}

fn segments_from_artifact_timeline(artifact: &TranscriptArtifact) -> Vec<TimedSegment> {
    parse_timeline_document(artifact)
        .map(|document| {
            document
                .segments
                .into_iter()
                .map(|segment| TimedSegment {
                    text: segment.text,
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                    speaker_id: segment.speaker_id,
                    speaker_label: segment.speaker_label,
                    words: segment
                        .words
                        .into_iter()
                        .map(|word| TimedWord {
                            text: word.text,
                            start_seconds: word.start_seconds,
                            end_seconds: word.end_seconds,
                            confidence: word.confidence,
                        })
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default()
}
