use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use sbobino_application::{
    dto::SummaryFaq, ApplicationError, ArtifactRepository, AudioTranscoder,
    RunTranscriptionRequest, SpeakerDiarizationEngine, SpeechToTextEngine, TranscriptEnhancer,
    TranscriptionService,
};
use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, JobProgress, JobStage, LanguageCode, SpeakerTurn,
    SpeechModel, TimedSegment, TranscriptArtifact, TranscriptionEngine, TranscriptionOutput,
    WhisperOptions,
};

const HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY: &str = "has_optimized_transcript";

#[derive(Default)]
struct MockTranscoder {
    calls: Mutex<usize>,
}

#[async_trait]
impl AudioTranscoder for MockTranscoder {
    async fn to_wav_mono_16k(&self, _input: &Path, _output: &Path) -> Result<(), ApplicationError> {
        let mut calls = self.calls.lock().expect("transcoder calls lock poisoned");
        *calls += 1;
        Ok(())
    }
}

struct MockSpeechEngine {
    transcript: String,
    segments: Vec<TimedSegment>,
}

#[async_trait]
impl SpeechToTextEngine for MockSpeechEngine {
    async fn transcribe(
        &self,
        _input_wav: &Path,
        _model_filename: &str,
        _language_code: &str,
        _options: &WhisperOptions,
        _total_audio_seconds: Option<f32>,
        _emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        _emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        Ok(TranscriptionOutput {
            text: self.transcript.clone(),
            segments: self.segments.clone(),
        })
    }
}

#[derive(Default)]
struct MockSpeakerDiarizer {
    turns: Vec<SpeakerTurn>,
    fail_with: Option<String>,
}

#[async_trait]
impl SpeakerDiarizationEngine for MockSpeakerDiarizer {
    async fn diarize(&self, _input_wav: &Path) -> Result<Vec<SpeakerTurn>, ApplicationError> {
        if let Some(message) = &self.fail_with {
            return Err(ApplicationError::SpeakerDiarization(message.clone()));
        }
        Ok(self.turns.clone())
    }
}

#[derive(Default)]
struct MockEnhancer {
    optimize_calls: Mutex<usize>,
    summarize_calls: Mutex<usize>,
    fail_optimize: bool,
    fail_summarize: bool,
}

#[async_trait]
impl TranscriptEnhancer for MockEnhancer {
    async fn optimize(&self, text: &str, _language_code: &str) -> Result<String, ApplicationError> {
        let mut optimize_calls = self
            .optimize_calls
            .lock()
            .expect("enhancer optimize lock poisoned");
        *optimize_calls += 1;
        if self.fail_optimize {
            return Err(ApplicationError::PostProcessing(
                "optimize failed".to_string(),
            ));
        }
        Ok(format!("optimized::{text}"))
    }

    async fn summarize_and_faq(
        &self,
        text: &str,
        _language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError> {
        let mut summarize_calls = self
            .summarize_calls
            .lock()
            .expect("enhancer summarize lock poisoned");
        *summarize_calls += 1;
        if self.fail_summarize {
            return Err(ApplicationError::PostProcessing(
                "summary failed".to_string(),
            ));
        }
        Ok(SummaryFaq {
            summary: format!("summary::{text}"),
            faqs: format!("faqs::{text}"),
        })
    }

    async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
        let transcript = prompt
            .split("Transcript:\n")
            .nth(1)
            .or_else(|| prompt.split("Chunk notes:\n").nth(1))
            .unwrap_or_default()
            .trim();
        let mut summarize_calls = self
            .summarize_calls
            .lock()
            .expect("enhancer summarize lock poisoned");
        *summarize_calls += 1;
        if self.fail_summarize {
            return Err(ApplicationError::PostProcessing(
                "summary failed".to_string(),
            ));
        }
        Ok(format!(
            "Summary:\nsummary::{transcript}\nFAQs:\nfaqs::{transcript}"
        ))
    }

    fn telemetry_provider_label(&self) -> &'static str {
        "mock"
    }
}

struct RetryableEnhancer {
    label: &'static str,
    optimize_calls: Arc<Mutex<usize>>,
    summarize_calls: Arc<Mutex<usize>>,
    fail_optimize_retryably: bool,
}

#[async_trait]
impl TranscriptEnhancer for RetryableEnhancer {
    async fn optimize(&self, text: &str, _language_code: &str) -> Result<String, ApplicationError> {
        let mut optimize_calls = self
            .optimize_calls
            .lock()
            .expect("retryable enhancer optimize lock poisoned");
        *optimize_calls += 1;
        if self.fail_optimize_retryably {
            return Err(ApplicationError::PostProcessing(
                "AI request failed: connection refused".to_string(),
            ));
        }
        Ok(format!("{text}."))
    }

    async fn summarize_and_faq(
        &self,
        text: &str,
        _language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError> {
        let mut summarize_calls = self
            .summarize_calls
            .lock()
            .expect("retryable enhancer summarize lock poisoned");
        *summarize_calls += 1;
        Ok(SummaryFaq {
            summary: format!("{}::summary::{text}", self.label),
            faqs: String::new(),
        })
    }

    async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
        let transcript = prompt
            .split("Transcript:\n")
            .nth(1)
            .or_else(|| prompt.split("Chunk notes:\n").nth(1))
            .unwrap_or_default()
            .trim();
        let mut summarize_calls = self
            .summarize_calls
            .lock()
            .expect("retryable enhancer summarize lock poisoned");
        *summarize_calls += 1;
        Ok(format!(
            "Summary:\n{}::summary::{transcript}\nFAQs:\n",
            self.label
        ))
    }

    fn telemetry_provider_label(&self) -> &'static str {
        self.label
    }
}

#[derive(Default)]
struct InMemoryArtifactRepository {
    artifacts: Mutex<Vec<TranscriptArtifact>>,
    deleted_ids: Mutex<HashSet<String>>,
}

#[async_trait]
impl ArtifactRepository for InMemoryArtifactRepository {
    async fn save(&self, artifact: &TranscriptArtifact) -> Result<(), ApplicationError> {
        self.artifacts
            .lock()
            .expect("artifact repo lock poisoned")
            .push(artifact.clone());
        Ok(())
    }

    async fn list_recent(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");
        Ok(artifacts
            .iter()
            .filter(|artifact| !deleted_ids.contains(&artifact.id))
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_filtered(
        &self,
        kind: Option<ArtifactKind>,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");
        let query = query.map(|needle| needle.to_lowercase());

        let filtered = artifacts
            .iter()
            .filter(|artifact| {
                if deleted_ids.contains(&artifact.id) {
                    return false;
                }
                let kind_match = kind
                    .as_ref()
                    .is_none_or(|expected| &artifact.kind == expected);
                let query_match = query.as_ref().is_none_or(|needle| {
                    artifact.title.to_lowercase().contains(needle)
                        || artifact.source_label.to_lowercase().contains(needle)
                        || artifact
                            .optimized_transcript
                            .to_lowercase()
                            .contains(needle)
                });
                kind_match && query_match
            })
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        Ok(filtered)
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");
        Ok(artifacts
            .iter()
            .find(|artifact| artifact.id == id && !deleted_ids.contains(&artifact.id))
            .cloned())
    }

    async fn update_content(
        &self,
        id: &str,
        optimized_transcript: &str,
        summary: &str,
        faqs: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let mut artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let Some(artifact) = artifacts.iter_mut().find(|artifact| artifact.id == id) else {
            return Ok(None);
        };

        artifact.optimized_transcript = optimized_transcript.to_string();
        artifact.summary = summary.to_string();
        artifact.faqs = faqs.to_string();
        if optimized_transcript.trim().is_empty() {
            artifact
                .metadata
                .remove(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY);
        } else {
            artifact.metadata.insert(
                HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY.to_string(),
                "true".to_string(),
            );
        }
        artifact.touch();
        Ok(Some(artifact.clone()))
    }

    async fn update_metadata_entry(
        &self,
        id: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let mut artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let Some(artifact) = artifacts.iter_mut().find(|artifact| artifact.id == id) else {
            return Ok(None);
        };

        match value {
            Some(next_value) => {
                artifact
                    .metadata
                    .insert(key.to_string(), next_value.to_string());
            }
            None => {
                artifact.metadata.remove(key);
            }
        }
        artifact.touch();
        Ok(Some(artifact.clone()))
    }

    async fn update_timeline_v2(
        &self,
        id: &str,
        timeline_v2_json: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let mut artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let Some(artifact) = artifacts.iter_mut().find(|artifact| artifact.id == id) else {
            return Ok(None);
        };

        artifact
            .metadata
            .insert("timeline_v2".to_string(), timeline_v2_json.to_string());
        artifact.touch();
        Ok(Some(artifact.clone()))
    }

    async fn update_emotion_analysis(
        &self,
        id: &str,
        emotion_analysis_json: &str,
        generated_at: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let mut artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let Some(artifact) = artifacts.iter_mut().find(|artifact| artifact.id == id) else {
            return Ok(None);
        };

        artifact.metadata.insert(
            "emotion_analysis_v1".to_string(),
            emotion_analysis_json.to_string(),
        );
        artifact.metadata.insert(
            "emotion_analysis_generated_at".to_string(),
            generated_at.to_string(),
        );
        artifact.touch();
        Ok(Some(artifact.clone()))
    }

    async fn rename(
        &self,
        id: &str,
        new_title: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let mut artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let Some(artifact) = artifacts.iter_mut().find(|artifact| artifact.id == id) else {
            return Ok(None);
        };

        artifact.title = new_title.to_string();
        artifact.touch();
        Ok(Some(artifact.clone()))
    }

    async fn delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let mut deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");

        let mut moved = 0;
        for id in ids {
            if artifacts.iter().any(|artifact| artifact.id == *id) && deleted_ids.insert(id.clone())
            {
                moved += 1;
            }
        }
        Ok(moved)
    }

    async fn list_deleted(
        &self,
        kind: Option<ArtifactKind>,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");
        let query = query.map(|needle| needle.to_lowercase());

        let filtered = artifacts
            .iter()
            .filter(|artifact| {
                if !deleted_ids.contains(&artifact.id) {
                    return false;
                }
                let kind_match = kind
                    .as_ref()
                    .is_none_or(|expected| &artifact.kind == expected);
                let query_match = query.as_ref().is_none_or(|needle| {
                    artifact.title.to_lowercase().contains(needle)
                        || artifact.source_label.to_lowercase().contains(needle)
                        || artifact
                            .optimized_transcript
                            .to_lowercase()
                            .contains(needle)
                });
                kind_match && query_match
            })
            .skip(offset)
            .take(limit)
            .cloned()
            .collect();

        Ok(filtered)
    }

    async fn restore_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let mut deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");
        let mut restored = 0;
        for id in ids {
            if deleted_ids.remove(id) {
                restored += 1;
            }
        }
        Ok(restored)
    }

    async fn hard_delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let mut artifacts = self.artifacts.lock().expect("artifact repo lock poisoned");
        let mut deleted_ids = self
            .deleted_ids
            .lock()
            .expect("artifact repo deleted ids lock poisoned");
        let before = artifacts.len();
        artifacts.retain(|artifact| !ids.contains(&artifact.id));
        for id in ids {
            deleted_ids.remove(id);
        }
        Ok(before.saturating_sub(artifacts.len()))
    }

    async fn purge_deleted_older_than_days(&self, _days: u32) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn read_audio_bytes(&self, _id: &str) -> Result<Option<Vec<u8>>, ApplicationError> {
        Err(ApplicationError::Persistence(
            "audio bytes not available in test repository".to_string(),
        ))
    }
}

#[tokio::test]
async fn run_file_transcription_without_ai_emits_expected_stages_and_persists() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("lecture.mp3");
    tokio::fs::write(&input_path, b"fake mp3 content")
        .await
        .expect("failed to create test input file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "raw transcript".to_string(),
        segments: Vec::new(),
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service =
        TranscriptionService::new(transcoder.clone(), speech, enhancer.clone(), repo.clone());

    let emitted: Arc<Mutex<Vec<JobProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-001".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(move |event| {
                emitted_clone
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(event);
            }),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription service should succeed");

    let stage_list: Vec<JobStage> = emitted
        .lock()
        .expect("emitted lock poisoned")
        .iter()
        .map(|item| item.stage.clone())
        .collect();

    assert_eq!(
        stage_list,
        vec![
            JobStage::PreparingAudio,
            JobStage::Transcribing,
            JobStage::Persisting,
            JobStage::Completed
        ]
    );

    assert_eq!(artifact.raw_transcript, "raw transcript");
    assert!(artifact.optimized_transcript.is_empty());
    assert!(!artifact
        .metadata
        .contains_key(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY));
    assert!(artifact.summary.is_empty());
    assert!(artifact.faqs.is_empty());

    assert_eq!(
        *transcoder
            .calls
            .lock()
            .expect("transcoder calls lock poisoned"),
        1
    );
    assert_eq!(
        *enhancer
            .optimize_calls
            .lock()
            .expect("enhancer optimize lock poisoned"),
        0
    );
    assert_eq!(
        *enhancer
            .summarize_calls
            .lock()
            .expect("enhancer summarize lock poisoned"),
        0
    );

    let persisted = repo.list_recent(10, 0).await.expect("list should succeed");
    assert_eq!(persisted.len(), 1);
}

#[tokio::test]
async fn run_file_transcription_emits_final_transcript_snapshot_before_post_processing() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("lecture.mp3");
    tokio::fs::write(&input_path, b"fake mp3 content")
        .await
        .expect("failed to create test input file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "line one\nline two".to_string(),
        segments: Vec::new(),
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service = TranscriptionService::new(transcoder, speech, enhancer, repo);
    let emitted_partials: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_partials_clone = emitted_partials.clone();

    service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-001b".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(move |text: String| {
                emitted_partials_clone
                    .lock()
                    .expect("emitted partials lock poisoned")
                    .push(text);
            }),
            CancellationToken::new(),
        )
        .await
        .expect("transcription service should succeed");

    let partials = emitted_partials
        .lock()
        .expect("emitted partials lock poisoned");
    assert_eq!(
        partials.last().map(String::as_str),
        Some("line one\nline two")
    );
}

#[tokio::test]
async fn run_file_transcription_with_ai_runs_enhancer_steps() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("meeting.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "meeting raw".to_string(),
        segments: Vec::new(),
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service = TranscriptionService::new(transcoder, speech, enhancer.clone(), repo);

    let emitted: Arc<Mutex<Vec<JobProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-002".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Small,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: true,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(move |event| {
                emitted_clone
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(event);
            }),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription with ai should succeed");

    let stages: Vec<JobStage> = emitted
        .lock()
        .expect("emitted lock poisoned")
        .iter()
        .map(|item| item.stage.clone())
        .collect();

    assert!(stages.contains(&JobStage::Optimizing));
    assert!(stages.contains(&JobStage::Summarizing));

    assert_eq!(artifact.optimized_transcript, "meeting raw");
    assert_eq!(
        artifact
            .metadata
            .get(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY)
            .map(String::as_str),
        None
    );
    assert_eq!(artifact.summary, "summary::meeting raw");
    assert_eq!(artifact.faqs, "faqs::meeting raw");

    assert_eq!(
        *enhancer
            .optimize_calls
            .lock()
            .expect("enhancer optimize lock poisoned"),
        1
    );
    assert_eq!(
        *enhancer
            .summarize_calls
            .lock()
            .expect("enhancer summarize lock poisoned"),
        1
    );
}

#[tokio::test]
async fn run_file_transcription_rejects_missing_input_path() {
    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "raw transcript".to_string(),
        segments: Vec::new(),
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service = TranscriptionService::new(transcoder, speech, enhancer, repo);

    let error = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-003".to_string(),
                input_path: "non-existent-file.wav".to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect_err("missing file should fail validation");

    match error {
        ApplicationError::Validation(message) => {
            assert!(message.contains("input file not found"));
        }
        other => panic!("expected validation error, got: {other:?}"),
    }
}

#[tokio::test]
async fn run_file_transcription_assigns_speakers_into_timeline_metadata() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("interview.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "Hello there.\nGeneral Kenobi.".to_string(),
        segments: vec![
            TimedSegment {
                text: "Hello there.".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(1.8),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "General Kenobi.".to_string(),
                start_seconds: Some(2.0),
                end_seconds: Some(3.8),
                ..TimedSegment::default()
            },
        ],
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());
    let diarizer = Arc::new(MockSpeakerDiarizer {
        turns: vec![
            SpeakerTurn {
                speaker_id: "speaker_1".to_string(),
                speaker_label: Some("Speaker 1".to_string()),
                start_seconds: 0.0,
                end_seconds: 2.0,
            },
            SpeakerTurn {
                speaker_id: "speaker_2".to_string(),
                speaker_label: Some("Speaker 2".to_string()),
                start_seconds: 2.0,
                end_seconds: 4.0,
            },
        ],
        fail_with: None,
    });

    let service = TranscriptionService::new(transcoder, speech, enhancer, repo)
        .with_speaker_diarizer(diarizer);

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-004".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription with diarization should succeed");

    let timeline = artifact
        .metadata
        .get("timeline_v2")
        .expect("timeline metadata should be present");
    assert!(timeline.contains("\"speaker_id\":\"speaker_1\""));
    assert!(timeline.contains("\"speaker_label\":\"Speaker 1\""));
    assert!(timeline.contains("\"speaker_id\":\"speaker_2\""));
    assert_eq!(
        artifact
            .metadata
            .get("speaker_diarization_status")
            .map(String::as_str),
        Some("completed")
    );
}

#[tokio::test]
async fn run_file_transcription_persists_diarization_failure_metadata() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("meeting.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "meeting raw".to_string(),
        segments: vec![TimedSegment {
            text: "Hello there.".to_string(),
            start_seconds: Some(0.0),
            end_seconds: Some(1.8),
            ..TimedSegment::default()
        }],
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());
    let diarizer = Arc::new(MockSpeakerDiarizer {
        fail_with: Some("pyannote crashed".to_string()),
        ..MockSpeakerDiarizer::default()
    });

    let service = TranscriptionService::new(transcoder, speech, enhancer, repo)
        .with_speaker_diarizer(diarizer);

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-004b".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription should still succeed when diarization fails");

    assert_eq!(
        artifact
            .metadata
            .get("speaker_diarization_status")
            .map(String::as_str),
        Some("failed")
    );
    assert_eq!(
        artifact
            .metadata
            .get("speaker_diarization_error")
            .map(String::as_str),
        Some("speaker diarization failed: pyannote crashed")
    );
}

#[tokio::test]
async fn run_file_transcription_keeps_raw_transcript_when_ai_fails() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("meeting.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "meeting raw".to_string(),
        segments: Vec::new(),
    });
    let enhancer = Arc::new(MockEnhancer {
        fail_optimize: true,
        ..MockEnhancer::default()
    });
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service = TranscriptionService::new(transcoder, speech, enhancer.clone(), repo);

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-005".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Small,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: true,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription should still succeed when ai fails");

    assert_eq!(artifact.raw_transcript, "meeting raw");
    assert!(artifact.optimized_transcript.is_empty());
    assert!(!artifact
        .metadata
        .contains_key(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY));
    assert!(artifact.summary.is_empty());
    assert!(artifact.faqs.is_empty());
    assert_eq!(
        *enhancer
            .optimize_calls
            .lock()
            .expect("enhancer optimize lock poisoned"),
        1
    );
}

#[tokio::test]
async fn run_file_transcription_falls_back_to_secondary_ai_provider() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("fallback.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let first_optimize_calls = Arc::new(Mutex::new(0));
    let first_summarize_calls = Arc::new(Mutex::new(0));
    let second_optimize_calls = Arc::new(Mutex::new(0));
    let second_summarize_calls = Arc::new(Mutex::new(0));

    let primary = Arc::new(RetryableEnhancer {
        label: "remote",
        optimize_calls: first_optimize_calls.clone(),
        summarize_calls: first_summarize_calls.clone(),
        fail_optimize_retryably: true,
    });
    let fallback = Arc::new(RetryableEnhancer {
        label: "foundation",
        optimize_calls: second_optimize_calls.clone(),
        summarize_calls: second_summarize_calls.clone(),
        fail_optimize_retryably: false,
    });

    let service = TranscriptionService::new(
        Arc::new(MockTranscoder::default()),
        Arc::new(MockSpeechEngine {
            transcript: "meeting raw".to_string(),
            segments: Vec::new(),
        }),
        primary,
        Arc::new(InMemoryArtifactRepository::default()),
    )
    .with_fallback_enhancers(vec![fallback]);

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-006".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Small,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: true,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription should succeed through fallback");

    assert_eq!(artifact.raw_transcript, "meeting raw");
    assert_eq!(artifact.optimized_transcript, "meeting raw.");
    assert_eq!(artifact.summary, "foundation::summary::meeting raw.");
    assert_eq!(
        *first_optimize_calls
            .lock()
            .expect("first optimize lock poisoned"),
        1
    );
    assert_eq!(
        *first_summarize_calls
            .lock()
            .expect("first summarize lock poisoned"),
        0
    );
    assert_eq!(
        *second_optimize_calls
            .lock()
            .expect("second optimize lock poisoned"),
        1
    );
    assert_eq!(
        *second_summarize_calls
            .lock()
            .expect("second summarize lock poisoned"),
        1
    );
}

#[tokio::test]
async fn run_file_transcription_preserves_auto_import_metadata_and_fingerprint() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("memo.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let service = TranscriptionService::new(
        Arc::new(MockTranscoder::default()),
        Arc::new(MockSpeechEngine {
            transcript: "memo raw".to_string(),
            segments: Vec::new(),
        }),
        Arc::new(MockEnhancer::default()),
        Arc::new(InMemoryArtifactRepository::default()),
    );

    let mut metadata = BTreeMap::new();
    metadata.insert("workspace_id".to_string(), "work".to_string());
    metadata.insert("auto_import_preset".to_string(), "voice_memo".to_string());
    metadata.insert(
        "auto_import_source_path".to_string(),
        input_path.to_string_lossy().to_string(),
    );

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-007".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: Some("Memo".to_string()),
                parent_id: None,
                metadata,
                source_fingerprint_json: Some(
                    "{\"path\":\"/tmp/memo.wav\",\"dedupe_key\":\"123\"}".to_string(),
                ),
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("transcription should succeed");

    assert_eq!(
        artifact.metadata.get("workspace_id").map(String::as_str),
        Some("work")
    );
    assert_eq!(
        artifact
            .metadata
            .get("auto_import_preset")
            .map(String::as_str),
        Some("voice_memo")
    );
    assert_eq!(
        artifact.source_fingerprint_json.as_deref(),
        Some("{\"path\":\"/tmp/memo.wav\",\"dedupe_key\":\"123\"}")
    );
}

#[tokio::test]
async fn run_file_transcription_transcodes_wav_inputs_unconditionally() {
    // Regression: previously, WAV inputs were fs::copy'd straight through to the
    // pyannote helper, which uses Python's `wave` module and rejects non-PCM
    // formats (IEEE float, mu-law, ...) with "unknown format: 3". Every job
    // must now go through ffmpeg so downstream engines receive PCM-16 mono 16 kHz.
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("float32_source.wav");
    tokio::fs::write(&input_path, b"fake float32 wav payload")
        .await
        .expect("failed to create wav file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(MockSpeechEngine {
        transcript: "already transcoded".to_string(),
        segments: Vec::new(),
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service = TranscriptionService::new(transcoder.clone(), speech, enhancer, repo);

    let _ = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-wav-transcode".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_text: String| {}),
            CancellationToken::new(),
        )
        .await
        .expect("wav transcription should succeed");

    assert_eq!(
        *transcoder
            .calls
            .lock()
            .expect("transcoder calls lock poisoned"),
        1,
        "ffmpeg transcoder must be invoked for every input"
    );
}
