pub mod artifact;
pub mod error;
pub mod job;
pub mod settings;
pub mod transcript_cleanup;

pub use artifact::{
    ArtifactKind, SpeakerTurn, TimedSegment, TimedWord, TranscriptArtifact, TranscriptionOutput,
};
pub use error::DomainError;
pub use job::{JobProgress, JobStage, JobStatus, TranscriptionJob};
pub use settings::{
    default_prompt_templates, AiProvider, AiSettings, AppLanguage, AppSettings, AppearanceMode,
    GeneralSettings, LanguageCode, PromptBindings, PromptCategory, PromptSettings, PromptTask,
    PromptTemplate, RemoteServiceConfig, RemoteServiceKind, SpeakerDiarizationSettings,
    SpeechModel, TranscriptionEngine, TranscriptionSettings, WhisperOptions,
};
pub use transcript_cleanup::{
    collapse_consecutive_repeated_segments, constrain_transcript_edit,
    merge_optimized_transcript_sections, minimize_transcript_repetitions,
};
