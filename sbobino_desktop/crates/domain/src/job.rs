use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DomainError, LanguageCode, SpeechModel};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStage {
    Queued,
    PreparingAudio,
    Transcribing,
    Diarizing,
    Optimizing,
    Summarizing,
    Persisting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobProgress {
    pub job_id: String,
    pub stage: JobStage,
    pub message: String,
    pub percentage: u8,
    pub current_seconds: Option<f32>,
    pub total_seconds: Option<f32>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub progress_kind: ProgressKind,
    #[serde(default = "default_progress_attempt")]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_model: Option<String>,
}

fn default_progress_attempt() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProgressKind {
    #[default]
    Actual,
    Estimated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionJob {
    pub id: String,
    pub input_path: String,
    pub language: LanguageCode,
    pub model: SpeechModel,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TranscriptionJob {
    pub fn new(
        input_path: impl Into<String>,
        language: LanguageCode,
        model: SpeechModel,
    ) -> Result<Self, DomainError> {
        let input_path = input_path.into();
        if input_path.trim().is_empty() {
            return Err(DomainError::EmptyInputPath);
        }

        let now = Utc::now();
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            input_path,
            language,
            model,
            status: JobStatus::Queued,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn set_status(&mut self, status: JobStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }
}
