pub mod artifact;
pub mod emotion_analysis;
pub mod error;
pub mod job;
pub mod quality;
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
pub use quality::{
    repair_segments, speaker_quality_report, QualityReportStatus, SegmentRepairReport,
    SpeakerQualityReport, SpeakerQualityWarning, SpeakerQualityWarningKind,
    SEGMENT_REPAIR_METADATA_KEY, SPEAKER_QUALITY_METADATA_KEY, TIMELINE_MANUAL_EDITS_METADATA_KEY,
};
pub use settings::{
    default_prompt_templates, normalize_automatic_import_tags, transcription_language_catalog,
    whisper_live_model_manifest, AiProvider, AiSettings, AppLanguage, AppSettings, AppearanceMode,
    AutomaticImportActivityEntry, AutomaticImportActivityLevel,
    AutomaticImportPostProcessingSettings, AutomaticImportPreset, AutomaticImportQuarantineItem,
    AutomaticImportSettings, AutomaticImportSource, AutomaticImportSourceHealth,
    AutomaticImportSourceStatus, GeneralSettings, LanguageCode, OrganizationSettings,
    ParakeetModel, PromptBindings, PromptCategory, PromptSettings, PromptTask, PromptTemplate,
    RemoteServiceConfig, RemoteServiceKind, SpeakerDiarizationSettings, SpeechModel,
    TranscriptionComputeDevice, TranscriptionEngine, TranscriptionLanguageOption,
    TranscriptionLanguagePolicy, TranscriptionSettings, WhisperLiveCoreMlEncoderManifest,
    WhisperLiveModelManifest, WhisperOptions, WorkspaceConfig,
};
pub use transcript_cleanup::{
    collapse_consecutive_repeated_segments, constrain_transcript_edit,
    merge_optimized_transcript_sections, minimize_transcript_repetitions,
};
