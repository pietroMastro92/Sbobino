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
    ArtifactKind, ArtifactSourceOrigin, JobProgress, JobStage, LanguageCode, ParakeetModel,
    SpeakerTurn, SpeechModel, TimedSegment, TranscriptArtifact, TranscriptionEngine,
    TranscriptionLanguagePolicy, TranscriptionOutput, WhisperOptions,
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

struct FailingSpeechEngine;

#[async_trait]
impl SpeechToTextEngine for FailingSpeechEngine {
    async fn transcribe(
        &self,
        _input_wav: &Path,
        _model_filename: &str,
        _language_policy: &TranscriptionLanguagePolicy,
        _options: &WhisperOptions,
        _total_audio_seconds: Option<f32>,
        _emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        _emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        Err(ApplicationError::SpeechToText(
            "deterministic engine failure".to_string(),
        ))
    }
}

#[async_trait]
impl SpeechToTextEngine for MockSpeechEngine {
    async fn transcribe(
        &self,
        _input_wav: &Path,
        _model_filename: &str,
        _language_policy: &TranscriptionLanguagePolicy,
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
struct RecordingSpeechEngine {
    calls: Mutex<Vec<RecordingSpeechCall>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordingSpeechCall {
    model_filename: String,
    language_code: String,
}

#[async_trait]
impl SpeechToTextEngine for RecordingSpeechEngine {
    async fn transcribe(
        &self,
        _input_wav: &Path,
        model_filename: &str,
        language_policy: &TranscriptionLanguagePolicy,
        _options: &WhisperOptions,
        _total_audio_seconds: Option<f32>,
        _emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        _emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        self.calls
            .lock()
            .expect("recording speech calls lock poisoned")
            .push(RecordingSpeechCall {
                model_filename: model_filename.to_string(),
                language_code: language_policy.preferred_language.as_code().to_string(),
            });

        Ok(TranscriptionOutput {
            text: "parakeet transcript".to_string(),
            segments: vec![TimedSegment {
                text: "parakeet transcript".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(1.0),
                speaker_id: None,
                speaker_label: None,
                language_code: None,
                language_confidence: None,
                words: Vec::new(),
            }],
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
    optimize_inputs: Mutex<Vec<(String, String)>>,
    summarize_calls: Mutex<usize>,
    summarize_languages: Mutex<Vec<String>>,
    prompts: Mutex<Vec<String>>,
    fail_optimize: bool,
    fail_summarize: bool,
}

#[async_trait]
impl TranscriptEnhancer for MockEnhancer {
    async fn optimize(&self, text: &str, language_code: &str) -> Result<String, ApplicationError> {
        let mut optimize_calls = self
            .optimize_calls
            .lock()
            .expect("enhancer optimize lock poisoned");
        *optimize_calls += 1;
        self.optimize_inputs
            .lock()
            .expect("enhancer optimize inputs lock poisoned")
            .push((text.to_string(), language_code.to_string()));
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
        language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError> {
        let mut summarize_calls = self
            .summarize_calls
            .lock()
            .expect("enhancer summarize lock poisoned");
        *summarize_calls += 1;
        self.summarize_languages
            .lock()
            .expect("enhancer summarize languages lock poisoned")
            .push(language_code.to_string());
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
        self.prompts
            .lock()
            .expect("enhancer prompt lock poisoned")
            .push(prompt.to_string());
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
                parakeet_model: ParakeetModel::default(),
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
            JobStage::PreparingAudio,
            JobStage::Transcribing,
            JobStage::Persisting,
            JobStage::Completed
        ]
    );
    {
        let progress = emitted.lock().expect("emitted lock poisoned");
        assert_eq!(progress[0].overall_percentage, 0);
        assert_eq!(progress[1].overall_percentage, 5);
        assert_eq!(progress[2].overall_percentage, 5);
        assert_eq!(progress[3].overall_percentage, 98);
        assert_eq!(progress[4].overall_percentage, 100);
        assert_eq!(
            progress
                .iter()
                .filter(|event| event.overall_percentage == 100)
                .count(),
            1,
            "a successful job must report overall 100 exactly once"
        );
        assert!(progress
            .windows(2)
            .all(|pair| pair[1].overall_percentage >= pair[0].overall_percentage));
    }

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
async fn run_file_transcription_with_parakeet_uses_gguf_model_and_persists_engine_metadata() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("lezione.m4a");
    tokio::fs::write(&input_path, b"fake m4a content")
        .await
        .expect("failed to create test input file");

    let transcoder = Arc::new(MockTranscoder::default());
    let speech = Arc::new(RecordingSpeechEngine::default());
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());
    let service =
        TranscriptionService::new(transcoder.clone(), speech.clone(), enhancer, repo.clone());

    let emitted: Arc<Mutex<Vec<JobProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-parakeet-001".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::It,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::ParakeetCpp,
                parakeet_model: ParakeetModel::RealtimeEou120mV1F16,
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
        .expect("parakeet transcription service should succeed");

    let calls = speech
        .calls
        .lock()
        .expect("recording speech calls lock poisoned")
        .clone();
    assert_eq!(
        calls,
        vec![RecordingSpeechCall {
            model_filename: "realtime_eou_120m-v1-f16.gguf".to_string(),
            language_code: "it".to_string(),
        }]
    );

    assert_eq!(artifact.raw_transcript, "parakeet transcript");
    assert_eq!(artifact.processing_engine.as_deref(), Some("parakeet_cpp"));
    assert_eq!(
        artifact.processing_model.as_deref(),
        Some("realtime_eou_120m-v1-f16.gguf")
    );
    assert_eq!(
        artifact.metadata.get("model").map(String::as_str),
        Some("realtime_eou_120m-v1-f16.gguf")
    );

    assert!(emitted
        .lock()
        .expect("emitted lock poisoned")
        .iter()
        .any(|event| event.stage == JobStage::Transcribing
            && event.message == "Running Parakeet transcription"));

    let persisted = repo.list_recent(10, 0).await.expect("list should succeed");
    assert_eq!(persisted.len(), 1);
    assert_eq!(
        persisted[0].processing_engine.as_deref(),
        Some("parakeet_cpp")
    );
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
                parakeet_model: ParakeetModel::default(),
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
                parakeet_model: ParakeetModel::default(),
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

    // The MockEnhancer returns "optimized::meeting raw", which is
    // the source plus a 1-token addition. With the relaxed safety
    // net (MAX_CONTEXTUAL_INSERT_TOKENS = 2), small additive topic-
    // aware changes are preserved, so the optimization flows
    // through to the persisted artifact. Summarization then runs
    // against the optimized text, not the raw one.
    assert_eq!(artifact.raw_transcript, "meeting raw");
    assert_eq!(artifact.optimized_transcript, "optimized::meeting raw");
    assert_eq!(
        artifact
            .metadata
            .get(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY)
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(artifact.summary, "summary::optimized::meeting raw");
    assert_eq!(artifact.faqs, "faqs::optimized::meeting raw");

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
                parakeet_model: ParakeetModel::default(),
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
                parakeet_model: ParakeetModel::default(),
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

    let repair: serde_json::Value = serde_json::from_str(
        artifact
            .metadata
            .get("segment_repair_v1")
            .expect("segment repair report should be persisted"),
    )
    .expect("segment repair report should be valid JSON");
    assert_eq!(repair["version"].as_str(), Some("segment_repair_v1"));
    assert_eq!(repair["input_segment_count"].as_u64(), Some(2));
    assert_eq!(repair["output_segment_count"].as_u64(), Some(2));

    let speaker_quality: serde_json::Value = serde_json::from_str(
        artifact
            .metadata
            .get("speaker_quality_v1")
            .expect("speaker quality report should be persisted"),
    )
    .expect("speaker quality report should be valid JSON");
    assert_eq!(
        speaker_quality["version"].as_str(),
        Some("speaker_quality_v1")
    );
    assert_eq!(speaker_quality["warning_count"].as_u64(), Some(0));
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
                parakeet_model: ParakeetModel::default(),
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
async fn automatic_ai_optimizes_contiguous_source_language_groups_separately() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("mixed.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());
    let service = TranscriptionService::new(
        Arc::new(MockTranscoder::default()),
        Arc::new(MockSpeechEngine {
            transcript: "ciao mondo\ncome stai\nhello world".to_string(),
            segments: vec![
                TimedSegment {
                    text: "ciao mondo".to_string(),
                    language_code: Some("it-IT".to_string()),
                    ..TimedSegment::default()
                },
                TimedSegment {
                    text: "come stai".to_string(),
                    language_code: Some("it".to_string()),
                    ..TimedSegment::default()
                },
                TimedSegment {
                    text: "hello world".to_string(),
                    language_code: Some("en-US".to_string()),
                    ..TimedSegment::default()
                },
            ],
        }),
        enhancer.clone(),
        repo,
    );

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-mixed-language".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Small,
                engine: TranscriptionEngine::WhisperCpp,
                parakeet_model: ParakeetModel::default(),
                enable_ai: true,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_| {}),
            CancellationToken::new(),
        )
        .await
        .expect("mixed-language transcription should succeed");

    assert_eq!(
        *enhancer
            .optimize_inputs
            .lock()
            .expect("optimize input lock poisoned"),
        vec![
            ("ciao mondo\ncome stai".to_string(), "it".to_string()),
            ("hello world".to_string(), "en".to_string()),
        ]
    );
    let summary_prompts = enhancer
        .prompts
        .lock()
        .expect("summary prompt lock poisoned");
    assert!(summary_prompts
        .iter()
        .any(|prompt| prompt.contains("Generate in language en:")));
    assert!(!summary_prompts
        .iter()
        .any(|prompt| prompt.contains("Generate in language it:")));
    assert!(!artifact.optimized_transcript.contains("[source_language="));
    assert!(artifact
        .optimized_transcript
        .find("optimized::ciao")
        .is_some());
    assert!(artifact
        .optimized_transcript
        .find("optimized::hello")
        .is_some());
    assert!(
        artifact
            .optimized_transcript
            .find("optimized::ciao")
            .expect("Italian group output")
            < artifact
                .optimized_transcript
                .find("optimized::hello")
                .expect("English group output")
    );
}

#[tokio::test]
async fn automatic_ai_falls_back_to_complete_auto_group_for_partial_segments() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("partial-segments.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create wav file");

    let enhancer = Arc::new(MockEnhancer::default());
    let service = TranscriptionService::new(
        Arc::new(MockTranscoder::default()),
        Arc::new(MockSpeechEngine {
            transcript: "ciao\nhello\nextra".to_string(),
            segments: vec![
                TimedSegment {
                    text: "ciao".to_string(),
                    language_code: Some("it".to_string()),
                    ..TimedSegment::default()
                },
                TimedSegment {
                    text: "hello".to_string(),
                    language_code: Some("en".to_string()),
                    ..TimedSegment::default()
                },
            ],
        }),
        enhancer.clone(),
        Arc::new(InMemoryArtifactRepository::default()),
    );

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-partial-segments".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Small,
                engine: TranscriptionEngine::WhisperCpp,
                parakeet_model: ParakeetModel::default(),
                enable_ai: true,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|_| {}),
            Arc::new(|_| {}),
            CancellationToken::new(),
        )
        .await
        .expect("partial segment transcription should succeed");

    assert_eq!(
        *enhancer
            .optimize_inputs
            .lock()
            .expect("optimize input lock poisoned"),
        vec![("ciao\nhello\nextra".to_string(), "auto".to_string())]
    );
    assert!(artifact.optimized_transcript.contains("extra"));
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
                parakeet_model: ParakeetModel::default(),
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
                parakeet_model: ParakeetModel::default(),
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
                parakeet_model: ParakeetModel::default(),
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
                parakeet_model: ParakeetModel::default(),
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

/// Engine that replays a fixed sequence of progress values (in seconds) to its
/// `emit_progress_seconds` callback, simulating the non-monotonic sequence an
/// engine emits when it resets progress before a CPU-safe retry.
struct ProgressReplayEngine {
    progress_seconds: Vec<f32>,
}

#[async_trait]
impl SpeechToTextEngine for ProgressReplayEngine {
    async fn transcribe(
        &self,
        _input_wav: &Path,
        _model_filename: &str,
        _language_policy: &TranscriptionLanguagePolicy,
        _options: &WhisperOptions,
        _total_audio_seconds: Option<f32>,
        _emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        for seconds in &self.progress_seconds {
            emit_progress_seconds(*seconds);
        }
        Ok(TranscriptionOutput {
            text: "replayed transcript".to_string(),
            segments: Vec::new(),
        })
    }
}

#[tokio::test]
async fn progress_callback_keeps_visible_progress_monotonic_across_retry() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("lecture.mp3");
    tokio::fs::write(&input_path, b"fake mp3 content")
        .await
        .expect("failed to create test input file");

    let transcoder = Arc::new(MockTranscoder::default());
    // Sequence simulating: first attempt advances to 10s, then an internal
    // CPU-safe retry reports local timestamps near zero. Those callbacks are
    // unconfirmed replay work and must not reset the one visible job.
    let speech = Arc::new(ProgressReplayEngine {
        progress_seconds: vec![10.0, 0.0, 1.0, 2.0, 3.0],
    });
    let enhancer = Arc::new(MockEnhancer::default());
    let repo = Arc::new(InMemoryArtifactRepository::default());

    let service =
        TranscriptionService::new(transcoder.clone(), speech, enhancer.clone(), repo.clone());

    let emitted: Arc<Mutex<Vec<JobProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_clone = emitted.clone();

    service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-progress".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                parakeet_model: ParakeetModel::default(),
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

    // Collect only the Transcribing progress events driven by the engine's
    // seconds callback (those carry current_seconds).
    let progress_seconds: Vec<f32> = emitted
        .lock()
        .expect("emitted lock poisoned")
        .iter()
        .filter(|event| event.stage == JobStage::Transcribing)
        .filter_map(|event| event.current_seconds)
        .collect();

    // No callback may move the visible timeline backwards after the first
    // attempt has established a confirmed prefix.
    assert!(
        progress_seconds
            .windows(2)
            .all(|pair| pair[1] + 0.001 >= pair[0]),
        "visible progress must stay monotonic across retry callbacks: {:?}",
        progress_seconds
    );
    assert_eq!(progress_seconds, vec![0.0, 10.0]);

    let events = emitted.lock().expect("emitted lock poisoned");
    assert!(
        events
            .windows(2)
            .all(|pair| pair[1].overall_percentage >= pair[0].overall_percentage),
        "overall percentage must stay monotonic: {:?}",
        events
            .iter()
            .map(|event| (event.stage.clone(), event.overall_percentage))
            .collect::<Vec<_>>()
    );
    assert!(events.iter().all(|event| {
        event.committed_seconds + 0.001 >= event.processed_seconds
            || (event.stage == JobStage::PreparingAudio && event.processed_seconds == 0.0)
    }));
}

#[tokio::test]
async fn terminal_failure_never_reports_overall_completion() {
    let temp = tempdir().expect("failed to create temp dir");
    let input_path = temp.path().join("failure.wav");
    tokio::fs::write(&input_path, b"fake wav content")
        .await
        .expect("failed to create test input file");

    let service = TranscriptionService::new(
        Arc::new(MockTranscoder::default()),
        Arc::new(FailingSpeechEngine),
        Arc::new(MockEnhancer::default()),
        Arc::new(InMemoryArtifactRepository::default()),
    );
    let emitted: Arc<Mutex<Vec<JobProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let emitted_ref = emitted.clone();
    let result = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "job-failure-progress".to_string(),
                input_path: input_path.to_string_lossy().to_string(),
                language: LanguageCode::En,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::WhisperCpp,
                parakeet_model: ParakeetModel::default(),
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: None,
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(move |event| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(event);
            }),
            Arc::new(|_| {}),
            CancellationToken::new(),
        )
        .await;
    assert!(result.is_err());
    let events = emitted.lock().expect("emitted lock poisoned");
    let terminal = events
        .last()
        .expect("failure must emit a terminal progress event");
    assert_eq!(terminal.stage, JobStage::Failed);
    assert!(terminal.overall_percentage < 100);
    assert!(terminal.percentage < 100);
    assert!(events
        .windows(2)
        .all(|pair| pair[1].overall_percentage >= pair[0].overall_percentage));
}
