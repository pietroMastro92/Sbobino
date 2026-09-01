use std::path::Path;

use async_trait::async_trait;

use sbobino_domain::{
    AppSettings, ArtifactChatMessage, ArtifactKind, PersonalizationEntry, SpeakerTurn,
    TranscriptArtifact, TranscriptionLanguagePolicy, TranscriptionOutput, WhisperOptions,
};

use crate::{dto::SummaryFaq, ApplicationError};

#[async_trait]
pub trait AudioTranscoder: Send + Sync {
    async fn to_wav_mono_16k(&self, input: &Path, output: &Path) -> Result<(), ApplicationError>;
}

#[async_trait]
pub trait SpeechToTextEngine: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_policy: &TranscriptionLanguagePolicy,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: std::sync::Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: std::sync::Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError>;
}

#[async_trait]
pub trait SpeakerDiarizationEngine: Send + Sync {
    async fn diarize(&self, input_wav: &Path) -> Result<Vec<SpeakerTurn>, ApplicationError>;
}

#[async_trait]
pub trait TranscriptEnhancer: Send + Sync {
    async fn optimize(&self, text: &str, language_code: &str) -> Result<String, ApplicationError>;
    async fn summarize_and_faq(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError>;

    async fn ask(&self, _prompt: &str) -> Result<String, ApplicationError> {
        Err(ApplicationError::PostProcessing(
            "chat is not supported by the active AI provider".to_string(),
        ))
    }

    fn prefers_single_pass_summary(&self) -> bool {
        false
    }

    fn summary_chunk_concurrency_limit(&self) -> usize {
        3
    }

    fn summary_direct_prompt_char_budget(&self) -> usize {
        14_000
    }

    fn prefers_single_pass_optimize(&self) -> bool {
        false
    }

    fn optimize_chunk_concurrency_limit(&self) -> usize {
        3
    }

    fn optimize_direct_prompt_char_budget(&self) -> usize {
        3_200
    }

    fn emotion_direct_prompt_char_budget(&self) -> usize {
        9_000
    }

    fn telemetry_provider_label(&self) -> &'static str {
        "unknown"
    }
}

#[async_trait]
pub trait ArtifactRepository: Send + Sync {
    async fn save(&self, artifact: &TranscriptArtifact) -> Result<(), ApplicationError>;
    async fn list_recent(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError>;
    async fn list_filtered(
        &self,
        kind: Option<ArtifactKind>,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError>;
    async fn get_by_id(&self, id: &str) -> Result<Option<TranscriptArtifact>, ApplicationError>;
    async fn update_content(
        &self,
        id: &str,
        optimized_transcript: &str,
        summary: &str,
        faqs: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError>;
    async fn update_metadata_entry(
        &self,
        id: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError>;
    async fn apply_artifact_review_update(
        &self,
        _id: &str,
        _expected_revision: i64,
        _optimized_transcript: Option<&str>,
        _review_metadata_json: &str,
        _remembered_correction: Option<&PersonalizationEntry>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        Err(ApplicationError::Persistence(
            "atomic artifact review updates are not supported".to_string(),
        ))
    }
    async fn update_timeline_v2(
        &self,
        id: &str,
        timeline_v2_json: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError>;
    async fn update_emotion_analysis(
        &self,
        id: &str,
        emotion_analysis_json: &str,
        generated_at: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError>;
    async fn rename(
        &self,
        id: &str,
        new_title: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError>;
    async fn list_deleted(
        &self,
        kind: Option<ArtifactKind>,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError>;
    async fn restore_many(&self, ids: &[String]) -> Result<usize, ApplicationError>;
    async fn hard_delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError>;
    async fn purge_deleted_older_than_days(&self, days: u32) -> Result<usize, ApplicationError>;
    async fn delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError>;
    async fn read_audio_bytes(&self, id: &str) -> Result<Option<Vec<u8>>, ApplicationError>;

    async fn append_chat_message(
        &self,
        _message: &ArtifactChatMessage,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Persistence(
            "artifact chat persistence is not supported".to_string(),
        ))
    }

    async fn list_chat_messages(
        &self,
        _artifact_id: &str,
    ) -> Result<Vec<ArtifactChatMessage>, ApplicationError> {
        Ok(Vec::new())
    }

    async fn save_chat_summary(
        &self,
        _artifact_id: &str,
        _summary: &str,
    ) -> Result<(), ApplicationError> {
        Ok(())
    }

    async fn load_chat_summary(
        &self,
        _artifact_id: &str,
    ) -> Result<Option<String>, ApplicationError> {
        Ok(None)
    }

    /// List local vocabulary and correction-memory entries.  The default keeps
    /// lightweight repository test doubles source-compatible while the
    /// SQLite adapter provides the durable implementation.
    async fn list_personalization_entries(
        &self,
    ) -> Result<Vec<PersonalizationEntry>, ApplicationError> {
        Ok(Vec::new())
    }

    async fn upsert_personalization_entry(
        &self,
        _entry: &PersonalizationEntry,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Persistence(
            "personalization persistence is not supported".to_string(),
        ))
    }

    async fn delete_personalization_entry(&self, _id: &str) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn clear_personalization_entries(&self) -> Result<usize, ApplicationError> {
        Ok(0)
    }

    async fn increment_personalization_hit_count(&self, _id: &str) -> Result<(), ApplicationError> {
        Ok(())
    }
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn load(&self) -> Result<AppSettings, ApplicationError>;
    async fn save(&self, settings: &AppSettings) -> Result<(), ApplicationError>;
}
