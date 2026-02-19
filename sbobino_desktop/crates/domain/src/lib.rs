pub mod artifact;
pub mod error;
pub mod job;
pub mod settings;

pub use artifact::{ArtifactKind, TranscriptArtifact};
pub use error::DomainError;
pub use job::{JobProgress, JobStage, JobStatus, TranscriptionJob};
pub use settings::{
    default_prompt_templates, AiProvider, AiSettings, AppSettings, GeneralSettings, LanguageCode,
    PromptBindings, PromptCategory, PromptSettings, PromptTask, PromptTemplate, SpeechModel,
    TranscriptionSettings, WhisperOptions,
};
