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

use sbobino_domain::{ArtifactKind, JobProgress, JobStage, TranscriptArtifact};

use crate::{
    dto::{RunTranscriptionRequest, SummaryFaq},
    ApplicationError, ArtifactRepository, AudioTranscoder, SpeechToTextEngine, TranscriptEnhancer,
};

#[derive(Clone)]
pub struct TranscriptionService {
    transcoder: Arc<dyn AudioTranscoder>,
    speech_engine: Arc<dyn SpeechToTextEngine>,
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
            enhancer,
            artifacts,
        }
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

            let raw_transcript = self
                .run_cancellable(
                    &cancellation_token,
                    self.speech_engine.transcribe(
                        &wav_path,
                        request.model.ggml_filename(),
                        request.language.as_whisper_code(),
                        &request.whisper_options,
                        emit_delta.clone(),
                        progress_callback,
                    ),
                )
                .await?;

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

            let (optimized, summary_faq) = if request.enable_ai {
                self.emit(
                    &emit_progress,
                    &request.job_id,
                    JobStage::Optimizing,
                    "Optimizing transcript with AI",
                    65,
                    None,
                    None,
                );

                let optimized = self
                    .run_cancellable(
                        &cancellation_token,
                        self.enhancer
                            .optimize(&raw_transcript, request.language.as_whisper_code()),
                    )
                    .await?;

                self.emit(
                    &emit_progress,
                    &request.job_id,
                    JobStage::Summarizing,
                    "Generating summary and FAQs",
                    80,
                    None,
                    None,
                );

                let summary_faq = self
                    .run_cancellable(
                        &cancellation_token,
                        self.enhancer
                            .summarize_and_faq(&optimized, request.language.as_whisper_code()),
                    )
                    .await?;
                (optimized, summary_faq)
            } else {
                (
                    raw_transcript.clone(),
                    SummaryFaq {
                        summary: String::new(),
                        faqs: String::new(),
                    },
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

            let artifact = TranscriptArtifact::new(
                request.job_id.clone(),
                input_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&request.input_path)
                    .to_string(),
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
