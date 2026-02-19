use std::path::Path;

use async_trait::async_trait;

use sbobino_domain::{AppSettings, ArtifactKind, TranscriptArtifact, WhisperOptions};

use crate::{dto::SummaryFaq, ApplicationError};

#[async_trait]
pub trait AudioTranscoder: Send + Sync {
    async fn to_wav_mono_16k(&self, input: &Path, output: &Path) -> Result<(), ApplicationError>;
}

#[async_trait]
pub trait SpeechToTextEngine: Send + Sync {
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_code: &str,
        options: &WhisperOptions,
        emit_partial: std::sync::Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: std::sync::Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<String, ApplicationError>;
}

#[async_trait]
pub trait TranscriptEnhancer: Send + Sync {
    async fn optimize(&self, text: &str, language_code: &str) -> Result<String, ApplicationError>;
    async fn summarize_and_faq(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError>;
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
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn load(&self) -> Result<AppSettings, ApplicationError>;
    async fn save(&self, settings: &AppSettings) -> Result<(), ApplicationError>;
}
