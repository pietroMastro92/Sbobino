use std::{
    collections::BTreeMap,
    future::Future,
    path::Path,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tokio::fs;
use tokio_util::sync::CancellationToken;
use tracing::{instrument, warn};

use sbobino_domain::{
    constrain_transcript_edit, minimize_transcript_repetitions, ArtifactKind, JobProgress,
    JobStage, SpeakerTurn, TimedSegment, TranscriptArtifact, TranscriptionOutput,
};

use crate::{
    dto::{RunTranscriptionRequest, SummaryFaq},
    ApplicationError, ArtifactRepository, AudioTranscoder, SpeakerDiarizationEngine,
    SpeechToTextEngine, TranscriptEnhancer,
};

const HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY: &str = "has_optimized_transcript";

#[derive(Clone)]
pub struct TranscriptionService {
    transcoder: Arc<dyn AudioTranscoder>,
    speech_engine: Arc<dyn SpeechToTextEngine>,
    speaker_diarizer: Option<Arc<dyn SpeakerDiarizationEngine>>,
    enhancer: Arc<dyn TranscriptEnhancer>,
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

        self.emit(
            &emit_progress,
            &request.job_id,
            JobStage::PreparingAudio,
            "Preparing audio",
            10,
            None,
            None,
        );
        let job_id = request.job_id.clone();

        let wav_path = self.normalized_wav_path(&input_path, &request.job_id);
        let result = async {
            if input_path.extension().and_then(|s| s.to_str()) != Some("wav") {
                self.run_cancellable(
                    &cancellation_token,
                    self.transcoder.to_wav_mono_16k(&input_path, &wav_path),
                )
                .await?;
            } else {
                self.run_cancellable(&cancellation_token, async {
                    fs::copy(&input_path, &wav_path).await.map_err(|e| {
                        ApplicationError::AudioTranscoding(format!("failed to copy wav input: {e}"))
                    })?;
                    Ok(())
                })
                .await?;
            }

            let total_audio_seconds = self.wav_duration_seconds(&wav_path);

            self.emit(
                &emit_progress,
                &request.job_id,
                JobStage::Transcribing,
                "Running Whisper transcription",
                0,
                Some(0.0),
                total_audio_seconds,
            );

            let progress_callback = {
                let emit_progress = emit_progress.clone();
                let job_id = request.job_id.clone();
                let total_audio_seconds = total_audio_seconds;
                let last_emitted_seconds = Arc::new(Mutex::new(0_f32));
                let last_emitted_seconds_ref = last_emitted_seconds.clone();

                Arc::new(move |current_seconds: f32| {
                    let sanitized_seconds = current_seconds.max(0.0);
                    if let Ok(mut last) = last_emitted_seconds_ref.lock() {
                        if sanitized_seconds <= *last + 0.05 {
                            return;
                        }
                        *last = sanitized_seconds;
                    }

                    let percentage = match total_audio_seconds {
                        Some(total) if total > 0.0 => {
                            ((sanitized_seconds / total).clamp(0.0, 1.0) * 100.0).round() as u8
                        }
                        _ => 0,
                    };

                    emit_progress(JobProgress {
                        job_id: job_id.clone(),
                        stage: JobStage::Transcribing,
                        message: "Running Whisper transcription".to_string(),
                        percentage,
                        current_seconds: Some(sanitized_seconds),
                        total_seconds: total_audio_seconds,
                    });
                }) as Arc<dyn Fn(f32) + Send + Sync>
            };

            let mut transcription_output = self
                .run_cancellable(
                    &cancellation_token,
                    self.speech_engine.transcribe(
                        &wav_path,
                        request.model.ggml_filename(),
                        request.language.as_whisper_code(),
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
                    &request.job_id,
                    JobStage::Transcribing,
                    "Running Whisper transcription",
                    100,
                    Some(total),
                    Some(total),
                );
            }

            let mut diarization_status: Option<String> = None;
            let mut diarization_error: Option<String> = None;
            if let Some(speaker_diarizer) = &self.speaker_diarizer {
                self.emit(
                    &emit_progress,
                    &request.job_id,
                    JobStage::Diarizing,
                    "Assigning speakers with pyannote",
                    60,
                    None,
                    None,
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
            }

            let (optimized, summary_faq, has_optimized_transcript) = if request.enable_ai {
                self.emit(
                    &emit_progress,
                    &request.job_id,
                    JobStage::Optimizing,
                    "Optimizing transcript with AI",
                    65,
                    None,
                    None,
                );

                match self
                    .run_cancellable(
                        &cancellation_token,
                        self.enhancer
                            .optimize(&raw_transcript, request.language.as_whisper_code()),
                    )
                    .await
                {
                    Ok(optimized) => {
                        let constrained_optimized =
                            constrain_transcript_edit(&raw_transcript, &optimized);
                        self.emit(
                            &emit_progress,
                            &request.job_id,
                            JobStage::Summarizing,
                            "Generating summary and FAQs",
                            80,
                            None,
                            None,
                        );

                        let summary_faq = match self
                            .run_cancellable(
                                &cancellation_token,
                                self.enhancer.summarize_and_faq(
                                    &constrained_optimized,
                                    request.language.as_whisper_code(),
                                ),
                            )
                            .await
                        {
                            Ok(value) => value,
                            Err(ApplicationError::Cancelled) => {
                                return Err(ApplicationError::Cancelled);
                            }
                            Err(error) => {
                                warn!("summary/faq generation skipped after optimization: {error}");
                                SummaryFaq {
                                    summary: String::new(),
                                    faqs: String::new(),
                                }
                            }
                        };

                        (constrained_optimized, summary_faq, true)
                    }
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
                )
            };

            self.emit(
                &emit_progress,
                &request.job_id,
                JobStage::Persisting,
                "Persisting transcription artifact",
                90,
                None,
                None,
            );

            let mut metadata = BTreeMap::new();
            metadata.insert(
                "model".to_string(),
                request.model.ggml_filename().to_string(),
            );
            metadata.insert(
                "language".to_string(),
                request.language.as_whisper_code().to_string(),
            );
            metadata.insert(
                "timeline_v2".to_string(),
                transcription_output.timeline_v2_metadata_json(),
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

            let final_title = request.title.clone().unwrap_or_else(|| {
                input_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&request.input_path)
                    .to_string()
            });

            let artifact = TranscriptArtifact::new(
                request.job_id.clone(),
                final_title,
                ArtifactKind::File,
                request.input_path.clone(),
                raw_transcript,
                optimized,
                summary_faq.summary,
                summary_faq.faqs,
                metadata,
            )
            .map_err(|e| ApplicationError::Validation(e.to_string()))?;

            self.run_cancellable(&cancellation_token, self.artifacts.save(&artifact))
                .await?;

            self.emit(
                &emit_progress,
                &artifact.job_id,
                JobStage::Completed,
                "Transcription completed",
                100,
                None,
                None,
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
                    &job_id,
                    JobStage::Cancelled,
                    "Transcription cancelled",
                    100,
                    None,
                    None,
                );
            }
            Err(error) => {
                self.emit(
                    &emit_progress,
                    &job_id,
                    JobStage::Failed,
                    &format!("Transcription failed: {error}"),
                    100,
                    None,
                    None,
                );
            }
            Ok(_) => {}
        }

        result
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

    fn emit(
        &self,
        callback: &Arc<dyn Fn(JobProgress) + Send + Sync>,
        job_id: &str,
        stage: JobStage,
        message: &str,
        percentage: u8,
        current_seconds: Option<f32>,
        total_seconds: Option<f32>,
    ) {
        callback(JobProgress {
            job_id: job_id.to_string(),
            stage,
            message: message.to_string(),
            percentage,
            current_seconds,
            total_seconds,
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

    fn assign_speakers_to_segments(
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
