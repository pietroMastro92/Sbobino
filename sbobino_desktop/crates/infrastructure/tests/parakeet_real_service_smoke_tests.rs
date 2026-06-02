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

#[tokio::test]
#[ignore = "requires real parakeet-cli, GGUF model, ffmpeg, and spoken audio env vars"]
async fn parakeet_service_real_smoke_persists_metadata() {
    let cli_path = required_env("SBOBINO_PARAKEET_CLI");
    let models_dir = required_env("SBOBINO_PARAKEET_MODELS_DIR");
    let audio_path = required_env("SBOBINO_PARAKEET_AUDIO");
    let model_filename =
        optional_env("SBOBINO_PARAKEET_MODEL").unwrap_or_else(|| DEFAULT_REAL_SMOKE_MODEL.into());
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
    let service = TranscriptionService::new(
        Arc::new(FfmpegAdapter::new("ffmpeg".to_string())),
        Arc::new(ParakeetCppEngine::new(cli_path, models_dir)),
        Arc::new(NoopEnhancer),
        repo.clone(),
    );

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
            Arc::new(|delta| {
                eprintln!("service_delta={delta}");
            }),
            CancellationToken::new(),
        )
        .await
        .expect("service real Parakeet transcription should succeed");

    assert!(
        !artifact.raw_transcript.trim().is_empty(),
        "service smoke produced an empty raw transcript"
    );
    assert_eq!(artifact.processing_engine.as_deref(), Some("parakeet_cpp"));
    assert_eq!(
        artifact.processing_model.as_deref(),
        Some(model_filename.as_str())
    );
    assert_eq!(
        artifact.metadata.get("model").map(String::as_str),
        Some(model_filename.as_str())
    );
    assert_eq!(
        artifact.metadata.get("language").map(String::as_str),
        Some("auto")
    );
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
}
