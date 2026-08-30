use std::{
    collections::BTreeMap,
    future::Future,
    path::Path,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use chrono::Utc;
use serde_json::json;
use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{instrument, warn};

use sbobino_domain::{
    constrain_transcript_edit, minimize_transcript_repetitions, repair_segments,
    speaker_quality_report, ArtifactKind, JobProgress, JobStage, LanguageCode, SpeakerTurn,
    TimedSegment, TranscriptArtifact, TranscriptionEngine, TranscriptionOutput,
    SEGMENT_REPAIR_METADATA_KEY, SPEAKER_QUALITY_METADATA_KEY,
};

use crate::{
    dto::{RunTranscriptionRequest, SummaryFaq},
    is_retryable_ai_provider_error, summarize_and_faq_adaptive, summarize_transcript_adaptive,
    ApplicationError, ArtifactRepository, AudioTranscoder, SpeakerDiarizationEngine,
    SpeechToTextEngine, TranscriptEnhancer,
};

const HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY: &str = "has_optimized_transcript";
const STUDY_PACK_METADATA_KEY: &str = "study_pack_v1";
const MEETING_PACK_METADATA_KEY: &str = "meeting_intelligence_v1";
const AUTO_IMPORT_GENERATE_SUMMARY_METADATA_KEY: &str = "auto_import_generate_summary";
const AUTO_IMPORT_GENERATE_FAQS_METADATA_KEY: &str = "auto_import_generate_faqs";
const AUTO_IMPORT_GENERATE_PRESET_OUTPUT_METADATA_KEY: &str = "auto_import_generate_preset_output";
const AUTO_POST_SUMMARY_STATUS_METADATA_KEY: &str = "auto_post_summary_status";
const AUTO_POST_FAQS_STATUS_METADATA_KEY: &str = "auto_post_faqs_status";
const AUTO_POST_PRESET_OUTPUT_STATUS_METADATA_KEY: &str = "auto_post_preset_output_status";

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceLanguageOptimizationGroup {
    language_code: String,
    text: String,
}

/// Per-file progress state.  Engines may internally retry an unconfirmed
/// interval (for example after a Metal allocation failure), so the visible
/// job must own the monotonic counters rather than trusting every callback's
/// local timestamp.
#[derive(Debug, Clone, Copy, Default)]
struct ProgressSnapshot {
    committed_seconds: f32,
    processed_seconds: f32,
    stage_percentage: u8,
    overall_percentage: u8,
}

impl ProgressSnapshot {
    fn update(
        &mut self,
        stage: &JobStage,
        desired_overall: u8,
        desired_stage: u8,
        current_seconds: Option<f32>,
        terminal: bool,
    ) {
        if let Some(seconds) = current_seconds.filter(|value| value.is_finite()) {
            let seconds = seconds.max(0.0);
            // Both values are intentionally max-based: a retry can report a
            // local timestamp near zero, but that is not a user-visible reset.
            self.processed_seconds = self.processed_seconds.max(seconds);
            self.committed_seconds = self.committed_seconds.max(seconds);
        }
        if !terminal {
            self.stage_percentage = desired_stage.min(100);
        }
        let desired_overall = desired_overall.min(100);
        self.overall_percentage = if terminal {
            // A failed/cancelled job leaves the last confirmed visible value
            // untouched. In particular, it must not jump to 99/100 merely
            // because the terminal caller uses its legacy `percentage: 100`.
            self.overall_percentage
        } else if matches!(stage, JobStage::Completed) {
            100
        } else {
            self.overall_percentage.max(desired_overall.min(99))
        };
    }
}

#[derive(Clone)]
pub struct TranscriptionService {
    transcoder: Arc<dyn AudioTranscoder>,
    speech_engine: Arc<dyn SpeechToTextEngine>,
    speaker_diarizer: Option<Arc<dyn SpeakerDiarizationEngine>>,
    enhancer: Arc<dyn TranscriptEnhancer>,
    fallback_enhancers: Vec<Arc<dyn TranscriptEnhancer>>,
    artifacts: Arc<dyn ArtifactRepository>,
}

impl TranscriptionService {
    pub fn new(
        transcoder: Arc<dyn AudioTranscoder>,
        speech_engine: Arc<dyn SpeechToTextEngine>,
        enhancer: Arc<dyn TranscriptEnhancer>,
        artifacts: Arc<dyn ArtifactRepository>,
    ) -> Self {
        Self {
            transcoder,
            speech_engine,
            speaker_diarizer: None,
            enhancer,
            fallback_enhancers: Vec::new(),
            artifacts,
        }
    }

    pub fn with_speaker_diarizer(
        mut self,
        speaker_diarizer: Arc<dyn SpeakerDiarizationEngine>,
    ) -> Self {
        self.speaker_diarizer = Some(speaker_diarizer);
        self
    }

    pub fn with_fallback_enhancers(
        mut self,
        fallback_enhancers: Vec<Arc<dyn TranscriptEnhancer>>,
    ) -> Self {
        self.fallback_enhancers = fallback_enhancers;
        self
    }

    #[instrument(skip(self, emit_progress, emit_delta), fields(job_id = %request.job_id))]
    pub async fn run_file_transcription(
        &self,
        request: RunTranscriptionRequest,
        emit_progress: Arc<dyn Fn(JobProgress) + Send + Sync>,
        emit_delta: Arc<dyn Fn(String) + Send + Sync>,
        cancellation_token: CancellationToken,
    ) -> Result<TranscriptArtifact, ApplicationError> {
        if request.input_path.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "input path cannot be empty".to_string(),
            ));
        }
        if cancellation_token.is_cancelled() {
            return Err(ApplicationError::Cancelled);
        }

        let input_path = PathBuf::from(&request.input_path);
        if !fs::try_exists(&input_path).await.map_err(|e| {
            ApplicationError::Validation(format!("failed to validate input path: {e}"))
        })? {
            return Err(ApplicationError::Validation(format!(
                "input file not found: {}",
                request.input_path
            )));
        }

        let progress_state = Arc::new(Mutex::new(ProgressSnapshot::default()));
        self.emit(
            &emit_progress,
            &progress_state,
            &request.job_id,
            JobStage::PreparingAudio,
            "Preparing audio",
            0,
            None,
            None,
            Some(0),
        );
        let job_id = request.job_id.clone();

        let wav_path = self.normalized_wav_path(&input_path, &request.job_id);
        let result = async {
            // Always transcode through ffmpeg so downstream engines (whisper-cli and
            // the pyannote helper, which uses Python's `wave` module) receive a
            // deterministic PCM-16 mono 16 kHz stream. The skip-ffmpeg fast path
            // shipped in v0.1.36 caused field reports of "transcription does not
            // start"; reverted until we can repro and root-cause.
            self.run_cancellable(
                &cancellation_token,
                self.transcoder.to_wav_mono_16k(&input_path, &wav_path),
            )
            .await?;

            let total_audio_seconds = self.wav_duration_seconds(&wav_path);
            let transcription_progress_message =
                transcription_progress_message(&request.engine).to_string();

            self.emit(
                &emit_progress,
                &progress_state,
                &request.job_id,
                JobStage::PreparingAudio,
                "Audio prepared",
                5,
                None,
                total_audio_seconds,
                Some(100),
            );

            self.emit(
                &emit_progress,
                &progress_state,
                &request.job_id,
                JobStage::Transcribing,
                &transcription_progress_message,
                5,
                Some(0.0),
                total_audio_seconds,
                Some(0),
            );

            let progress_callback = {
                let emit_progress = emit_progress.clone();
                let job_id = request.job_id.clone();
                let transcription_progress_message = transcription_progress_message.clone();
                let progress_state = progress_state.clone();

                Arc::new(move |current_seconds: f32| {
                    let sanitized_seconds = current_seconds.max(0.0);
                    let stage_percentage = match total_audio_seconds {
                        Some(total) if total > 0.0 => {
                            ((sanitized_seconds / total).clamp(0.0, 1.0) * 100.0).round() as u8
                        }
                        _ => 0,
                    };

                    let mut state = match progress_state.lock() {
                        Ok(state) => state,
                        Err(_) => return,
                    };
                    // Ignore callbacks that belong to an internal retry and
                    // point behind the already-confirmed prefix.  This keeps
                    // both visible seconds and overall percentage monotonic.
                    if sanitized_seconds <= state.processed_seconds + 0.05 {
                        return;
                    }
                    let overall = match total_audio_seconds {
                        Some(total) if total > 0.0 => {
                            (5.0 + (sanitized_seconds / total).clamp(0.0, 1.0) * 80.0).round() as u8
                        }
                        _ => 5,
                    };
                    state.update(
                        &JobStage::Transcribing,
                        overall,
                        stage_percentage,
                        Some(sanitized_seconds),
                        false,
                    );

                    emit_progress(JobProgress {
                        job_id: job_id.clone(),
                        stage: JobStage::Transcribing,
                        message: transcription_progress_message.clone(),
                        percentage: state.overall_percentage,
                        current_seconds: Some(sanitized_seconds),
                        total_seconds: total_audio_seconds,
                        committed_seconds: state.committed_seconds,
                        processed_seconds: state.processed_seconds,
                        stage_percentage: state.stage_percentage,
                        overall_percentage: state.overall_percentage,
                    });
                }) as Arc<dyn Fn(f32) + Send + Sync>
            };

            let mut transcription_output = self
                .run_cancellable(
                    &cancellation_token,
                    self.speech_engine.transcribe(
                        &wav_path,
                        request.speech_model_filename(),
                        &request.language_policy(),
                        &request.whisper_options,
                        total_audio_seconds,
                        emit_delta.clone(),
                        progress_callback,
                    ),
                )
                .await?;
            let raw_transcript = minimize_transcript_repetitions(&Self::select_raw_transcript(
                &transcription_output,
            ));
            if raw_transcript.is_empty() {
                return Err(ApplicationError::SpeechToText(
                    "speech-to-text engine produced empty output".to_string(),
                ));
            }

            emit_delta(raw_transcript.clone());

            if let Some(total) = total_audio_seconds {
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Transcribing,
                    &transcription_progress_message,
                    85,
                    Some(total),
                    Some(total),
                    Some(100),
                );
            }

            let mut diarization_status: Option<String> = None;
            let mut diarization_error: Option<String> = None;
            if let Some(speaker_diarizer) = &self.speaker_diarizer {
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Diarizing,
                    "Assigning speakers with pyannote",
                    85,
                    None,
                    None,
                    Some(0),
                );
                match self
                    .run_cancellable(&cancellation_token, speaker_diarizer.diarize(&wav_path))
                    .await
                {
                    Ok(turns) => {
                        diarization_status = Some("completed".to_string());
                        if !turns.is_empty() && !transcription_output.segments.is_empty() {
                            transcription_output.segments = Self::assign_speakers_to_segments(
                                &transcription_output.segments,
                                &turns,
                            );
                        }
                    }
                    Err(ApplicationError::Cancelled) => return Err(ApplicationError::Cancelled),
                    Err(error) => {
                        diarization_status = Some("failed".to_string());
                        diarization_error = Some(error.to_string());
                        warn!("speaker diarization skipped after transcription: {error}");
                    }
                }
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Diarizing,
                    "Speaker assignment complete",
                    93,
                    None,
                    None,
                    Some(100),
                );
            }

            let (optimized, summary_faq, has_optimized_transcript, generated_outputs) = if request
                .enable_ai
            {
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Optimizing,
                    "Optimizing transcript with AI",
                    93,
                    None,
                    None,
                    Some(0),
                );
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Summarizing,
                    "Generating summary and FAQs",
                    96,
                    None,
                    None,
                    Some(0),
                );

                let ai_language = if request.language.is_auto() {
                    transcription_output
                        .dominant_language_code()
                        .unwrap_or_else(|| "en".to_string())
                } else {
                    request.language.as_code().to_string()
                };
                let optimization_groups =
                    Self::language_optimization_groups(&transcription_output, &raw_transcript);
                match self
                    .run_cancellable(
                        &cancellation_token,
                        self.run_ai_post_processing(&optimization_groups, &ai_language, &request),
                    )
                    .await
                {
                    Ok(result) => result,
                    Err(ApplicationError::Cancelled) => return Err(ApplicationError::Cancelled),
                    Err(error) => {
                        warn!("ai optimization skipped; keeping raw transcript: {error}");
                        (
                            String::new(),
                            SummaryFaq {
                                summary: String::new(),
                                faqs: String::new(),
                            },
                            false,
                            BTreeMap::new(),
                        )
                    }
                }
            } else {
                (
                    String::new(),
                    SummaryFaq {
                        summary: String::new(),
                        faqs: String::new(),
                    },
                    false,
                    BTreeMap::new(),
                )
            };

            if request.enable_ai {
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Optimizing,
                    "Transcript optimization complete",
                    96,
                    None,
                    None,
                    Some(100),
                );
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &request.job_id,
                    JobStage::Summarizing,
                    "Summary generation complete",
                    98,
                    None,
                    None,
                    Some(100),
                );
            }

            // Structural cleanup is evidence-bound and runs only on the
            // decoded timeline. It happens after AI grouping so duplicate
            // segments cannot erase language evidence needed by post-processing.
            // The raw transcript remains untouched, and speaker labels are
            // never inferred or rewritten by this pass.
            let (repaired_segments, segment_repair_report) =
                repair_segments(&transcription_output.segments);
            transcription_output.segments = repaired_segments;
            let speaker_quality_report = speaker_quality_report(&transcription_output.segments);

            self.emit(
                &emit_progress,
                &progress_state,
                &request.job_id,
                JobStage::Persisting,
                "Persisting transcription artifact",
                98,
                None,
                None,
                Some(0),
            );

            let mut metadata = request.metadata.clone();
            let processing_language = transcription_output.processing_language_code();
            metadata.insert(
                "model".to_string(),
                request.speech_model_filename().to_string(),
            );
            metadata.insert("language".to_string(), processing_language.clone());
            metadata.insert(
                "preferred_language".to_string(),
                request.language.as_code().to_string(),
            );
            metadata.insert("language_detection_version".to_string(), "1".to_string());
            metadata.insert(
                "detected_languages".to_string(),
                transcription_output.detected_languages_json(),
            );
            metadata.insert(
                "timeline_v2".to_string(),
                transcription_output.timeline_v2_metadata_json(),
            );
            metadata.insert(
                SEGMENT_REPAIR_METADATA_KEY.to_string(),
                serde_json::to_string(&segment_repair_report).unwrap_or_else(|_| {
                    format!(
                        r#"{{"version":"{SEGMENT_REPAIR_METADATA_KEY}","status":"unavailable"}}"#
                    )
                }),
            );
            metadata.insert(
                SPEAKER_QUALITY_METADATA_KEY.to_string(),
                serde_json::to_string(&speaker_quality_report).unwrap_or_else(|_| {
                    format!(
                        r#"{{"version":"{SPEAKER_QUALITY_METADATA_KEY}","status":"unavailable","warning_count":0,"warnings":[]}}"#
                    )
                }),
            );
            if let Some(status) = diarization_status {
                metadata.insert("speaker_diarization_status".to_string(), status);
            }
            if let Some(error) = diarization_error {
                metadata.insert("speaker_diarization_error".to_string(), error);
            }

            if let Some(pid) = &request.parent_id {
                metadata.insert("parent_id".to_string(), pid.clone());
            }
            if has_optimized_transcript {
                metadata.insert(
                    HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY.to_string(),
                    "true".to_string(),
                );
            }
            if !request.enable_ai {
                metadata.insert(
                    AUTO_POST_SUMMARY_STATUS_METADATA_KEY.to_string(),
                    "disabled".to_string(),
                );
                metadata.insert(
                    AUTO_POST_FAQS_STATUS_METADATA_KEY.to_string(),
                    "disabled".to_string(),
                );
                metadata.insert(
                    AUTO_POST_PRESET_OUTPUT_STATUS_METADATA_KEY.to_string(),
                    "disabled".to_string(),
                );
            }
            metadata.extend(generated_outputs);

            let final_title = request.title.clone().unwrap_or_else(|| {
                input_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&request.input_path)
                    .to_string()
            });

            let mut artifact = TranscriptArtifact::new(
                request.job_id.clone(),
                final_title,
                ArtifactKind::File,
                input_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&request.input_path)
                    .to_string(),
                request.source_origin.clone(),
                raw_transcript,
                optimized,
                summary_faq.summary,
                summary_faq.faqs,
                metadata,
            )
            .map_err(|e| ApplicationError::Validation(e.to_string()))?;
            artifact.audio_duration_seconds = total_audio_seconds;
            artifact.parent_artifact_id = request.parent_id.clone();
            artifact.processing_engine = Some(request.engine.as_str().to_string());
            artifact.processing_model = Some(request.speech_model_filename().to_string());
            artifact.processing_language = Some(processing_language);
            artifact.whisper_options_json = serde_json::to_string(&request.whisper_options).ok();
            artifact.ai_provider_snapshot_json = Some(
                serde_json::json!({
                    "enabled": request.enable_ai,
                })
                .to_string(),
            );
            artifact.set_source_external_path(request.input_path.clone());
            artifact.source_fingerprint_json = request.source_fingerprint_json.clone();

            self.run_cancellable(&cancellation_token, self.artifacts.save(&artifact))
                .await?;

            self.emit(
                &emit_progress,
                &progress_state,
                &artifact.job_id,
                JobStage::Completed,
                "Transcription completed",
                100,
                None,
                None,
                Some(100),
            );

            Ok(artifact)
        }
        .await;

        if let Err(error) = fs::remove_file(&wav_path).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    path = %wav_path.display(),
                    "failed to remove temporary wav file: {error}"
                );
            }
        }

        match &result {
            Err(ApplicationError::Cancelled) => {
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &job_id,
                    JobStage::Cancelled,
                    "Transcription cancelled",
                    100,
                    None,
                    None,
                    None,
                );
            }
            Err(error) => {
                self.emit(
                    &emit_progress,
                    &progress_state,
                    &job_id,
                    JobStage::Failed,
                    &format!("Transcription failed: {error}"),
                    100,
                    None,
                    None,
                    None,
                );
            }
            Ok(_) => {}
        }

        result
    }

    async fn run_ai_post_processing(
        &self,
        optimization_groups: &[SourceLanguageOptimizationGroup],
        summary_language_code: &str,
        request: &RunTranscriptionRequest,
    ) -> Result<(String, SummaryFaq, bool, BTreeMap<String, String>), ApplicationError> {
        let safety_source = optimization_groups
            .iter()
            .map(|group| group.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let generate_summary =
            metadata_bool(request, AUTO_IMPORT_GENERATE_SUMMARY_METADATA_KEY, true);
        let generate_faqs = metadata_bool(request, AUTO_IMPORT_GENERATE_FAQS_METADATA_KEY, true);
        let generate_preset_output = metadata_bool(
            request,
            AUTO_IMPORT_GENERATE_PRESET_OUTPUT_METADATA_KEY,
            true,
        );
        let mut last_retryable_error: Option<ApplicationError> = None;

        for enhancer in self.ordered_enhancers() {
            let optimized = match Self::optimize_language_groups(
                enhancer.as_ref(),
                optimization_groups,
            )
            .await
            {
                Ok(value) => value,
                Err(error) if is_retryable_ai_provider_error(&error) => {
                    last_retryable_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };

            let optimized = Self::strip_language_service_markers(&optimized);
            let constrained_optimized = constrain_transcript_edit(&safety_source, &optimized);
            let has_optimized_transcript = constrained_optimized != safety_source;

            let mut summary_faq = if generate_summary || generate_faqs {
                match summarize_and_faq_adaptive(
                    enhancer.as_ref(),
                    &constrained_optimized,
                    summary_language_code,
                )
                .await
                {
                    Ok(value) => value,
                    Err(error) if is_retryable_ai_provider_error(&error) => {
                        last_retryable_error = Some(error);
                        continue;
                    }
                    Err(error) => {
                        warn!("summary/faq generation skipped after optimization: {error}");
                        SummaryFaq {
                            summary: String::new(),
                            faqs: String::new(),
                        }
                    }
                }
            } else {
                SummaryFaq {
                    summary: String::new(),
                    faqs: String::new(),
                }
            };
            summary_faq.summary = Self::strip_language_service_markers(&summary_faq.summary);
            summary_faq.faqs = Self::strip_language_service_markers(&summary_faq.faqs);
            if !generate_summary {
                summary_faq.summary.clear();
            }
            if !generate_faqs {
                summary_faq.faqs.clear();
            }

            let mut generated_outputs = if generate_preset_output {
                match self
                    .generate_preset_outputs(
                        enhancer.as_ref(),
                        &constrained_optimized,
                        summary_language_code,
                        request,
                    )
                    .await
                {
                    Ok(outputs) => outputs,
                    Err(error) => {
                        warn!("preset-specific outputs skipped after summary generation: {error}");
                        BTreeMap::new()
                    }
                }
            } else {
                BTreeMap::new()
            };
            for value in generated_outputs.values_mut() {
                *value = Self::strip_language_service_markers(value);
            }
            generated_outputs.insert(
                AUTO_POST_SUMMARY_STATUS_METADATA_KEY.to_string(),
                if !generate_summary {
                    "skipped"
                } else if summary_faq.summary.trim().is_empty() {
                    "unavailable"
                } else {
                    "generated"
                }
                .to_string(),
            );
            generated_outputs.insert(
                AUTO_POST_FAQS_STATUS_METADATA_KEY.to_string(),
                if !generate_faqs {
                    "skipped"
                } else if summary_faq.faqs.trim().is_empty() {
                    "unavailable"
                } else {
                    "generated"
                }
                .to_string(),
            );
            let has_preset_output = generated_outputs.contains_key(STUDY_PACK_METADATA_KEY)
                || generated_outputs.contains_key(MEETING_PACK_METADATA_KEY);
            generated_outputs.insert(
                AUTO_POST_PRESET_OUTPUT_STATUS_METADATA_KEY.to_string(),
                if !generate_preset_output {
                    "skipped"
                } else if has_preset_output {
                    "generated"
                } else {
                    "unavailable"
                }
                .to_string(),
            );

            return Ok((
                constrained_optimized,
                summary_faq,
                has_optimized_transcript,
                generated_outputs,
            ));
        }

        Err(last_retryable_error.unwrap_or_else(|| {
            ApplicationError::PostProcessing(
                "no AI provider was able to process the transcript".to_string(),
            )
        }))
    }

    async fn generate_preset_outputs(
        &self,
        enhancer: &dyn TranscriptEnhancer,
        transcript: &str,
        language_code: &str,
        request: &RunTranscriptionRequest,
    ) -> Result<BTreeMap<String, String>, ApplicationError> {
        let Some(preset) = request
            .metadata
            .get("auto_import_preset")
            .map(|value| value.trim())
        else {
            return Ok(BTreeMap::new());
        };
        if transcript.trim().is_empty() {
            return Ok(BTreeMap::new());
        }

        let mut outputs = BTreeMap::new();
        match preset {
            "lecture" => {
                let body_markdown = summarize_transcript_adaptive(
                    enhancer,
                    transcript,
                    &Self::build_study_pack_prompt(language_code),
                )
                .await?;
                outputs.insert(
                    STUDY_PACK_METADATA_KEY.to_string(),
                    json!({
                        "kind": "study_pack",
                        "generated_at": Utc::now().to_rfc3339(),
                        "body_markdown": body_markdown,
                    })
                    .to_string(),
                );
            }
            "meeting" | "interview" => {
                let body_markdown = summarize_transcript_adaptive(
                    enhancer,
                    transcript,
                    &Self::build_meeting_pack_prompt(language_code, preset == "interview"),
                )
                .await?;
                outputs.insert(
                    MEETING_PACK_METADATA_KEY.to_string(),
                    json!({
                        "kind": "meeting_intelligence",
                        "generated_at": Utc::now().to_rfc3339(),
                        "body_markdown": body_markdown,
                    })
                    .to_string(),
                );
            }
            _ => {}
        }
        Ok(outputs)
    }

    fn ordered_enhancers(&self) -> Vec<Arc<dyn TranscriptEnhancer>> {
        let mut enhancers = Vec::with_capacity(1 + self.fallback_enhancers.len());
        enhancers.push(self.enhancer.clone());
        enhancers.extend(self.fallback_enhancers.iter().cloned());
        enhancers
    }

    fn build_study_pack_prompt(language_code: &str) -> String {
        format!(
            "Write the entire output in {language_code}. Produce only markdown.\n\n\
             Build a student study pack from the transcript with these sections in order:\n\
             1. Overview\n\
             2. Structured Notes\n\
             3. Glossary of Key Terms\n\
             4. Probable Exam Questions\n\
             5. Flashcards\n\n\
             Requirements:\n\
             - Stay faithful to the transcript and do not invent facts.\n\
             - Use concise headings and bullet points where helpful.\n\
             - In Glossary, define the most important terms in plain language.\n\
             - In Probable Exam Questions, include short model answers.\n\
             - In Flashcards, format each item as `Q:` followed by `A:`.\n\
             - If the transcript does not support a section, write `Not enough evidence.` under that heading."
        )
    }

    fn build_meeting_pack_prompt(language_code: &str, interview_mode: bool) -> String {
        let opening = if interview_mode {
            "Build an interview intelligence pack from the transcript"
        } else {
            "Build a meeting intelligence pack from the transcript"
        };
        format!(
            "{opening}. Write the entire output in {language_code}. Produce only markdown.\n\n\
             Use these sections in order:\n\
             1. Executive Summary\n\
             2. Decisions\n\
             3. Action Items\n\
             4. Open Questions\n\
             5. Risks and Blockers\n\n\
             Requirements:\n\
             - Stay faithful to the transcript and do not invent facts.\n\
             - Where owners or deadlines are explicit, capture them.\n\
             - If an item is uncertain, mark it clearly as tentative.\n\
             - If no evidence exists for a section, write `Not enough evidence.` under that heading."
        )
    }

    pub async fn list_recent_artifacts(
        &self,
        limit: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        self.artifacts.list_recent(limit, 0).await
    }

    pub async fn get_artifact_by_id(
        &self,
        id: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts.get_by_id(id).await
    }

    pub async fn update_artifact_content(
        &self,
        id: &str,
        optimized_transcript: &str,
        summary: &str,
        faqs: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts
            .update_content(id, optimized_transcript, summary, faqs)
            .await
    }

    fn normalized_wav_path(&self, input_path: &Path, job_id: &str) -> PathBuf {
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("audio")
            .to_string();
        std::env::temp_dir().join(format!("{stem}-{job_id}.wav"))
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        callback: &Arc<dyn Fn(JobProgress) + Send + Sync>,
        progress_state: &Arc<Mutex<ProgressSnapshot>>,
        job_id: &str,
        stage: JobStage,
        message: &str,
        percentage: u8,
        current_seconds: Option<f32>,
        total_seconds: Option<f32>,
        stage_percentage_override: Option<u8>,
    ) {
        let stage_percentage = if let Some(override_value) = stage_percentage_override {
            override_value.min(100)
        } else if matches!(stage, JobStage::Completed) {
            100
        } else if matches!(stage, JobStage::Transcribing) {
            match (current_seconds, total_seconds) {
                (Some(current), Some(total)) if total > 0.0 => {
                    ((current / total).clamp(0.0, 1.0) * 100.0).round() as u8
                }
                _ => 0,
            }
        } else {
            0
        };
        let terminal = matches!(stage, JobStage::Failed | JobStage::Cancelled);
        let mut state = progress_state
            .lock()
            .expect("transcription progress state lock poisoned");
        state.update(
            &stage,
            percentage,
            stage_percentage,
            current_seconds,
            terminal,
        );
        callback(JobProgress {
            job_id: job_id.to_string(),
            stage,
            message: message.to_string(),
            percentage: state.overall_percentage,
            current_seconds,
            total_seconds,
            committed_seconds: state.committed_seconds,
            processed_seconds: state.processed_seconds,
            stage_percentage: state.stage_percentage,
            overall_percentage: state.overall_percentage,
        });
    }

    fn select_raw_transcript(transcription_output: &TranscriptionOutput) -> String {
        let direct = transcription_output.text.trim();
        if !direct.is_empty() {
            return direct.to_string();
        }

        transcription_output
            .segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }

    fn normalize_source_language(language: Option<&str>) -> String {
        let Some(language) = language.map(str::trim).filter(|value| !value.is_empty()) else {
            return "auto".to_string();
        };
        if language.eq_ignore_ascii_case("auto") || language.eq_ignore_ascii_case("und") {
            return "auto".to_string();
        }
        LanguageCode::try_from_code(language)
            .map(|code| {
                if code.is_auto() {
                    "auto".to_string()
                } else {
                    code.as_code().to_string()
                }
            })
            .unwrap_or_else(|_| "auto".to_string())
    }

    fn normalize_source_text(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn language_optimization_groups(
        transcription_output: &TranscriptionOutput,
        fallback_text: &str,
    ) -> Vec<SourceLanguageOptimizationGroup> {
        let fallback_text = fallback_text.trim();
        if fallback_text.is_empty() {
            return Vec::new();
        }

        let mut groups = Vec::<SourceLanguageOptimizationGroup>::new();
        for segment in &transcription_output.segments {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            let language = Self::normalize_source_language(segment.language_code.as_deref());
            let append_to_previous = groups
                .last()
                .is_some_and(|previous| previous.language_code == language);
            if append_to_previous {
                if let Some(previous) = groups.last_mut() {
                    previous.text.push('\n');
                    previous.text.push_str(text);
                }
            } else {
                groups.push(SourceLanguageOptimizationGroup {
                    language_code: language,
                    text: text.to_string(),
                });
            }
        }

        let grouped_text = groups
            .iter()
            .map(|group| group.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if !groups.is_empty()
            && Self::normalize_source_text(&grouped_text)
                == Self::normalize_source_text(fallback_text)
        {
            return groups;
        }

        vec![SourceLanguageOptimizationGroup {
            language_code: "auto".to_string(),
            text: fallback_text.to_string(),
        }]
    }

    async fn optimize_language_groups(
        enhancer: &dyn TranscriptEnhancer,
        groups: &[SourceLanguageOptimizationGroup],
    ) -> Result<String, ApplicationError> {
        let mut optimized_groups = Vec::with_capacity(groups.len());
        for group in groups {
            let optimized = enhancer.optimize(&group.text, &group.language_code).await?;
            let optimized = Self::strip_language_service_markers(&optimized);
            let constrained = constrain_transcript_edit(&group.text, &optimized);
            optimized_groups.push(if constrained.trim().is_empty() {
                group.text.clone()
            } else {
                constrained
            });
        }
        Ok(optimized_groups.join("\n").trim().to_string())
    }

    fn strip_language_service_markers(value: &str) -> String {
        let mut cleaned = value.to_string();
        while let Some(start) = cleaned.find("[source_language=") {
            let Some(relative_end) = cleaned[start..].find(']') else {
                break;
            };
            let end = start + relative_end + 1;
            cleaned.replace_range(start..end, "");
        }
        cleaned
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }

    fn wav_duration_seconds(&self, wav_path: &Path) -> Option<f32> {
        let reader = hound::WavReader::open(wav_path).ok()?;
        let spec = reader.spec();
        if spec.channels == 0 || spec.sample_rate == 0 {
            return None;
        }

        let samples = reader.duration() as f32;
        let frames = samples / f32::from(spec.channels);
        if frames <= 0.0 {
            return None;
        }

        Some(frames / (spec.sample_rate as f32))
    }

    pub fn assign_speakers_to_segments(
        segments: &[TimedSegment],
        turns: &[SpeakerTurn],
    ) -> Vec<TimedSegment> {
        let sanitized_turns = turns
            .iter()
            .filter_map(|turn| {
                if !turn.start_seconds.is_finite()
                    || !turn.end_seconds.is_finite()
                    || turn.end_seconds <= turn.start_seconds
                    || turn.speaker_id.trim().is_empty()
                {
                    return None;
                }

                Some(SpeakerTurn {
                    speaker_id: turn.speaker_id.trim().to_string(),
                    speaker_label: turn
                        .speaker_label
                        .as_ref()
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty()),
                    start_seconds: turn.start_seconds.max(0.0),
                    end_seconds: turn.end_seconds.max(0.0),
                })
            })
            .collect::<Vec<_>>();

        if sanitized_turns.is_empty() {
            return segments.to_vec();
        }

        segments
            .iter()
            .map(|segment| {
                let Some((segment_start, segment_end)) = Self::segment_bounds(segment) else {
                    return segment.clone();
                };

                let midpoint = (segment_start + segment_end) / 2.0;
                let mut best_overlap = 0.0_f32;
                let mut best_distance = f32::MAX;
                let mut best_turn: Option<&SpeakerTurn> = None;

                for turn in &sanitized_turns {
                    let overlap = (segment_end.min(turn.end_seconds)
                        - segment_start.max(turn.start_seconds))
                    .max(0.0);
                    let distance = if midpoint < turn.start_seconds {
                        turn.start_seconds - midpoint
                    } else if midpoint > turn.end_seconds {
                        midpoint - turn.end_seconds
                    } else {
                        0.0
                    };

                    if overlap > best_overlap + 0.001
                        || ((overlap - best_overlap).abs() <= 0.001 && distance < best_distance)
                    {
                        best_overlap = overlap;
                        best_distance = distance;
                        best_turn = Some(turn);
                    }
                }

                let Some(turn) = best_turn else {
                    return segment.clone();
                };

                let mut next = segment.clone();
                next.speaker_id = Some(turn.speaker_id.clone());
                next.speaker_label = turn.speaker_label.clone();
                next
            })
            .collect()
    }

    fn segment_bounds(segment: &TimedSegment) -> Option<(f32, f32)> {
        let start = segment.start_seconds.or_else(|| {
            segment
                .words
                .iter()
                .find_map(|word| word.start_seconds.filter(|value| value.is_finite()))
        })?;
        let end = segment.end_seconds.or_else(|| {
            segment
                .words
                .iter()
                .rev()
                .find_map(|word| word.end_seconds.filter(|value| value.is_finite()))
        })?;

        if !start.is_finite() || !end.is_finite() || end <= start {
            return None;
        }

        Some((start.max(0.0), end.max(0.0)))
    }

    async fn run_cancellable<T, F>(
        &self,
        cancellation_token: &CancellationToken,
        operation: F,
    ) -> Result<T, ApplicationError>
    where
        F: Future<Output = Result<T, ApplicationError>>,
    {
        tokio::select! {
            _ = cancellation_token.cancelled() => Err(ApplicationError::Cancelled),
            result = operation => result,
        }
    }
}

fn metadata_bool(request: &RunTranscriptionRequest, key: &str, default: bool) -> bool {
    request
        .metadata
        .get(key)
        .map(|value| matches!(value.trim(), "true" | "1" | "yes"))
        .unwrap_or(default)
}

fn transcription_progress_message(engine: &TranscriptionEngine) -> &'static str {
    match engine {
        TranscriptionEngine::WhisperCpp => "Running Whisper transcription",
        TranscriptionEngine::ParakeetCpp => "Running Parakeet transcription",
    }
}
