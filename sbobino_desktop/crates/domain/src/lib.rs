pub mod artifact;
pub mod emotion_analysis;
pub mod error;
pub mod job;
pub mod settings;
pub mod transcript_cleanup;

pub use artifact::{
    ArtifactAudioBackfillStatus, ArtifactChatMessage, ArtifactKind, ArtifactSourceOrigin,
    DetectedLanguageSummary, SpeakerTurn, TimedSegment, TimedWord, TranscriptArtifact,
    TranscriptionOutput,
};
pub use emotion_analysis::{
    EmotionAnalysisResult, EmotionBridge, EmotionOverview, EmotionSemanticCluster,
    EmotionSemanticEdge, EmotionSemanticMap, EmotionSemanticNode, EmotionTimelineEntry,
};
pub use error::DomainError;
pub use job::{JobProgress, JobStage, JobStatus, TranscriptionJob};
pub use settings::{
    default_prompt_templates, transcription_language_catalog, AiProvider, AiSettings, AppLanguage,
    AppSettings, AppearanceMode, AutomaticImportActivityEntry, AutomaticImportActivityLevel,
    AutomaticImportPostProcessingSettings, AutomaticImportPreset, AutomaticImportQuarantineItem,
    AutomaticImportSettings, AutomaticImportSource, AutomaticImportSourceHealth,
    AutomaticImportSourceStatus, GeneralSettings, LanguageCode, OrganizationSettings,
    ParakeetModel, PromptBindings, PromptCategory, PromptSettings, PromptTask, PromptTemplate,
    RemoteServiceConfig, RemoteServiceKind, SpeakerDiarizationSettings, SpeechModel,
    TranscriptionEngine, TranscriptionLanguageOption, TranscriptionLanguagePolicy,
    TranscriptionSettings, WhisperOptions, WorkspaceConfig,
};
pub use transcript_cleanup::{
    collapse_consecutive_repeated_segments, constrain_transcript_edit,
    merge_optimized_transcript_sections, minimize_transcript_repetitions,
};
