use std::sync::Arc;

use sbobino_domain::TranscriptArtifact;

use crate::{ApplicationError, ArtifactQuery, ArtifactRepository};

#[derive(Clone)]
pub struct ArtifactService {
    artifacts: Arc<dyn ArtifactRepository>,
}

impl ArtifactService {
    pub fn new(artifacts: Arc<dyn ArtifactRepository>) -> Self {
        Self { artifacts }
    }

    pub async fn save(&self, artifact: &TranscriptArtifact) -> Result<(), ApplicationError> {
        self.artifacts.save(artifact).await
    }

    pub async fn list(
        &self,
        query: ArtifactQuery,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let limit = query.limit.unwrap_or(50).clamp(1, 500);
        let offset = query.offset.unwrap_or(0);
        self.artifacts
            .list_filtered(query.kind, query.query.as_deref(), limit, offset)
            .await
    }

    pub async fn list_deleted(
        &self,
        query: ArtifactQuery,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let limit = query.limit.unwrap_or(50).clamp(1, 500);
        let offset = query.offset.unwrap_or(0);
        self.artifacts
            .list_deleted(query.kind, query.query.as_deref(), limit, offset)
            .await
    }

    pub async fn get(&self, id: &str) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts.get_by_id(id).await
    }

    pub async fn update_content(
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

    pub async fn update_metadata_entry(
        &self,
        id: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts.update_metadata_entry(id, key, value).await
    }

    pub async fn update_timeline_v2(
        &self,
        id: &str,
        timeline_v2_json: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts
            .update_timeline_v2(id, timeline_v2_json)
            .await
    }

    pub async fn update_diarization_result(
        &self,
        id: &str,
        timeline_v2_json: Option<&str>,
        status: &str,
        error: Option<&str>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts
            .update_diarization_result(id, timeline_v2_json, status, error)
            .await
    }

    pub async fn interrupt_pending_postprocessing_jobs(&self) -> Result<usize, ApplicationError> {
        self.artifacts.interrupt_pending_postprocessing_jobs().await
    }

    pub async fn attach_audio_file(
        &self,
        id: &str,
        source_path: &std::path::Path,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts.attach_audio_file(id, source_path).await
    }

    pub async fn update_emotion_analysis(
        &self,
        id: &str,
        emotion_analysis_json: &str,
        generated_at: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        self.artifacts
            .update_emotion_analysis(id, emotion_analysis_json, generated_at)
            .await
    }

    pub async fn rename(
        &self,
        id: &str,
        new_title: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        if new_title.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "artifact title cannot be empty".to_string(),
            ));
        }
        self.artifacts.rename(id, new_title).await
    }

    pub async fn delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        if ids.is_empty() {
            return Ok(0);
        }
        self.artifacts.delete_many(ids).await
    }

    pub async fn restore_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        if ids.is_empty() {
            return Ok(0);
        }
        self.artifacts.restore_many(ids).await
    }

    pub async fn hard_delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        if ids.is_empty() {
            return Ok(0);
        }
        self.artifacts.hard_delete_many(ids).await
    }

    pub async fn purge_deleted_older_than_days(
        &self,
        days: u32,
    ) -> Result<usize, ApplicationError> {
        self.artifacts.purge_deleted_older_than_days(days).await
    }

    pub async fn read_audio_bytes(&self, id: &str) -> Result<Option<Vec<u8>>, ApplicationError> {
        self.artifacts.read_audio_bytes(id).await
    }
}
