#![cfg(unix)]

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use sbobino_application::{
    ApplicationError, ArtifactRepository, RunTranscriptionRequest, TranscriptionService,
};
use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, LanguageCode, ParakeetModel, SpeechModel,
    TranscriptArtifact, TranscriptionEngine, WhisperOptions,
};
use sbobino_infrastructure::adapters::{
    ffmpeg::FfmpegAdapter, noop_enhancer::NoopEnhancer, parakeet_cpp::ParakeetCppEngine,
};

const DEFAULT_REAL_SMOKE_MODEL: &str = "tdt-0.6b-v3-q4_k.gguf";

#[derive(Default)]
struct SmokeArtifactRepository {
    artifacts: Mutex<Vec<TranscriptArtifact>>,
}

#[async_trait]
impl ArtifactRepository for SmokeArtifactRepository {
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
        Ok(self
            .artifacts
            .lock()
            .expect("artifact repo lock poisoned")
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_filtered(
        &self,
        _kind: Option<ArtifactKind>,
        _query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        self.list_recent(limit, offset).await
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Ok(self
            .artifacts
            .lock()
            .expect("artifact repo lock poisoned")
            .iter()
            .find(|artifact| artifact.id == id)
            .cloned())
    }

    async fn update_content(
        &self,
        _id: &str,
        _optimized_transcript: &str,
        _summary: &str,
        _faqs: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Ok(None)
    }

    async fn update_metadata_entry(
        &self,
        _id: &str,
        _key: &str,
        _value: Option<&str>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Ok(None)
    }

    async fn update_timeline_v2(
        &self,
        _id: &str,
        _timeline_v2_json: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Ok(None)
    }

    async fn update_emotion_analysis(
        &self,
        _id: &str,
        _emotion_analysis_json: &str,
        _generated_at: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Ok(None)
    }

    async fn rename(
        &self,
        _id: &str,
        _new_title: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Ok(None)
    }

    async fn list_deleted(
        &self,
        _kind: Option<ArtifactKind>,
        _query: Option<&str>,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        Ok(Vec::new())
    }

    async fn restore_many(&self, _ids: &[String]) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn hard_delete_many(&self, _ids: &[String]) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn purge_deleted_older_than_days(&self, _days: u32) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn delete_many(&self, _ids: &[String]) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn read_audio_bytes(&self, _id: &str) -> Result<Option<Vec<u8>>, ApplicationError> {
        Ok(None)
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must be set for the Parakeet service smoke test");
    })
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn parakeet_model_for_filename(filename: &str) -> ParakeetModel {
    match filename {
        "realtime_eou_120m-v1-f16.gguf" => ParakeetModel::RealtimeEou120mV1F16,
        "realtime_eou_120m-v1-q8_0.gguf" => ParakeetModel::RealtimeEou120mV1Q8,
        "nemotron-3.5-asr-streaming-0.6b-f16.gguf" => ParakeetModel::Nemotron35AsrStreaming06bF16,
        "nemotron-3.5-asr-streaming-0.6b-q4_k.gguf" => ParakeetModel::Nemotron35AsrStreaming06bQ4,
        "nemotron-3.5-asr-streaming-0.6b-q8_0.gguf" => ParakeetModel::Nemotron35AsrStreaming06bQ8,
        "tdt-0.6b-v3-f16.gguf" => ParakeetModel::Tdt06bV3F16,
        "tdt-0.6b-v3-q8_0.gguf" => ParakeetModel::Tdt06bV3Q8,
        "tdt-0.6b-v3-q4_k.gguf" => ParakeetModel::Tdt06bV3Q4,
        other => panic!("unsupported Parakeet model for service smoke: {other}"),
    }
}

fn assert_timeline_has_word_timestamp(artifact: &TranscriptArtifact) {
    let timeline = artifact
        .metadata
        .get("timeline_v2")
        .expect("artifact should persist timeline_v2 metadata");
    let payload: Value = serde_json::from_str(timeline).expect("timeline_v2 should be JSON");
    let segments = payload
        .get("segments")
        .and_then(Value::as_array)
        .expect("timeline_v2 should contain segments");
    assert!(
        !segments.is_empty(),
        "service smoke persisted no timeline segments"
    );

    let has_word_timestamp = segments.iter().any(|segment| {
        segment
            .get("words")
            .and_then(Value::as_array)
            .is_some_and(|words| {
                words.iter().any(|word| {
                    word.get("start_seconds").is_some_and(Value::is_number)
                        || word.get("end_seconds").is_some_and(Value::is_number)
                })
            })
    });
    assert!(
        has_word_timestamp,
        "service smoke persisted no word-level timestamp"
    );
}

fn assert_detected_language_contract(
    artifact: &TranscriptArtifact,
    expected_detected_language: &str,
) {
    assert_eq!(
        artifact
            .metadata
            .get("preferred_language")
            .map(String::as_str),
        Some("auto"),
        "service smoke should preserve automatic language as the requested preference"
    );
    assert_eq!(
        artifact
            .metadata
            .get("language_detection_version")
            .map(String::as_str),
        Some("1"),
        "service smoke should persist the current language-detection contract version"
    );
    let metadata_language = artifact.metadata.get("language").map(String::as_str);
    assert_eq!(
        metadata_language,
        Some(expected_detected_language),
        "service smoke should persist the detected processing language, not the auto preference"
    );
    assert_eq!(
        artifact.processing_language.as_deref(),
        metadata_language,
        "artifact processing language should match persisted detected metadata"
    );

    let detected_languages = artifact
        .metadata
        .get("detected_languages")
        .expect("service smoke should persist detected_languages metadata");
    let detected_languages: Value = serde_json::from_str(detected_languages)
        .expect("detected_languages metadata should be valid JSON");
    let detected_languages = detected_languages
        .as_array()
        .expect("detected_languages metadata should be a JSON array");
    assert!(
        detected_languages.iter().any(|summary| {
            summary.get("code").and_then(Value::as_str) == Some(expected_detected_language)
        }),
        "detected_languages metadata should include {expected_detected_language}, got {detected_languages:?}"
    );
}

#[tokio::test]
#[ignore = "requires real parakeet-cli, GGUF model, ffmpeg, and spoken audio env vars"]
async fn parakeet_service_real_smoke_persists_metadata() {
    let cli_path = required_env("SBOBINO_PARAKEET_CLI");
    let models_dir = required_env("SBOBINO_PARAKEET_MODELS_DIR");
    let audio_path = required_env("SBOBINO_PARAKEET_AUDIO");
    let model_filename =
        optional_env("SBOBINO_PARAKEET_MODEL").unwrap_or_else(|| DEFAULT_REAL_SMOKE_MODEL.into());
    let expected_detected_language = optional_env("SBOBINO_PARAKEET_EXPECTED_DETECTED_LANGUAGE")
        .unwrap_or_else(|| "it".to_string());
    let model = parakeet_model_for_filename(&model_filename);

    assert!(
        Path::new(&cli_path).is_file(),
        "SBOBINO_PARAKEET_CLI must point to an existing file"
    );
    assert!(
        Path::new(&models_dir).join(&model_filename).is_file(),
        "Parakeet model file must exist in SBOBINO_PARAKEET_MODELS_DIR"
    );
    assert!(
        Path::new(&audio_path).is_file(),
        "SBOBINO_PARAKEET_AUDIO must point to an existing audio file"
    );

    let repo = Arc::new(SmokeArtifactRepository::default());
    let emitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let service = TranscriptionService::new(
        Arc::new(FfmpegAdapter::new("ffmpeg".to_string())),
        Arc::new(ParakeetCppEngine::new(cli_path, models_dir)),
        Arc::new(NoopEnhancer),
        repo.clone(),
    );
    let emitted_ref = emitted.clone();

    let artifact = service
        .run_file_transcription(
            RunTranscriptionRequest {
                job_id: "parakeet-real-smoke".to_string(),
                input_path: audio_path,
                language: LanguageCode::Auto,
                model: SpeechModel::Base,
                engine: TranscriptionEngine::ParakeetCpp,
                parakeet_model: model,
                enable_ai: false,
                source_origin: ArtifactSourceOrigin::Imported,
                whisper_options: WhisperOptions::default(),
                title: Some("Parakeet real smoke".to_string()),
                parent_id: None,
                metadata: BTreeMap::new(),
                source_fingerprint_json: None,
            },
            Arc::new(|progress| {
                eprintln!(
                    "service_progress={:?}:{}:{}",
                    progress.stage, progress.percentage, progress.message
                );
            }),
            Arc::new(move |delta| {
                let mut emitted = emitted_ref.lock().expect("emit lock poisoned");
                if emitted.len() < 5 {
                    eprintln!("service_delta={delta}");
                }
                emitted.push(delta);
            }),
            CancellationToken::new(),
        )
        .await
        .expect("service real Parakeet transcription should succeed");

    assert!(
        !artifact.raw_transcript.trim().is_empty(),
        "service smoke produced an empty raw transcript"
    );
    {
        let emitted = emitted.lock().expect("emit lock poisoned");
        let preview_delta_count = emitted
            .iter()
            .filter(|delta| delta.starts_with("\u{001F}REPLACE:"))
            .count();
        assert!(
            preview_delta_count >= 2,
            "expected at least two progressive service deltas before final Parakeet transcript, got {preview_delta_count}"
        );
        assert_eq!(
            emitted.last().map(String::as_str),
            Some(artifact.raw_transcript.as_str()),
            "final service delta should match persisted raw transcript"
        );
    }
    assert_eq!(artifact.processing_engine.as_deref(), Some("parakeet_cpp"));
    assert_eq!(
        artifact.processing_model.as_deref(),
        Some(model_filename.as_str())
    );
    assert_eq!(
        artifact.metadata.get("model").map(String::as_str),
        Some(model_filename.as_str())
    );
    assert_detected_language_contract(&artifact, &expected_detected_language);
    assert_timeline_has_word_timestamp(&artifact);

    let persisted = repo
        .list_recent(10, 0)
        .await
        .expect("service smoke should list persisted artifacts");
    assert_eq!(persisted.len(), 1);
    assert_eq!(
        persisted[0].processing_engine.as_deref(),
        Some("parakeet_cpp")
    );
    assert_eq!(
        persisted[0].processing_model.as_deref(),
        Some(model_filename.as_str())
    );
    assert_detected_language_contract(&persisted[0], &expected_detected_language);
    assert_eq!(
        persisted[0].processing_language, artifact.processing_language,
        "persisted artifact should preserve the detected processing language"
    );
    for metadata_key in [
        "language",
        "preferred_language",
        "language_detection_version",
        "detected_languages",
        "timeline_v2",
    ] {
        assert_eq!(
            persisted[0].metadata.get(metadata_key),
            artifact.metadata.get(metadata_key),
            "persisted artifact should preserve {metadata_key} metadata"
        );
    }
    assert_timeline_has_word_timestamp(&persisted[0]);
}
