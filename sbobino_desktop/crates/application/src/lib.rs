pub mod dto;
pub mod error;
pub mod ports;
pub mod services;
pub mod summary_pipeline;

pub use dto::{
    ArtifactQuery, DiarizationProgress, RealtimeDelta, RealtimeDeltaKind, RunTranscriptionRequest,
    SummaryFaq, TranscriptionProgress,
};
pub use error::{is_retryable_ai_provider_error, ApplicationError};
pub use ports::{
    ArtifactRepository, AudioTranscoder, SettingsRepository, SpeakerDiarizationEngine,
    SpeechToTextEngine, TranscriptEnhancer,
};
pub use sbobino_domain::{
    EmotionAnalysisResult, ProgressKind, SpeakerTurn, TimedSegment, TimedWord, TranscriptionOutput,
};
pub use services::{ArtifactService, SettingsService, TranscriptionService};
pub use summary_pipeline::{
    summarize_and_faq_adaptive, summarize_transcript_adaptive, PreparedSummaryContext, SummaryMode,
};
