use std::collections::BTreeMap;
use std::fs::File;
use std::future::Future;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use docx_rs::{Docx, Paragraph, Run};
use futures_util::stream::{self, StreamExt};
use printpdf::{
    matrix::TextMatrix, ops::PdfPage, text::TextItem, units::Pt, Color, FontId, Mm, Op, ParsedFont,
    PdfDocument, Rgb,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use sbobino_application::{
    summarize_transcript_adaptive, ApplicationError, ArtifactQuery, TranscriptEnhancer,
    TranscriptionService,
};
use sbobino_domain::{
    constrain_transcript_edit, merge_optimized_transcript_sections,
    minimize_transcript_repetitions, speaker_quality_report, AppLanguage, ArtifactChatMessage,
    ArtifactKind, DetectedLanguageSummary, LanguageCode, PromptTask, TimedSegment, TimedWord,
    TranscriptArtifact, TranscriptionOutput, SPEAKER_QUALITY_METADATA_KEY,
    TIMELINE_MANUAL_EDITS_METADATA_KEY,
};

use crate::{
    ai_support::{missing_ai_provider_command_error, run_with_enhancer_fallback},
    commands::emotion_analysis::{
        analyze_emotions_with_enhancers, EmotionAnalysisInput, EmotionAnalysisOptions,
    },
    commands::prepared_transcript::{
        format_mm_ss, normalize_optional_text, parse_timeline_context_segments,
        parse_timeline_document, ArtifactAiContextOptions, PreparedTranscriptContext,
    },
    error::CommandError,
    state::{AppState, DiarizationTask},
};

const MIN_TRIMMED_AUDIO_DURATION_SECONDS: f64 = 1.5;
const PDF_LEFT_X: f32 = 28.0;
const PDF_TITLE_Y: f32 = 810.0;
const PDF_BODY_START_Y: f32 = 780.0;
const PDF_NEW_PAGE_BODY_START_Y: f32 = 810.0;
const PDF_BOTTOM_Y: f32 = 42.0;
const PDF_LINE_HEIGHT: f32 = 14.0;
const PDF_BODY_MAX_CHARS: usize = 96;
const NOTO_SANS_FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/NotoSans[wdth,wght].ttf");
const SPEAKER_COLOR_PALETTE: &[&str] = &[
    "#4F7CFF", "#EC6A5E", "#27A376", "#D88B15", "#1293A5", "#C85F39", "#5F8D3D", "#B04A64",
    "#6C7A2D",
];

fn default_summary_language() -> String {
    "en".to_string()
}

fn default_summary_sections() -> bool {
    true
}

fn default_summary_action_items() -> bool {
    true
}

fn default_summary_key_points_only() -> bool {
    false
}

fn default_emotion_speaker_dynamics() -> bool {
    true
}

fn app_language_code(language: &AppLanguage) -> &'static str {
    match language {
        AppLanguage::En => "en",
        AppLanguage::It => "it",
        AppLanguage::Es => "es",
        AppLanguage::De => "de",
    }
}

/// Resolve an AI output preference without reanalysing legacy artifacts.  A
/// requested code wins; Auto uses the language metadata already produced by
/// transcription, then the interface language as the final fallback.
fn resolve_ai_output_language(
    artifact: &TranscriptArtifact,
    requested: &str,
    interface_language: &str,
) -> String {
    let requested = requested.trim();
    if !requested.is_empty() && !requested.eq_ignore_ascii_case("auto") {
        return LanguageCode::try_from_code(requested)
            .map(|code| code.as_code().to_string())
            .unwrap_or_else(|_| requested.to_ascii_lowercase());
    }

    if let Some(raw) = artifact.metadata.get("detected_languages") {
        if let Ok(summaries) = serde_json::from_str::<Vec<DetectedLanguageSummary>>(raw) {
            let has_duration = summaries
                .iter()
                .any(|summary| summary.duration_seconds > 0.0);
            if let Some(dominant) = summaries.iter().max_by(|left, right| {
                let left_weight = if has_duration {
                    left.duration_seconds
                } else {
                    left.character_count as f32
                };
                let right_weight = if has_duration {
                    right.duration_seconds
                } else {
                    right.character_count as f32
                };
                left_weight
                    .partial_cmp(&right_weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                return dominant.code.clone();
            }
        }
    }

    if let Some(processing_language) = artifact
        .metadata
        .get("language")
        .map(String::as_str)
        .filter(|language| {
            !language.is_empty()
                && !language.eq_ignore_ascii_case("auto")
                && !language.eq_ignore_ascii_case("mixed")
                && !language.eq_ignore_ascii_case("und")
        })
    {
        return processing_language.to_ascii_lowercase();
    }

    LanguageCode::try_from_code(interface_language)
        .map(|code| code.as_code().to_string())
        .unwrap_or_else(|_| "en".to_string())
}

#[derive(Debug, Deserialize)]
pub struct GetArtifactPayload {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateArtifactPayload {
    pub id: String,
    pub optimized_transcript: String,
    pub summary: String,
    pub faqs: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateArtifactTimelinePayload {
    pub id: String,
    pub timeline_v2: String,
    #[serde(default)]
    pub manual_edit: bool,
}

#[derive(Debug, Deserialize)]
pub struct ArtifactSpeakerDiarizationPayload {
    pub artifact_id: String,
    #[serde(default)]
    pub allow_overwrite_manual_edits: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TimelineManualEditMetadata {
    version: String,
    manual_edit_count: usize,
    last_edited_at: String,
}

#[derive(Debug, Serialize)]
pub struct ArtifactSpeakerDiarizationResponse {
    pub artifact_id: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactSpeakerDiarizationProgressEvent {
    pub artifact_id: String,
    pub state: String,
    pub message: String,
    pub percentage: u8,
}

#[derive(Debug, Deserialize)]
pub struct ChatArtifactPayload {
    pub id: String,
    pub prompt: String,
    #[serde(default = "default_chat_origin")]
    pub origin: String,
    #[serde(flatten)]
    pub context: ArtifactAiContextOptions,
}

fn default_chat_origin() -> String {
    "typed".to_string()
}

#[tauri::command]
pub async fn list_artifact_chat(
    state: State<'_, AppState>,
    payload: GetArtifactPayload,
) -> Result<Vec<ArtifactChatMessage>, CommandError> {
    state
        .artifact_service
        .list_chat_messages(&payload.id)
        .await
        .map_err(CommandError::from)
}

#[derive(Debug, Deserialize)]
pub struct SummarizeArtifactPayload {
    pub id: String,
    #[serde(default = "default_summary_language")]
    pub language: String,
    #[serde(flatten)]
    pub context: ArtifactAiContextOptions,
    #[serde(default = "default_summary_sections")]
    pub sections: bool,
    #[serde(default)]
    pub bullet_points: bool,
    #[serde(default = "default_summary_action_items")]
    pub action_items: bool,
    #[serde(default = "default_summary_key_points_only")]
    pub key_points_only: bool,
    #[serde(default)]
    pub custom_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OptimizeArtifactPayload {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct EmotionAnalysisPayload {
    pub id: String,
    #[serde(default = "default_summary_language")]
    pub language: String,
    #[serde(flatten)]
    pub context: ArtifactAiContextOptions,
    #[serde(default = "default_emotion_speaker_dynamics")]
    pub speaker_dynamics: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedArtifactPackKind {
    StudyPack,
    MeetingIntelligence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedArtifactPack {
    pub kind: GeneratedArtifactPackKind,
    pub generated_at: String,
    pub body_markdown: String,
}

#[derive(Debug, Deserialize)]
pub struct GenerateArtifactPackPayload {
    pub id: String,
    pub kind: GeneratedArtifactPackKind,
    #[serde(default = "default_summary_language")]
    pub language: String,
    #[serde(flatten)]
    pub context: ArtifactAiContextOptions,
}

const CHAT_CONTEXT_BUDGETS: &[(usize, usize)] = &[(8, 7600), (6, 5200), (4, 3400), (2, 2000)];
const CHAT_CHUNK_TARGET_CHARS: usize = 900;
const CHAT_CHUNK_OVERLAP_WORDS: usize = 24;
const OPTIMIZE_CHUNK_TARGET_CHAR_BUDGETS: &[usize] = &[2600, 1800, 1200, 800, 550];
const OPTIMIZE_CHUNK_OVERLAP_WORDS: usize = 28;
const OPTIMIZE_CHUNK_CONCURRENCY_LIMIT: usize = 3;
#[cfg(test)]
const SUMMARY_CHUNK_TARGET_CHARS: usize = 4000;
#[cfg(test)]
const SUMMARY_CHUNK_OVERLAP_WORDS: usize = 30;
#[cfg(test)]
const SUMMARY_CHUNK_CONCURRENCY_LIMIT: usize = 3;
#[cfg(test)]
const SUMMARY_SYNTHESIS_BUDGETS: &[usize] = &[12_000, 8_000, 5_000, 3_000];
const LOW_CONFIDENCE_WORD_THRESHOLD: f32 = 0.58;
const LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD: f32 = 0.72;
const LOW_CONFIDENCE_CONTEXT_RADIUS_WORDS: usize = 3;
const MAX_LOW_CONFIDENCE_PROMPT_SPANS: usize = 10;
const SUMMARY_CONTEXT_OVERFLOW_MESSAGE: &str =
    "Exceeded model context window size. The app now uses chunked retrieval, but this request is still too large. Try a shorter custom prompt or fewer summary constraints.";
const OPTIMIZE_CONTEXT_OVERFLOW_MESSAGE: &str =
    "Exceeded model context window size while optimizing the transcript. The app retried with smaller chunks, but this transcript is still too large for the selected AI provider. Try a provider with a larger context window or optimize a shorter section.";
const STUDY_PACK_METADATA_KEY: &str = "study_pack_v1";
const MEETING_PACK_METADATA_KEY: &str = "meeting_intelligence_v1";

#[derive(Debug, Clone)]
struct LowConfidenceSpan {
    suspect_text: String,
    excerpt: String,
    avg_confidence: f32,
    time_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceLanguageOptimizationGroup {
    language_code: String,
    text: String,
}

#[derive(Debug, Deserialize)]
pub struct ListArtifactsPayload {
    pub kind: Option<ArtifactKind>,
    pub query: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct RenameArtifactPayload {
    pub id: String,
    pub new_title: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteArtifactsPayload {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Txt,
    Docx,
    Html,
    Pdf,
    Json,
    Srt,
    Vtt,
    Csv,
    Md,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportStyle {
    Transcript,
    Subtitles,
    Segments,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExportSegment {
    pub time: String,
    pub line: String,
    #[serde(
        default,
        alias = "startSeconds",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_seconds: Option<f64>,
    #[serde(default, alias = "endSeconds", skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f64>,
    #[serde(default, alias = "speakerId")]
    pub speaker_id: Option<String>,
    #[serde(default, alias = "speakerLabel")]
    pub speaker_label: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportGrouping {
    None,
    SpeakerParagraphs,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExportOptions {
    #[serde(default)]
    pub include_timestamps: bool,
    #[serde(default)]
    pub grouping: Option<ExportGrouping>,
    #[serde(default)]
    pub include_speaker_names: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_timestamps: false,
            grouping: Some(ExportGrouping::None),
            include_speaker_names: false,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ExportArtifactPayload {
    pub id: String,
    pub format: ExportFormat,
    pub destination_path: String,
    pub language: Option<String>,
    pub style: Option<ExportStyle>,
    pub options: Option<ExportOptions>,
    pub segments: Option<Vec<ExportSegment>>,
    pub content_override: Option<String>,
    pub summary_override: Option<String>,
    pub faqs_override: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PreviewArtifactExportPayload {
    pub id: String,
    pub format: ExportFormat,
    pub language: Option<String>,
    pub style: Option<ExportStyle>,
    pub options: Option<ExportOptions>,
    pub segments: Option<Vec<ExportSegment>>,
    pub content_override: Option<String>,
    pub summary_override: Option<String>,
    pub faqs_override: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPreviewMode {
    Exact,
    Document,
}

#[derive(Debug, Serialize)]
pub struct ExportPreviewResponse {
    pub content: String,
    pub mode: ExportPreviewMode,
}

#[derive(Debug, Clone)]
struct ExportDocument {
    title: String,
    sections: Vec<ExportDocumentSection>,
}

#[derive(Debug, Clone)]
struct ExportDocumentSection {
    title: String,
    body: String,
    styled_lines: Option<Vec<ExportStyledLine>>,
}

#[derive(Debug, Clone)]
struct ExportStyledLine {
    text: String,
    speaker_color: Option<String>,
}

#[derive(Debug, Clone)]
struct ExportPreparationInput {
    id: String,
    format: ExportFormat,
    language: Option<String>,
    style: Option<ExportStyle>,
    options: Option<ExportOptions>,
    segments: Option<Vec<ExportSegment>>,
    content_override: Option<String>,
    summary_override: Option<String>,
    faqs_override: Option<String>,
}

#[derive(Debug)]
struct PreparedArtifactExport {
    artifact: TranscriptArtifact,
    format: ExportFormat,
    language: &'static str,
    style: ExportStyle,
    options: ExportOptions,
    grouping: ExportGrouping,
    transcription: String,
    summary: String,
    faqs: String,
    segments: Vec<ExportSegment>,
    content: String,
    document: ExportDocument,
}

impl ExportPreparationInput {
    fn from_export_payload(payload: &ExportArtifactPayload) -> Self {
        Self {
            id: payload.id.clone(),
            format: payload.format,
            language: payload.language.clone(),
            style: payload.style,
            options: payload.options.clone(),
            segments: payload.segments.clone(),
            content_override: payload.content_override.clone(),
            summary_override: payload.summary_override.clone(),
            faqs_override: payload.faqs_override.clone(),
        }
    }

    fn from_preview_payload(payload: &PreviewArtifactExportPayload) -> Self {
        Self {
            id: payload.id.clone(),
            format: payload.format,
            language: payload.language.clone(),
            style: payload.style,
            options: payload.options.clone(),
            segments: payload.segments.clone(),
            content_override: payload.content_override.clone(),
            summary_override: payload.summary_override.clone(),
            faqs_override: payload.faqs_override.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ReadAudioFilePayload {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadArtifactAudioPayload {
    pub artifact_id: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteArtifactsResponse {
    pub deleted: usize,
}

#[derive(Debug, Serialize)]
pub struct RestoreArtifactsResponse {
    pub restored: usize,
}

#[derive(Debug, Serialize)]
pub struct ExportArtifactResponse {
    pub path: String,
}

#[tauri::command]
pub async fn list_recent_artifacts(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<TranscriptArtifact>, CommandError> {
    state
        .artifact_service
        .list(ArtifactQuery {
            kind: None,
            query: None,
            limit,
            offset: Some(0),
        })
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn list_artifacts(
    state: State<'_, AppState>,
    payload: Option<ListArtifactsPayload>,
) -> Result<Vec<TranscriptArtifact>, CommandError> {
    let payload = payload.unwrap_or(ListArtifactsPayload {
        kind: None,
        query: None,
        limit: Some(100),
        offset: Some(0),
    });

    state
        .artifact_service
        .list(ArtifactQuery {
            kind: payload.kind,
            query: payload.query,
            limit: payload.limit,
            offset: payload.offset,
        })
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn list_deleted_artifacts(
    state: State<'_, AppState>,
    payload: Option<ListArtifactsPayload>,
) -> Result<Vec<TranscriptArtifact>, CommandError> {
    let payload = payload.unwrap_or(ListArtifactsPayload {
        kind: None,
        query: None,
        limit: Some(100),
        offset: Some(0),
    });

    state
        .artifact_service
        .purge_deleted_older_than_days(30)
        .await
        .map_err(CommandError::from)?;

    state
        .artifact_service
        .list_deleted(ArtifactQuery {
            kind: payload.kind,
            query: payload.query,
            limit: payload.limit,
            offset: payload.offset,
        })
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_artifact(
    state: State<'_, AppState>,
    payload: GetArtifactPayload,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn update_artifact(
    state: State<'_, AppState>,
    payload: UpdateArtifactPayload,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    state
        .artifact_service
        .update_content(
            &payload.id,
            &payload.optimized_transcript,
            &payload.summary,
            &payload.faqs,
        )
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn update_artifact_timeline(
    state: State<'_, AppState>,
    payload: UpdateArtifactTimelinePayload,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    let Some(mut updated) = state
        .artifact_service
        .update_timeline_v2(&payload.id, &payload.timeline_v2)
        .await
        .map_err(CommandError::from)?
    else {
        return Ok(None);
    };

    if payload.manual_edit {
        let metadata = next_timeline_manual_edit_metadata(
            updated
                .metadata
                .get(TIMELINE_MANUAL_EDITS_METADATA_KEY)
                .map(String::as_str),
        );
        if let Some(metadata_updated) = state
            .artifact_service
            .update_metadata_entry(
                &payload.id,
                TIMELINE_MANUAL_EDITS_METADATA_KEY,
                Some(&metadata),
            )
            .await
            .map_err(CommandError::from)?
        {
            updated = metadata_updated;
        }
    }

    Ok(Some(updated))
}

#[tauri::command]
pub async fn run_artifact_speaker_diarization(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: ArtifactSpeakerDiarizationPayload,
) -> Result<ArtifactSpeakerDiarizationResponse, CommandError> {
    let artifact_id = payload.artifact_id.trim().to_string();
    if artifact_id.is_empty() {
        return Err(CommandError::new(
            "speaker_diarization",
            "artifact id cannot be empty",
        ));
    }

    if !payload.allow_overwrite_manual_edits {
        let has_manual_edits = state
            .artifact_service
            .get(&artifact_id)
            .await
            .map_err(CommandError::from)?
            .as_ref()
            .is_some_and(has_timeline_manual_edits);
        if has_manual_edits {
            return Err(CommandError::new(
                "speaker_diarization",
                "speaker diarization rerun is blocked because the timeline contains manual edits; explicitly allow overwriting manual edits to continue",
            ));
        }
    }

    let run_id = Uuid::new_v4().to_string();
    let cancellation_token = CancellationToken::new();
    {
        let mut registry = state.diarization_tasks.lock().await;
        if registry.contains_key(&artifact_id) {
            return Ok(ArtifactSpeakerDiarizationResponse {
                artifact_id,
                state: "running".to_string(),
            });
        }
        registry.insert(
            artifact_id.clone(),
            DiarizationTask {
                run_id: run_id.clone(),
                cancel_token: cancellation_token.clone(),
            },
        );
    }

    if let Some(updated) =
        set_diarization_metadata(state.inner(), &artifact_id, "running", None, Some(0)).await?
    {
        emit_artifact_updated(&app, &updated);
    }
    emit_diarization_progress(
        &app,
        &artifact_id,
        "running",
        "Preparing speaker diarization",
        0,
    );

    let task_state = state.inner().clone();
    let task_app = app.clone();
    let task_artifact_id = artifact_id.clone();
    tauri::async_runtime::spawn(async move {
        run_artifact_speaker_diarization_task(
            task_app,
            task_state,
            task_artifact_id,
            run_id,
            cancellation_token,
            payload.allow_overwrite_manual_edits,
        )
        .await;
    });

    Ok(ArtifactSpeakerDiarizationResponse {
        artifact_id,
        state: "running".to_string(),
    })
}

#[tauri::command]
pub async fn cancel_artifact_speaker_diarization(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: ArtifactSpeakerDiarizationPayload,
) -> Result<ArtifactSpeakerDiarizationResponse, CommandError> {
    let artifact_id = payload.artifact_id.trim().to_string();
    if artifact_id.is_empty() {
        return Err(CommandError::new(
            "speaker_diarization",
            "artifact id cannot be empty",
        ));
    }

    let task = {
        let mut registry = state.diarization_tasks.lock().await;
        registry.remove(&artifact_id)
    };

    if let Some(task) = task {
        task.cancel_token.cancel();
    }

    if let Some(updated) =
        set_diarization_metadata(state.inner(), &artifact_id, "cancelled", None, Some(100)).await?
    {
        emit_artifact_updated(&app, &updated);
    }
    emit_diarization_progress(
        &app,
        &artifact_id,
        "cancelled",
        "Speaker diarization cancelled",
        100,
    );

    Ok(ArtifactSpeakerDiarizationResponse {
        artifact_id,
        state: "cancelled".to_string(),
    })
}

async fn run_artifact_speaker_diarization_task(
    app: tauri::AppHandle,
    state: AppState,
    artifact_id: String,
    run_id: String,
    cancellation_token: CancellationToken,
    allow_overwrite_manual_edits: bool,
) {
    let result = run_artifact_speaker_diarization_inner(
        &app,
        &state,
        &artifact_id,
        &run_id,
        &cancellation_token,
        allow_overwrite_manual_edits,
    )
    .await;

    match result {
        Ok(()) => {}
        Err(error) if cancellation_token.is_cancelled() => {
            if let Ok(Some(updated)) =
                set_diarization_metadata(&state, &artifact_id, "cancelled", None, Some(100)).await
            {
                emit_artifact_updated(&app, &updated);
            }
            emit_diarization_progress(
                &app,
                &artifact_id,
                "cancelled",
                "Speaker diarization cancelled",
                100,
            );
            tracing::debug!(
                code = %error.code,
                message = %error.message,
                "artifact speaker diarization cancelled"
            );
        }
        Err(error) => {
            let message = error.message;
            if let Ok(Some(updated)) =
                set_diarization_metadata(&state, &artifact_id, "failed", Some(&message), Some(100))
                    .await
            {
                emit_artifact_updated(&app, &updated);
            }
            emit_diarization_progress(&app, &artifact_id, "failed", &message, 100);
        }
    }

    let mut registry = state.diarization_tasks.lock().await;
    let should_remove = registry
        .get(&artifact_id)
        .map(|task| task.run_id == run_id)
        .unwrap_or(false);
    if should_remove {
        registry.remove(&artifact_id);
    }
}

async fn run_artifact_speaker_diarization_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    artifact_id: &str,
    run_id: &str,
    cancellation_token: &CancellationToken,
    allow_overwrite_manual_edits: bool,
) -> Result<(), CommandError> {
    let artifact = state
        .artifact_service
        .get(artifact_id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("speaker_diarization", "transcript not found"))?;
    let segments = timeline_segments_for_diarization(&artifact)?;
    if segments.is_empty() {
        return Err(CommandError::new(
            "speaker_diarization",
            "timeline segments are not available for this transcript",
        ));
    }

    let audio_bytes = state
        .artifact_service
        .read_audio_bytes(artifact_id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| {
            CommandError::new(
                "speaker_diarization",
                "artifact audio is not available for speaker diarization",
            )
        })?;

    if cancellation_token.is_cancelled() {
        return Err(CommandError::from(ApplicationError::Cancelled));
    }

    let source_path = diarization_temp_source_path(&artifact, run_id);
    tokio::fs::write(&source_path, audio_bytes)
        .await
        .map_err(|error| {
            CommandError::new(
                "speaker_diarization",
                format!("failed to write temporary diarization audio: {error}"),
            )
        })?;

    let result = run_artifact_speaker_diarization_from_source(
        app,
        state,
        artifact_id,
        &source_path,
        &segments,
        cancellation_token,
        allow_overwrite_manual_edits,
    )
    .await;

    if let Err(error) = tokio::fs::remove_file(&source_path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                path = %source_path.display(),
                "failed to remove temporary speaker diarization source: {error}"
            );
        }
    }

    result
}

async fn run_artifact_speaker_diarization_from_source(
    app: &tauri::AppHandle,
    state: &AppState,
    artifact_id: &str,
    source_path: &Path,
    segments: &[TimedSegment],
    cancellation_token: &CancellationToken,
    allow_overwrite_manual_edits: bool,
) -> Result<(), CommandError> {
    let Some((transcoder, speaker_diarizer)) = state
        .runtime_factory
        .build_speaker_diarization_runtime()
        .map_err(|error| CommandError::new("speaker_diarization", error))?
    else {
        return Err(CommandError::new(
            "speaker_diarization",
            "Speaker diarization is disabled in transcription settings.",
        ));
    };

    let wav_path = diarization_temp_wav_path(artifact_id);
    emit_diarization_progress(
        app,
        artifact_id,
        "running",
        "Preparing audio for speaker diarization",
        8,
    );
    let result = async {
        run_cancellable(
            cancellation_token,
            transcoder.to_wav_mono_16k(source_path, &wav_path),
        )
        .await?;
        emit_diarization_progress(
            app,
            artifact_id,
            "running",
            "Assigning speakers with pyannote",
            35,
        );
        let turns =
            run_cancellable(cancellation_token, speaker_diarizer.diarize(&wav_path)).await?;
        if cancellation_token.is_cancelled() {
            return Err(ApplicationError::Cancelled);
        }
        emit_diarization_progress(app, artifact_id, "running", "Updating speaker timeline", 90);
        let assigned_segments = TranscriptionService::assign_speakers_to_segments(segments, &turns);
        let speaker_quality_metadata = serde_json::to_string(&speaker_quality_report(
            &assigned_segments,
        ))
        .map_err(|error| ApplicationError::Persistence(error.to_string()))?;
        let timeline_v2 = TranscriptionOutput {
            text: String::new(),
            segments: assigned_segments,
        }
        .timeline_v2_metadata_json();
        if cancellation_token.is_cancelled() {
            return Err(ApplicationError::Cancelled);
        }
        if !allow_overwrite_manual_edits {
            let has_manual_edits = state
                .artifact_service
                .get(artifact_id)
                .await
                .map_err(|error| ApplicationError::Persistence(error.to_string()))?
                .as_ref()
                .is_some_and(has_timeline_manual_edits);
            if has_manual_edits {
                return Err(ApplicationError::Validation(
                    "speaker diarization rerun is blocked because the timeline contains manual edits".to_string(),
                ));
            }
        }
        state
            .artifact_service
            .update_timeline_v2(artifact_id, &timeline_v2)
            .await?;
        state
            .artifact_service
            .update_metadata_entry(
                artifact_id,
                SPEAKER_QUALITY_METADATA_KEY,
                Some(&speaker_quality_metadata),
            )
            .await?;
        if allow_overwrite_manual_edits {
            // The explicit override means the previous manual timeline is no
            // longer the active source. Remove its guard so a later rerun is
            // not blocked by stale provenance.
            state
                .artifact_service
                .update_metadata_entry(artifact_id, TIMELINE_MANUAL_EDITS_METADATA_KEY, None)
                .await?;
        }
        if cancellation_token.is_cancelled() {
            return Err(ApplicationError::Cancelled);
        }
        Ok::<(), ApplicationError>(())
    }
    .await;

    if let Err(error) = tokio::fs::remove_file(&wav_path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                path = %wav_path.display(),
                "failed to remove temporary speaker diarization wav: {error}"
            );
        }
    }

    result.map_err(CommandError::from)?;
    if let Some(updated) =
        set_diarization_metadata(state, artifact_id, "completed", None, Some(100)).await?
    {
        emit_artifact_updated(app, &updated);
    }
    emit_diarization_progress(
        app,
        artifact_id,
        "completed",
        "Speaker diarization completed",
        100,
    );
    Ok(())
}

async fn run_cancellable<T, F>(
    cancellation_token: &CancellationToken,
    operation: F,
) -> Result<T, ApplicationError>
where
    F: Future<Output = Result<T, ApplicationError>>,
{
    tokio::select! {
        _ = cancellation_token.cancelled() => Err(ApplicationError::Cancelled),
        result = operation => result,
    }
}

async fn set_diarization_metadata(
    state: &AppState,
    artifact_id: &str,
    status: &str,
    error: Option<&str>,
    progress: Option<u8>,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    let mut latest = None;
    if let Some(updated) = state
        .artifact_service
        .update_metadata_entry(artifact_id, "speaker_diarization_status", Some(status))
        .await
        .map_err(CommandError::from)?
    {
        latest = Some(updated);
    }
    if let Some(updated) = state
        .artifact_service
        .update_metadata_entry(artifact_id, "speaker_diarization_error", error)
        .await
        .map_err(CommandError::from)?
    {
        latest = Some(updated);
    }
    let progress_string = progress.map(|value| value.to_string());
    if let Some(updated) = state
        .artifact_service
        .update_metadata_entry(
            artifact_id,
            "speaker_diarization_progress",
            progress_string.as_deref(),
        )
        .await
        .map_err(CommandError::from)?
    {
        latest = Some(updated);
    }

    Ok(latest)
}

fn has_timeline_manual_edits(artifact: &TranscriptArtifact) -> bool {
    artifact
        .metadata
        .contains_key(TIMELINE_MANUAL_EDITS_METADATA_KEY)
}

fn next_timeline_manual_edit_metadata(previous: Option<&str>) -> String {
    let previous_count = previous
        .and_then(|value| serde_json::from_str::<TimelineManualEditMetadata>(value).ok())
        .filter(|metadata| metadata.version == TIMELINE_MANUAL_EDITS_METADATA_KEY)
        .map(|metadata| metadata.manual_edit_count)
        .unwrap_or(0);

    serde_json::to_string(&TimelineManualEditMetadata {
        version: TIMELINE_MANUAL_EDITS_METADATA_KEY.to_string(),
        manual_edit_count: previous_count.saturating_add(1),
        last_edited_at: Utc::now().to_rfc3339(),
    })
    .unwrap_or_else(|_| {
        format!(
            r#"{{"version":"{TIMELINE_MANUAL_EDITS_METADATA_KEY}","manual_edit_count":1,"last_edited_at":""}}"#
        )
    })
}

fn emit_artifact_updated(app: &tauri::AppHandle, artifact: &TranscriptArtifact) {
    let _ = app.emit("artifact://updated", artifact);
}

fn emit_diarization_progress(
    app: &tauri::AppHandle,
    artifact_id: &str,
    state: &str,
    message: &str,
    percentage: u8,
) {
    let _ = app.emit(
        "artifact://speaker-diarization-progress",
        ArtifactSpeakerDiarizationProgressEvent {
            artifact_id: artifact_id.to_string(),
            state: state.to_string(),
            message: message.to_string(),
            percentage,
        },
    );
}

fn timeline_segments_for_diarization(
    artifact: &TranscriptArtifact,
) -> Result<Vec<TimedSegment>, CommandError> {
    let parsed = parse_timeline_document(artifact).ok_or_else(|| {
        CommandError::new(
            "speaker_diarization",
            "timeline_v2 metadata is missing or invalid",
        )
    })?;
    Ok(parsed
        .segments
        .into_iter()
        .map(|segment| TimedSegment {
            text: segment.text,
            start_seconds: segment.start_seconds.filter(|value| value.is_finite()),
            end_seconds: segment.end_seconds.filter(|value| value.is_finite()),
            speaker_id: None,
            speaker_label: None,
            language_code: segment.language_code,
            language_confidence: segment.language_confidence,
            words: segment
                .words
                .into_iter()
                .map(|word| TimedWord {
                    text: word.text,
                    start_seconds: word.start_seconds.filter(|value| value.is_finite()),
                    end_seconds: word.end_seconds.filter(|value| value.is_finite()),
                    confidence: word.confidence.filter(|value| value.is_finite()),
                })
                .collect(),
        })
        .collect())
}

fn diarization_temp_source_path(artifact: &TranscriptArtifact, run_id: &str) -> PathBuf {
    let extension = Path::new(&artifact.source_label)
        .extension()
        .and_then(|value| value.to_str())
        .map(sanitize_audio_extension)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "audio".to_string());
    std::env::temp_dir().join(format!(
        "sbobino_diarization_source_{}_{}.{}",
        artifact.id, run_id, extension
    ))
}

fn diarization_temp_wav_path(artifact_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sbobino_diarization_{}_{}.wav",
        artifact_id,
        Uuid::new_v4()
    ))
}

fn sanitize_audio_extension(extension: &str) -> String {
    extension
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
}

#[tauri::command]
pub async fn rename_artifact(
    state: State<'_, AppState>,
    payload: RenameArtifactPayload,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    state
        .artifact_service
        .rename(&payload.id, &payload.new_title)
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn delete_artifacts(
    state: State<'_, AppState>,
    payload: DeleteArtifactsPayload,
) -> Result<DeleteArtifactsResponse, CommandError> {
    let deleted = state
        .artifact_service
        .delete_many(&payload.ids)
        .await
        .map_err(CommandError::from)?;

    Ok(DeleteArtifactsResponse { deleted })
}

#[tauri::command]
pub async fn restore_artifacts(
    state: State<'_, AppState>,
    payload: DeleteArtifactsPayload,
) -> Result<RestoreArtifactsResponse, CommandError> {
    let restored = state
        .artifact_service
        .restore_many(&payload.ids)
        .await
        .map_err(CommandError::from)?;

    Ok(RestoreArtifactsResponse { restored })
}

#[tauri::command]
pub async fn hard_delete_artifacts(
    state: State<'_, AppState>,
    payload: DeleteArtifactsPayload,
) -> Result<DeleteArtifactsResponse, CommandError> {
    let deleted = state
        .artifact_service
        .hard_delete_many(&payload.ids)
        .await
        .map_err(CommandError::from)?;

    Ok(DeleteArtifactsResponse { deleted })
}

#[tauri::command]
pub async fn empty_deleted_artifacts(
    state: State<'_, AppState>,
) -> Result<DeleteArtifactsResponse, CommandError> {
    let mut offset = 0_usize;
    let mut ids = Vec::new();

    loop {
        let page = state
            .artifact_service
            .list_deleted(ArtifactQuery {
                kind: None,
                query: None,
                limit: Some(500),
                offset: Some(offset),
            })
            .await
            .map_err(CommandError::from)?;

        if page.is_empty() {
            break;
        }

        let page_len = page.len();
        ids.extend(page.into_iter().map(|artifact| artifact.id));

        if page_len < 500 {
            break;
        }
        offset += page_len;
    }

    let deleted = state
        .artifact_service
        .hard_delete_many(&ids)
        .await
        .map_err(CommandError::from)?;

    Ok(DeleteArtifactsResponse { deleted })
}

#[tauri::command]
pub async fn export_artifact(
    state: State<'_, AppState>,
    payload: ExportArtifactPayload,
) -> Result<ExportArtifactResponse, CommandError> {
    let destination_path = Path::new(&payload.destination_path);
    let prepared = prepare_artifact_export(
        state.inner(),
        ExportPreparationInput::from_export_payload(&payload),
    )
    .await?;
    write_prepared_artifact_export(destination_path, &prepared)?;

    Ok(ExportArtifactResponse {
        path: destination_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn preview_artifact_export(
    state: State<'_, AppState>,
    payload: PreviewArtifactExportPayload,
) -> Result<ExportPreviewResponse, CommandError> {
    let prepared = prepare_artifact_export(
        state.inner(),
        ExportPreparationInput::from_preview_payload(&payload),
    )
    .await?;
    render_prepared_artifact_preview(&prepared)
}

async fn prepare_artifact_export(
    state: &AppState,
    input: ExportPreparationInput,
) -> Result<PreparedArtifactExport, CommandError> {
    let style = input.style.unwrap_or(ExportStyle::Transcript);
    validate_export_combination(style, input.format)?;

    let artifact = state
        .artifact_service
        .get(&input.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let base_transcription = input
        .content_override
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if artifact.optimized_transcript.trim().is_empty() {
                artifact.raw_transcript.trim().to_string()
            } else {
                artifact.optimized_transcript.trim().to_string()
            }
        });

    if base_transcription.trim().is_empty() {
        return Err(CommandError::new(
            "empty_content",
            "no transcription available to export",
        ));
    }

    let options = input.options.unwrap_or_default();
    let grouping = options.grouping.unwrap_or(ExportGrouping::None);
    let language = normalize_export_language(input.language.as_deref());
    let summary = input
        .summary_override
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| artifact.summary.trim().to_string());
    let faqs = input
        .faqs_override
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| artifact.faqs.trim().to_string());
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let speaker_colors = settings.transcription.speaker_diarization.speaker_colors;
    let segments = match input.segments {
        Some(entries) if !entries.is_empty() => entries,
        Some(_) if input.content_override.is_some() => {
            build_segments_from_text(&base_transcription)
        }
        _ => build_export_segments(&artifact, &base_transcription),
    };
    let export_content = build_export_content(
        &base_transcription,
        &segments,
        style,
        options.include_timestamps,
        options.include_speaker_names,
    );
    let export_document = build_export_document(
        language,
        &artifact.title,
        &base_transcription,
        &summary,
        &faqs,
        &artifact.metadata,
        &segments,
        style,
        options.include_timestamps,
        options.include_speaker_names,
        &speaker_colors,
    );

    Ok(PreparedArtifactExport {
        artifact,
        format: input.format,
        language,
        style,
        options,
        grouping,
        transcription: base_transcription,
        summary,
        faqs,
        segments,
        content: export_content,
        document: export_document,
    })
}

fn validate_export_combination(
    style: ExportStyle,
    format: ExportFormat,
) -> Result<(), CommandError> {
    let supported = match style {
        ExportStyle::Transcript => matches!(
            format,
            ExportFormat::Txt
                | ExportFormat::Docx
                | ExportFormat::Html
                | ExportFormat::Pdf
                | ExportFormat::Md
        ),
        ExportStyle::Subtitles => matches!(format, ExportFormat::Srt | ExportFormat::Vtt),
        ExportStyle::Segments => matches!(
            format,
            ExportFormat::Txt
                | ExportFormat::Csv
                | ExportFormat::Docx
                | ExportFormat::Html
                | ExportFormat::Pdf
                | ExportFormat::Md
                | ExportFormat::Json
        ),
    };

    if supported {
        Ok(())
    } else {
        Err(CommandError::new(
            "invalid_export",
            format!("format {format:?} is not available for style {style:?}"),
        ))
    }
}

fn render_prepared_artifact_preview(
    prepared: &PreparedArtifactExport,
) -> Result<ExportPreviewResponse, CommandError> {
    let (content, mode) = match prepared.format {
        ExportFormat::Txt => (
            render_plain_text_document(&prepared.document),
            ExportPreviewMode::Exact,
        ),
        ExportFormat::Md => (markdown_export_content(prepared), ExportPreviewMode::Exact),
        ExportFormat::Srt => (prepared.content.clone(), ExportPreviewMode::Exact),
        ExportFormat::Vtt => (
            build_vtt_content(
                &prepared.segments,
                &prepared.transcription,
                prepared.options.include_speaker_names,
            ),
            ExportPreviewMode::Exact,
        ),
        ExportFormat::Csv => (
            render_csv_content(
                prepared.language,
                &prepared.segments,
                prepared.options.include_speaker_names,
            ),
            ExportPreviewMode::Exact,
        ),
        ExportFormat::Json => (render_json_content(prepared)?, ExportPreviewMode::Exact),
        ExportFormat::Docx | ExportFormat::Html | ExportFormat::Pdf => (
            render_plain_text_document(&prepared.document),
            ExportPreviewMode::Document,
        ),
    };

    Ok(ExportPreviewResponse { content, mode })
}

fn markdown_export_content(prepared: &PreparedArtifactExport) -> String {
    if prepared.style == ExportStyle::Subtitles {
        build_markdown_subtitles_content(
            &prepared.segments,
            &prepared.transcription,
            prepared.options.include_speaker_names,
        )
    } else {
        render_markdown_document(&prepared.document)
    }
}

fn write_prepared_artifact_export(
    destination_path: &Path,
    prepared: &PreparedArtifactExport,
) -> Result<(), CommandError> {
    match prepared.format {
        ExportFormat::Txt => export_txt(
            destination_path,
            &render_plain_text_document(&prepared.document),
        )?,
        ExportFormat::Docx => export_docx(destination_path, &prepared.document)?,
        ExportFormat::Html => export_html(destination_path, prepared.language, &prepared.document)?,
        ExportFormat::Pdf => export_pdf(destination_path, &prepared.document)?,
        ExportFormat::Json => export_txt(destination_path, &render_json_content(prepared)?)?,
        ExportFormat::Csv => export_txt(
            destination_path,
            &render_csv_content(
                prepared.language,
                &prepared.segments,
                prepared.options.include_speaker_names,
            ),
        )?,
        ExportFormat::Md => export_md(destination_path, &markdown_export_content(prepared))?,
        ExportFormat::Srt => export_txt(destination_path, &prepared.content)?,
        ExportFormat::Vtt => export_txt(
            destination_path,
            &build_vtt_content(
                &prepared.segments,
                &prepared.transcription,
                prepared.options.include_speaker_names,
            ),
        )?,
    }

    Ok(())
}

#[tauri::command]
pub async fn chat_artifact(
    state: State<'_, AppState>,
    payload: ChatArtifactPayload,
) -> Result<String, CommandError> {
    let chat_lock = {
        let mut locks = state.artifact_chat_locks.lock().await;
        if let Some(lock) = locks.get(&payload.id).and_then(std::sync::Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(payload.id.clone(), Arc::downgrade(&lock));
            lock
        }
    };
    let _chat_guard = chat_lock.lock().await;

    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return Err(CommandError::new(
            "validation",
            "chat prompt cannot be empty",
        ));
    }

    let previous_messages = state
        .artifact_service
        .list_chat_messages(&payload.id)
        .await
        .map_err(CommandError::from)?;
    let question = ArtifactChatMessage::new(
        payload.id.clone(),
        "user",
        prompt,
        payload.origin,
        "complete",
    );
    state
        .artifact_service
        .append_chat_message(&question)
        .await
        .map_err(CommandError::from)?;

    let enhancer = match state.runtime_factory.build_enhancer_candidates() {
        Ok(enhancers) if !enhancers.is_empty() => enhancers,
        Ok(_) => {
            let reason = state
                .runtime_factory
                .ai_capability_status()
                .ok()
                .and_then(|status| status.unavailable_reason);
            let error = missing_ai_provider_command_error(reason.as_deref());
            let failed = ArtifactChatMessage::new(
                payload.id,
                "assistant",
                error.message.clone(),
                question.origin,
                "error",
            );
            let _ = state.artifact_service.append_chat_message(&failed).await;
            return Err(error);
        }
        Err(message) => {
            let error = CommandError::new("runtime_factory", message);
            let failed = ArtifactChatMessage::new(
                payload.id,
                "assistant",
                error.message.clone(),
                question.origin,
                "error",
            );
            let _ = state.artifact_service.append_chat_message(&failed).await;
            return Err(error);
        }
    };

    let rolling_summary = build_rolling_chat_summary(&previous_messages);
    if let Some(summary) = rolling_summary.as_deref() {
        let _ = state
            .artifact_service
            .save_chat_summary(&payload.id, summary)
            .await;
    }
    let persisted_summary = match rolling_summary {
        Some(summary) => Some(summary),
        None => state
            .artifact_service
            .load_chat_summary(&payload.id)
            .await
            .unwrap_or(None),
    };
    let candidates = add_conversation_context(
        build_chat_context_candidates(&artifact, prompt, payload.context),
        &previous_messages,
        persisted_summary.as_deref(),
    );
    let result = run_with_enhancer_fallback(&enhancer, "chat", |active_enhancer| {
        let candidates = candidates.clone();
        Box::pin(async move { ask_with_overflow_fallback(active_enhancer, candidates).await })
    })
    .await;

    match result {
        Ok(answer) => {
            let mut response = ArtifactChatMessage::new(
                payload.id,
                "assistant",
                answer.clone(),
                question.origin,
                "complete",
            );
            response.provider = enhancer.first().map(|candidate| candidate.label.clone());
            state
                .artifact_service
                .append_chat_message(&response)
                .await
                .map_err(CommandError::from)?;
            Ok(answer)
        }
        Err(error) => {
            let failed = ArtifactChatMessage::new(
                payload.id,
                "assistant",
                error.to_string(),
                question.origin,
                "error",
            );
            let _ = state.artifact_service.append_chat_message(&failed).await;
            Err(CommandError::from(error))
        }
    }
}

fn add_conversation_context(
    candidates: Vec<String>,
    messages: &[ArtifactChatMessage],
    persisted_summary: Option<&str>,
) -> Vec<String> {
    let completed = completed_chat_turns(messages);
    if completed.is_empty() {
        return candidates;
    }

    let recent_start = completed.len().saturating_sub(12);
    let older = &completed[..recent_start];
    let recent = &completed[recent_start..];
    let older_digest =
        if let Some(summary) = persisted_summary.filter(|value| !value.trim().is_empty()) {
            summary.to_string()
        } else if older.is_empty() {
            String::new()
        } else {
            truncate_chars(
                &older
                    .iter()
                    .map(|message| format!("{}: {}", message.role, message.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
                2400,
            )
        };
    let recent_text = truncate_chars(
        &recent
            .iter()
            .map(|message| format!("{}: {}", message.role, message.text))
            .collect::<Vec<_>>()
            .join("\n"),
        7200,
    );
    let history = format!(
        "Previous conversation summary:\n{older_digest}\n\nRecent conversation turns:\n{recent_text}\n\n"
    );

    candidates
        .into_iter()
        .map(|candidate| {
            candidate.replace("User question:\n", &format!("{history}User question:\n"))
        })
        .collect()
}

fn completed_chat_turns(messages: &[ArtifactChatMessage]) -> Vec<&ArtifactChatMessage> {
    use std::collections::VecDeque;

    let mut completed = Vec::new();
    let mut pending_users = VecDeque::new();
    for message in messages {
        if message.role == "user" {
            if message.status == "complete" {
                pending_users.push_back(message);
            }
            continue;
        }
        if message.role == "assistant" {
            if message.status == "complete" {
                if let Some(user) = pending_users.pop_front() {
                    completed.push(user);
                    completed.push(message);
                }
            } else {
                pending_users.pop_front();
            }
        }
    }
    completed
}

fn build_rolling_chat_summary(messages: &[ArtifactChatMessage]) -> Option<String> {
    let completed = completed_chat_turns(messages);
    if completed.len() <= 12 {
        return None;
    }
    Some(truncate_chars(
        &completed[..completed.len() - 12]
            .iter()
            .map(|message| format!("{}: {}", message.role, message.text))
            .collect::<Vec<_>>()
            .join("\n"),
        2400,
    ))
}

#[tauri::command]
pub async fn optimize_artifact(
    state: State<'_, AppState>,
    payload: OptimizeArtifactPayload,
) -> Result<String, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let text = minimize_transcript_repetitions(payload.text.trim());
    if text.is_empty() {
        return Err(CommandError::new(
            "validation",
            "cannot optimize empty text",
        ));
    }

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let optimization_groups = manual_optimization_groups(&artifact, &text);
    let optimize_prompt_override = build_confidence_aware_optimize_prompt(
        &artifact,
        settings.prompt_for_task(PromptTask::Optimize),
    );

    let enhancers = state
        .runtime_factory
        .build_enhancer_candidates_with_overrides(None, optimize_prompt_override, None)
        .map_err(|e| CommandError::new("runtime_factory", e))?;

    if enhancers.is_empty() {
        let reason = state
            .runtime_factory
            .ai_capability_status()
            .ok()
            .and_then(|status| status.unavailable_reason);
        return Err(missing_ai_provider_command_error(reason.as_deref()));
    }

    run_with_enhancer_fallback(&enhancers, "optimize transcript", |enhancer| {
        let optimization_groups = optimization_groups.clone();
        Box::pin(
            async move { optimize_source_language_groups(enhancer, &optimization_groups).await },
        )
    })
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn summarize_artifact(
    state: State<'_, AppState>,
    payload: SummarizeArtifactPayload,
) -> Result<String, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let output_language = resolve_ai_output_language(
        &artifact,
        &payload.language,
        app_language_code(&settings.general.app_language),
    );

    let enhancers = state
        .runtime_factory
        .build_enhancer_candidates()
        .map_err(|e| CommandError::new("runtime_factory", e))?;
    if enhancers.is_empty() {
        let reason = state
            .runtime_factory
            .ai_capability_status()
            .ok()
            .and_then(|status| status.unavailable_reason);
        return Err(missing_ai_provider_command_error(reason.as_deref()));
    }

    let prepared = PreparedTranscriptContext::from_artifact(&artifact, payload.context);
    if prepared.ai_transcript.trim().is_empty() {
        return Err(CommandError::new(
            "empty_content",
            "no transcription available to summarize",
        ));
    }

    let instructions = build_summary_instructions(&payload, &output_language);

    run_with_enhancer_fallback(&enhancers, "summarize transcript", |enhancer| {
        let transcript = prepared.ai_transcript.clone();
        let instructions = instructions.clone();
        Box::pin(async move {
            summarize_transcript_adaptive(enhancer, &transcript, &instructions).await
        })
    })
    .await
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn generate_artifact_pack(
    state: State<'_, AppState>,
    payload: GenerateArtifactPackPayload,
) -> Result<GeneratedArtifactPack, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let output_language = resolve_ai_output_language(
        &artifact,
        &payload.language,
        app_language_code(&settings.general.app_language),
    );

    let enhancers = state
        .runtime_factory
        .build_enhancer_candidates()
        .map_err(|e| CommandError::new("runtime_factory", e))?;
    if enhancers.is_empty() {
        let reason = state
            .runtime_factory
            .ai_capability_status()
            .ok()
            .and_then(|status| status.unavailable_reason);
        return Err(missing_ai_provider_command_error(reason.as_deref()));
    }

    let prepared = PreparedTranscriptContext::from_artifact(&artifact, payload.context);
    if prepared.ai_transcript.trim().is_empty() {
        return Err(CommandError::new(
            "empty_content",
            "no transcription available to summarize",
        ));
    }

    let instructions = build_generated_pack_instructions(
        payload.kind,
        &output_language,
        payload.context.include_timestamps,
        payload.context.include_speakers,
    );

    let body_markdown =
        run_with_enhancer_fallback(&enhancers, "generate artifact pack", |enhancer| {
            let transcript = prepared.ai_transcript.clone();
            let instructions = instructions.clone();
            Box::pin(async move {
                summarize_transcript_adaptive(enhancer, &transcript, &instructions).await
            })
        })
        .await
        .map_err(CommandError::from)?;

    let generated = GeneratedArtifactPack {
        kind: payload.kind,
        generated_at: Utc::now().to_rfc3339(),
        body_markdown,
    };
    let serialized = serde_json::to_string(&generated)
        .map_err(|e| CommandError::new("serialize_pack", e.to_string()))?;
    state
        .artifact_service
        .update_metadata_entry(
            &payload.id,
            generated_pack_metadata_key(payload.kind),
            Some(&serialized),
        )
        .await
        .map_err(CommandError::from)?;

    Ok(generated)
}

#[tauri::command]
pub async fn analyze_artifact_emotions(
    state: State<'_, AppState>,
    payload: EmotionAnalysisPayload,
) -> Result<sbobino_domain::EmotionAnalysisResult, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let enhancers = state
        .runtime_factory
        .build_enhancer_candidates()
        .map_err(|e| CommandError::new("runtime_factory", e))?;
    if enhancers.is_empty() {
        let reason = state
            .runtime_factory
            .ai_capability_status()
            .ok()
            .and_then(|status| status.unavailable_reason);
        return Err(missing_ai_provider_command_error(reason.as_deref()));
    }

    let prepared = PreparedTranscriptContext::from_artifact(&artifact, payload.context);
    if prepared.ai_transcript.trim().is_empty() {
        return Err(CommandError::new(
            "empty_content",
            "no transcription available to analyze",
        ));
    }

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;

    let result = analyze_emotions_with_enhancers(
        &enhancers,
        EmotionAnalysisInput {
            title: artifact.title.clone(),
            prepared,
        },
        EmotionAnalysisOptions {
            language: payload.language.clone(),
            include_timestamps: payload.context.include_timestamps,
            include_speakers: payload.context.include_speakers,
            speaker_dynamics: payload.speaker_dynamics,
            prompt_override: settings.prompt_for_task(PromptTask::EmotionAnalysis),
        },
    )
    .await
    .map_err(CommandError::from)?;

    let serialized = serde_json::to_string(&result).map_err(|error| {
        CommandError::new(
            "emotion_analysis",
            format!("failed to serialize emotion analysis: {error}"),
        )
    })?;
    state
        .artifact_service
        .update_emotion_analysis(&artifact.id, &serialized, &Utc::now().to_rfc3339())
        .await
        .map_err(CommandError::from)?;

    Ok(result)
}

fn build_artifact_context_transcript(
    artifact: &TranscriptArtifact,
    context: ArtifactAiContextOptions,
) -> String {
    PreparedTranscriptContext::from_artifact(artifact, context).ai_transcript
}

fn normalize_source_language(language: Option<&str>) -> String {
    let Some(language) = language.map(str::trim).filter(|value| !value.is_empty()) else {
        return "auto".to_string();
    };
    if language.eq_ignore_ascii_case("auto") || language.eq_ignore_ascii_case("und") {
        return "auto".to_string();
    }
    LanguageCode::try_from_code(language)
        .map(|code| {
            if code.is_auto() {
                "auto".to_string()
            } else {
                code.as_code().to_string()
            }
        })
        .unwrap_or_else(|_| "auto".to_string())
}

fn contiguous_source_language_groups(
    segments: impl IntoIterator<Item = (Option<String>, String)>,
) -> Vec<SourceLanguageOptimizationGroup> {
    let mut groups = Vec::<SourceLanguageOptimizationGroup>::new();
    for (language, text) in segments {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let language_code = normalize_source_language(language.as_deref());
        if let Some(previous) = groups.last_mut() {
            if previous.language_code == language_code {
                previous.text.push('\n');
                previous.text.push_str(text);
                continue;
            }
        }
        groups.push(SourceLanguageOptimizationGroup {
            language_code,
            text: text.to_string(),
        });
    }
    groups
}

fn normalized_timeline_match_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn manual_optimization_groups(
    artifact: &TranscriptArtifact,
    submitted_text: &str,
) -> Vec<SourceLanguageOptimizationGroup> {
    let submitted_text = submitted_text.trim();
    if submitted_text.is_empty() {
        return Vec::new();
    }

    let timeline_segments = parse_timeline_context_segments(artifact);
    let timeline_text = timeline_segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let timeline_matches = !timeline_segments.is_empty()
        && normalized_timeline_match_text(&timeline_text)
            == normalized_timeline_match_text(submitted_text);
    if !timeline_matches {
        return vec![SourceLanguageOptimizationGroup {
            language_code: "auto".to_string(),
            text: submitted_text.to_string(),
        }];
    }

    contiguous_source_language_groups(
        timeline_segments
            .into_iter()
            .map(|segment| (segment.language_code, segment.text)),
    )
}

fn strip_language_service_markers(value: &str) -> String {
    let mut cleaned = value.to_string();
    while let Some(start) = cleaned.find("[source_language=") {
        let Some(relative_end) = cleaned[start..].find(']') else {
            break;
        };
        let end = start + relative_end + 1;
        cleaned.replace_range(start..end, "");
    }
    cleaned
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

async fn optimize_source_language_groups(
    enhancer: &dyn TranscriptEnhancer,
    groups: &[SourceLanguageOptimizationGroup],
) -> Result<String, ApplicationError> {
    let mut optimized_groups = Vec::with_capacity(groups.len());
    for group in groups {
        let optimized = optimize_with_rag(enhancer, &group.text, &group.language_code).await?;
        let optimized = strip_language_service_markers(&optimized);
        let constrained = constrain_transcript_edit(&group.text, &optimized);
        optimized_groups.push(if constrained.trim().is_empty() {
            group.text.clone()
        } else {
            constrained
        });
    }
    Ok(optimized_groups.join("\n").trim().to_string())
}

fn build_confidence_aware_optimize_prompt(
    artifact: &TranscriptArtifact,
    base_prompt: Option<String>,
) -> Option<String> {
    let low_confidence_spans = extract_low_confidence_spans(artifact);
    let normalized_base_prompt = base_prompt
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if low_confidence_spans.is_empty() {
        return normalized_base_prompt;
    }

    let mut sections = Vec::new();
    if let Some(base_prompt) = normalized_base_prompt {
        sections.push(base_prompt);
    }

    sections.push(
        "Confidence-aware guidance: Whisper provided word-level confidence scores. The suspect spans below are SOFT HINTS about where the speech engine was least sure of itself, so they are the most likely places to need aggressive local repair (ASR mishearings, garbled words, dropped syllables). Treat the highlighted regions as priorities for fixing garbled or misheard words, NOT as a fence around which the rest of the transcript must be left alone. Do not be timid: a transcript optimized by leaving 90% of the original words in place has not been optimized at all. Preserve the original language and the speaker's TONE, register, and level of formality — not the exact phrasing. The speaker's tone stays, but the words themselves should be the ones a careful editor would have chosen, not the ones that happened to come out of the speaker's mouth. You should still apply the same level of substantive syntactic, logical, and contextual cleanup to the rest of the transcript as inside the highlighted regions. Outside these spans, fix grammar, punctuation, sentence structure, false starts, filler, and any clearly garbled words with the same confidence you would inside them. The goal is a fully cleaned transcript, not a mostly-original transcript with edits only in highlighted places. Understand the topic being discussed; when the speaker's wording is ambiguous and the surrounding context makes the intended meaning clear, prefer the clearer wording, and when the speaker's flow of ideas is logically sound but the connection between sentences is implicit, make that connection explicit with a short connective if it improves readability, and when the speaker uses vague references like 'la cosa di cui parlavamo' or colloquial fillers like 'fare casino' and the topic is clear from the surrounding context, replace them with the topic-specific term or a more precise editorial form. If a suspect span is still ambiguous after considering the surrounding context, keep the original wording; do not invent missing facts. Example of the expected level of rewriting (Italian): Input: 'uh allora io dico che è importante capire il problema prima di iniziare a programmare e quindi dobbiamo prima fare una analisi attenta di quello che vogliamo realizzare' Output: 'È importante capire il problema prima di iniziare a programmare. Dobbiamo quindi condurre un'analisi attenta di ciò che vogliamo realizzare.'
Another example of the expected level of rewriting (Italian):
Input: 'il progetto ha avuto successo. il team ha lavorato bene.'
Output: 'Il progetto ha avuto successo perché il team ha lavorato bene.'
Another example of topic-aware rewriting (Italian):
Input: 'allora dobbiamo capire bene la cosa di cui parlavamo prima di iniziare a programmare perche senno facciamo casino'
Output: 'Dobbiamo comprendere a fondo i requisiti del progetto software prima di iniziare a programmare, altrimenti creeremo confusione.'

"
            .to_string(),
    );

    let low_confidence_lines = low_confidence_spans
        .iter()
        .map(|span| {
            let percent = (span.avg_confidence * 100.0).round().clamp(0.0, 100.0) as i32;
            match span.time_label.as_deref() {
                Some(time_label) => format!(
                    "- {percent}% confidence near {time_label}: suspect phrase \"{}\" in context \"{}\"",
                    span.suspect_text, span.excerpt
                ),
                None => format!(
                    "- {percent}% confidence: suspect phrase \"{}\" in context \"{}\"",
                    span.suspect_text, span.excerpt
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    sections.push(format!(
        "Low-confidence suspect spans from the original Whisper transcript (prioritize aggressive local repair in these regions, but still apply the same level of substantive cleanup everywhere else):\n{low_confidence_lines}"
    ));

    Some(sections.join("\n\n"))
}

fn extract_low_confidence_spans(artifact: &TranscriptArtifact) -> Vec<LowConfidenceSpan> {
    let Some(document) = parse_timeline_document(artifact) else {
        return Vec::new();
    };

    let mut spans = Vec::new();
    for segment in document.segments {
        let segment_start = segment.start_seconds.filter(|value| value.is_finite());
        let words: Vec<(String, Option<f32>, Option<f32>)> = segment
            .words
            .into_iter()
            .filter_map(|word| {
                normalize_timeline_word_text(&word.text).map(|text| {
                    (
                        text,
                        word.confidence.filter(|value| value.is_finite()),
                        word.start_seconds
                            .filter(|value| value.is_finite())
                            .or(segment_start),
                    )
                })
            })
            .collect();

        if words.is_empty() {
            continue;
        }

        let mut index = 0usize;
        while index < words.len() {
            let Some(confidence) = words[index].1 else {
                index += 1;
                continue;
            };
            if confidence > LOW_CONFIDENCE_WORD_THRESHOLD {
                index += 1;
                continue;
            }

            let span_start = index;
            let mut span_end = index + 1;
            let mut confidence_total = confidence;
            let mut confidence_count = 1usize;

            while span_end < words.len() {
                let Some(next_confidence) = words[span_end].1 else {
                    break;
                };
                if next_confidence > LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD {
                    break;
                }
                confidence_total += next_confidence;
                confidence_count += 1;
                span_end += 1;
            }

            let context_start = span_start.saturating_sub(LOW_CONFIDENCE_CONTEXT_RADIUS_WORDS);
            let context_end = (span_end + LOW_CONFIDENCE_CONTEXT_RADIUS_WORDS).min(words.len());
            let suspect_text = words[span_start..span_end]
                .iter()
                .map(|(text, _, _)| text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let excerpt = words[context_start..context_end]
                .iter()
                .map(|(text, _, _)| text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let time_label = words[span_start]
                .2
                .map(format_mm_ss)
                .filter(|value| !value.is_empty());

            spans.push(LowConfidenceSpan {
                suspect_text,
                excerpt,
                avg_confidence: confidence_total / confidence_count as f32,
                time_label,
            });

            index = span_end;
        }
    }

    spans.sort_by(|left, right| {
        left.avg_confidence
            .partial_cmp(&right.avg_confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut deduped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for span in spans {
        let key = format!(
            "{}::{}",
            span.suspect_text.to_lowercase(),
            span.excerpt.to_lowercase()
        );
        if !seen.insert(key) {
            continue;
        }
        deduped.push(span);
        if deduped.len() >= MAX_LOW_CONFIDENCE_PROMPT_SPANS {
            break;
        }
    }

    deduped
}

fn normalize_timeline_word_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || is_whisper_control_token(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

fn is_whisper_control_token(token_text: &str) -> bool {
    token_text.starts_with("[_") && token_text.ends_with(']')
}

fn chunk_text_by_words(text: &str, target_chars: usize, overlap_words: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0_usize;

    while start < words.len() {
        let mut end = start;
        let mut chars = 0_usize;

        while end < words.len() {
            let word_len = words[end].chars().count() + usize::from(end > start);
            if end > start && chars + word_len > target_chars {
                break;
            }
            chars += word_len;
            end += 1;
        }

        if end == start {
            end = (start + 1).min(words.len());
        }

        chunks.push(words[start..end].join(" "));

        if end >= words.len() {
            break;
        }

        let mut next_start = end.saturating_sub(overlap_words);
        if next_start <= start {
            next_start = end;
        }
        start = next_start;
    }

    chunks
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tokenize_for_search(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|token| {
            let trimmed = token.trim();
            if trimmed.chars().count() < 3 {
                None
            } else {
                Some(trimmed.to_lowercase())
            }
        })
        .collect()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect::<String>()
}

fn score_chunk(chunk_lower: &str, query_lower: &str, query_tokens: &[String]) -> f32 {
    let mut score = 0.0_f32;
    if !query_lower.is_empty() && chunk_lower.contains(query_lower) {
        score += 4.0;
    }

    for token in query_tokens {
        if chunk_lower.contains(token) {
            score += 1.0;
            score += (chunk_lower.matches(token).take(6).count() as f32) * 0.15;
        }
    }

    score
}

fn build_chat_context_candidates(
    artifact: &TranscriptArtifact,
    prompt: &str,
    context: ArtifactAiContextOptions,
) -> Vec<String> {
    let transcript = build_artifact_context_transcript(artifact, context);
    let normalized_prompt = normalize_whitespace(prompt);
    let query_lower = normalized_prompt.to_lowercase();
    let query_tokens = tokenize_for_search(&normalized_prompt);
    let chunks = chunk_text_by_words(
        &transcript,
        CHAT_CHUNK_TARGET_CHARS,
        CHAT_CHUNK_OVERLAP_WORDS,
    );

    let mut scored: Vec<(usize, f32, String)> = chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let chunk_lower = chunk.to_lowercase();
            let score = score_chunk(&chunk_lower, &query_lower, &query_tokens);
            (index, score, chunk.clone())
        })
        .collect();

    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut selected: Vec<(usize, String)> = scored
        .iter()
        .filter(|(_, score, _)| *score > 0.0)
        .take(10)
        .map(|(index, _, chunk)| (*index, chunk.clone()))
        .collect();

    if selected.is_empty() {
        selected = chunks
            .iter()
            .enumerate()
            .take(4)
            .map(|(index, chunk)| (index, chunk.clone()))
            .collect();
    }

    selected.sort_by_key(|(index, _)| *index);

    CHAT_CONTEXT_BUDGETS
        .iter()
        .map(|(max_chunks, max_chars)| {
            let mut packed = String::new();
            for (idx, chunk) in selected.iter().take(*max_chunks) {
                let line = format!("[{}] {}\n", idx + 1, chunk);
                if packed.chars().count() + line.chars().count() > *max_chars {
                    break;
                }
                packed.push_str(&line);
            }

            if packed.trim().is_empty() {
                packed = truncate_chars(
                    selected
                        .first()
                        .map(|(_, value)| value.as_str())
                        .unwrap_or_default(),
                    *max_chars,
                );
            }

            let summary = truncate_chars(artifact.summary.trim(), 1400);
            let faqs = truncate_chars(artifact.faqs.trim(), 1400);
            let title = artifact.title.trim();
            let timestamp_instruction = if context.include_timestamps {
                "When a relevant snippet includes a timestamp, cite it in the answer."
            } else {
                "Do not mention timestamps unless the user explicitly asks for unavailable timing."
            };
            let speaker_instruction = if context.include_speakers {
                "When speaker labels are present, attribute statements to the relevant speaker."
            } else {
                "Do not infer or invent speaker attributions."
            };

            format!(
                "You are an assistant for transcript analysis.\n\
                 Answer using the provided transcript snippets. If you cannot infer the answer, state what is missing.\n\
                 Reply in the same language as the user's question unless the user explicitly asks for a different language.\n\
                 {timestamp_instruction}\n\
                 {speaker_instruction}\n\n\
                 Artifact title: {title}\n\n\
                 Existing summary:\n{summary}\n\n\
                 Existing FAQs:\n{faqs}\n\n\
                 Transcript snippets:\n{packed}\n\
                 User question:\n{normalized_prompt}"
            )
        })
        .collect()
}

fn build_summary_instructions(payload: &SummarizeArtifactPayload, output_language: &str) -> String {
    let mut lines = vec![
        format!(
            "Write a detailed, self-contained brief in {}.",
            language_display_name(output_language)
        ),
        format!(
            "The entire output must be in {}.",
            language_display_name(output_language)
        ),
        "Produce only the final summary text. Do not add meta-commentary about the summarization process.".to_string(),
        "Assume the reader has not listened to the recording. The summary must stand on its own and preserve the substance of the discussion.".to_string(),
        "Preserve every source-language boundary in the transcript; never merge content across a language change.".to_string(),
    ];

    match (payload.sections, payload.bullet_points) {
        (true, true) => lines.push(
            "Organize the summary into clearly titled sections and use bullet points within sections when they improve clarity."
                .to_string(),
        ),
        (true, false) => lines.push(
            "Organize the summary into clearly titled sections and write each section in polished prose paragraphs."
                .to_string(),
        ),
        (false, true) => lines.push(
            "Write the summary as a single untitled bullet list without section headings."
                .to_string(),
        ),
        (false, false) => lines.push(
            "Write the summary as a single continuous section without headings or bullet lists."
                .to_string(),
        ),
    }

    if payload.key_points_only {
        lines.push(
            "Focus on the most important points, decisions, and takeaways. Omit minor tangents."
                .to_string(),
        );
    } else {
        lines.push(
            "Be thorough and cover all major topics with supporting details, technical explanations, examples, numbers, named entities, and the relationships between ideas."
                .to_string(),
        );
        lines.push(
            "Do not settle for a terse recap: explain what was discussed, why it mattered, and how the different topics connect."
                .to_string(),
        );
    }

    if payload.action_items {
        lines.push(
            "Include a dedicated final section for action items, tasks, decisions, or next steps when they appear in the transcript."
                .to_string(),
        );
    } else {
        lines.push(
            "Do not add a dedicated action-items section. Integrate next steps into the summary only when they are genuinely discussed."
                .to_string(),
        );
    }

    if payload.context.include_timestamps {
        lines.push(
            "Where timestamps are available in the transcript, keep them next to the relevant point."
                .to_string(),
        );
    } else {
        lines.push("Do not include timestamps in the final summary.".to_string());
    }

    if payload.context.include_speakers {
        lines.push(
            "Attribute statements to named speakers when speaker labels are available.".to_string(),
        );
    } else {
        lines.push("Do not include speaker attributions in the final summary.".to_string());
    }

    if let Some(custom_prompt) = payload
        .custom_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "Additional user instructions (apply these unless they conflict with the required language and formatting rules above):\n{custom_prompt}"
        ));
    }

    lines.join("\n\n")
}

fn build_generated_pack_instructions(
    kind: GeneratedArtifactPackKind,
    language: &str,
    include_timestamps: bool,
    include_speakers: bool,
) -> String {
    let language_name = language_display_name(language);
    let timestamp_rule = if include_timestamps {
        "Where timestamps are available in the prepared transcript, keep them next to the relevant item."
    } else {
        "Do not include timestamps in the final output."
    };
    let speaker_rule = if include_speakers {
        "When speaker labels are available, attribute decisions or statements to the relevant speaker."
    } else {
        "Do not invent or infer speaker attributions."
    };

    match kind {
        GeneratedArtifactPackKind::StudyPack => format!(
            "Write the entire output in {language_name}. Produce only markdown.\n\n\
             Build a student study pack from the transcript with these sections in order:\n\
             1. Overview\n\
             2. Structured Notes\n\
             3. Glossary of Key Terms\n\
             4. Probable Exam Questions\n\
             5. Flashcards\n\n\
             Requirements:\n\
             - Stay faithful to the transcript and do not invent facts.\n\
             - Use concise headings and bullet points where helpful.\n\
             - In Glossary, define the most important terms in plain language.\n\
             - In Probable Exam Questions, include short model answers.\n\
             - In Flashcards, format each item as `Q:` followed by `A:`.\n\
             - If the transcript does not support a section, write `Not enough evidence.` under that heading.\n\
             - Preserve every source-language boundary in the transcript; never merge content across a language change.\n\
             - {timestamp_rule}\n\
             - {speaker_rule}"
        ),
        GeneratedArtifactPackKind::MeetingIntelligence => format!(
            "Write the entire output in {language_name}. Produce only markdown.\n\n\
             Build a meeting intelligence pack from the transcript with these sections in order:\n\
             1. Executive Summary\n\
             2. Decisions\n\
             3. Action Items\n\
             4. Open Questions\n\
             5. Risks and Blockers\n\n\
             Requirements:\n\
             - Stay faithful to the transcript and do not invent facts.\n\
             - Where owners or deadlines are explicit, capture them.\n\
             - If an item is uncertain, mark it clearly as tentative.\n\
             - If the transcript does not support a section, write `Not enough evidence.` under that heading.\n\
             - Preserve every source-language boundary in the transcript; never merge content across a language change.\n\
             - {timestamp_rule}\n\
             - {speaker_rule}"
        ),
    }
}

fn language_display_name(language_code: &str) -> &str {
    match language_code.trim() {
        "auto" => "the same language as the transcript",
        "en" => "English",
        "it" => "Italian",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "zh" => "Chinese",
        "ja" => "Japanese",
        _ => "the requested language",
    }
}

fn generated_pack_metadata_key(kind: GeneratedArtifactPackKind) -> &'static str {
    match kind {
        GeneratedArtifactPackKind::StudyPack => STUDY_PACK_METADATA_KEY,
        GeneratedArtifactPackKind::MeetingIntelligence => MEETING_PACK_METADATA_KEY,
    }
}

async fn optimize_with_rag(
    enhancer: &dyn TranscriptEnhancer,
    transcript: &str,
    language_code: &str,
) -> Result<String, ApplicationError> {
    let cleaned = minimize_transcript_repetitions(transcript);
    if cleaned.trim().is_empty() {
        return Err(ApplicationError::Validation(
            "cannot optimize an empty transcript".to_string(),
        ));
    }

    let cleaned_char_count = cleaned.chars().count();
    let direct_prompt_budget = enhancer.optimize_direct_prompt_char_budget();
    let should_try_direct = cleaned_char_count <= direct_prompt_budget
        && (enhancer.prefers_single_pass_optimize()
            || direct_prompt_budget >= OPTIMIZE_CHUNK_TARGET_CHAR_BUDGETS[0]);

    if should_try_direct {
        match enhancer.optimize(&cleaned, language_code).await {
            Ok(optimized) => return Ok(constrain_transcript_edit(&cleaned, &optimized)),
            Err(error) if is_context_window_error(&error) => {}
            Err(error) => return Err(error),
        }
    }

    let concurrency_limit = enhancer
        .optimize_chunk_concurrency_limit()
        .clamp(1, OPTIMIZE_CHUNK_CONCURRENCY_LIMIT);

    for target_chars in OPTIMIZE_CHUNK_TARGET_CHAR_BUDGETS {
        let chunks = chunk_text_by_words(&cleaned, *target_chars, OPTIMIZE_CHUNK_OVERLAP_WORDS);
        if chunks.is_empty() {
            return Err(ApplicationError::Validation(
                "cannot optimize an empty transcript".to_string(),
            ));
        }

        let chunk_concurrency = chunks.len().clamp(1, concurrency_limit);
        let results = stream::iter(chunks)
            .map(|chunk| async move {
                enhancer
                    .optimize(&chunk, language_code)
                    .await
                    .map(|optimized| constrain_transcript_edit(&chunk, &optimized))
            })
            .buffered(chunk_concurrency)
            .collect::<Vec<_>>()
            .await;

        if results
            .iter()
            .any(|result| matches!(result, Err(error) if is_context_window_error(error)))
        {
            continue;
        }

        let current_sections = results.into_iter().collect::<Result<Vec<_>, _>>()?;
        let stitched = merge_optimized_transcript_sections(
            &current_sections,
            (OPTIMIZE_CHUNK_OVERLAP_WORDS / 2).max(4),
        );
        if stitched.trim().is_empty() {
            return Ok(cleaned);
        }

        let reduced = constrain_transcript_edit(&cleaned, &stitched);
        if reduced.trim().is_empty() {
            return Ok(cleaned);
        }
        return Ok(reduced);
    }

    Err(ApplicationError::PostProcessing(
        OPTIMIZE_CONTEXT_OVERFLOW_MESSAGE.to_string(),
    ))
}

#[cfg(test)]
async fn summarize_with_rag(
    enhancer: &dyn TranscriptEnhancer,
    transcript: &str,
    user_instructions: &str,
) -> Result<String, ApplicationError> {
    let chunks = chunk_text_by_words(
        transcript,
        SUMMARY_CHUNK_TARGET_CHARS,
        SUMMARY_CHUNK_OVERLAP_WORDS,
    );

    if chunks.is_empty() {
        return Err(ApplicationError::Validation(
            "cannot summarize an empty transcript".to_string(),
        ));
    }

    if enhancer.prefers_single_pass_summary() {
        match enhancer
            .ask(&build_direct_summary_prompt(transcript, user_instructions))
            .await
        {
            Ok(answer) => {
                let trimmed = answer.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
            Err(error) => {
                if !is_context_window_error(&error) {
                    return Err(error);
                }
            }
        }
    }

    if chunks.len() == 1 {
        return ask_with_overflow_fallback(
            enhancer,
            vec![build_direct_summary_prompt(transcript, user_instructions)],
        )
        .await;
    }

    let total = chunks.len();
    let chunk_concurrency_limit = enhancer
        .summary_chunk_concurrency_limit()
        .clamp(1, SUMMARY_CHUNK_CONCURRENCY_LIMIT);
    let chunk_concurrency = total.clamp(1, chunk_concurrency_limit);
    let chunk_notes = stream::iter(chunks.into_iter().enumerate())
        .map(|(index, chunk)| async move {
            let chunk_prompt =
                build_chunk_note_prompt(index + 1, total, user_instructions, chunk.as_str());
            let note = ask_with_overflow_fallback(
                enhancer,
                vec![
                    chunk_prompt.clone(),
                    truncate_chars(&chunk_prompt, 2600),
                    truncate_chars(&chunk_prompt, 1900),
                ],
            )
            .await?;

            Ok::<String, ApplicationError>(format!("Chunk {} notes:\n{}", index + 1, note.trim()))
        })
        .buffered(chunk_concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    let merged_notes = chunk_notes.join("\n\n");
    let candidates = SUMMARY_SYNTHESIS_BUDGETS
        .iter()
        .map(|budget| {
            let clipped_notes = truncate_chars(&merged_notes, *budget);
            build_summary_synthesis_prompt(&clipped_notes, user_instructions)
        })
        .collect::<Vec<_>>();

    ask_with_overflow_fallback(enhancer, candidates).await
}

#[cfg(test)]
fn build_direct_summary_prompt(transcript: &str, user_instructions: &str) -> String {
    format!(
        "You are writing the final summary of a transcript.\n\n\
         User instructions (follow these exactly — including language, structure, and formatting preferences):\n\
         {user_instructions}\n\n\
         Requirements for the final summary:\n\
         - Produce a dense, polished document — not a terse recap or a sparse outline.\n\
         - Cover all major subjects discussed in the transcript with enough depth that a reader \
         who has not heard the original audio would understand the goals, reasoning, evidence, and outcomes.\n\
         - Preserve specific details that matter: names, numbers, dates, technical terms, examples, constraints, and decisions.\n\
         - Explain how topics relate to one another instead of listing them in isolation.\n\
         - When the transcript is technical, keep the technical content explicit and accurate rather than generalizing it away.\n\
         - If there are debates, alternatives, uncertainties, or tradeoffs, describe them clearly.\n\
         - Maintain logical flow between topics: use transitions and group related ideas together.\n\
         - Respect the user's language, structural, and formatting preferences exactly.\n\
         - Output ONLY the summary text. Do not add meta-commentary or labels like \"Summary:\".\n\n\
         Full transcript:\n{transcript}"
    )
}

#[cfg(test)]
fn build_chunk_note_prompt(
    chunk_index: usize,
    total_chunks: usize,
    user_instructions: &str,
    chunk: &str,
) -> String {
    format!(
        "You are extracting detailed notes from a transcript chunk to support a comprehensive final brief.\n\
         Your goal is to capture ALL substantive content — not just keywords.\n\n\
         User instructions (follow these exactly):\n{user_instructions}\n\n\
         This is chunk {chunk_index}/{total_chunks} of the full transcript.\n\n\
         Extract the following from this chunk:\n\
         - Main topics, subtopics, and arguments discussed, with enough context to understand them\n\
         - Key facts, statistics, names, dates, technical terminology, and specific claims\n\
         - Explanations, reasoning, comparisons, and cause-effect relationships\n\
         - Decisions made, open questions, risks, action items, or next steps mentioned\n\
         - Examples, evidence, or concrete scenarios used by the speakers\n\
         - Any speaker attributions if present\n\n\
         Write thorough, self-contained notes, preferably in short prose bullets or compact paragraphs. \
         Each note should be understandable on its own without the original transcript, and should preserve dense technical detail where present.\n\n\
         Transcript chunk:\n{chunk}"
    )
}

#[cfg(test)]
fn build_summary_synthesis_prompt(chunk_notes: &str, user_instructions: &str) -> String {
    format!(
        "You are writing the final summary of a transcript from the extracted chunk notes below.\n\n\
         User instructions (follow these exactly — including language, structure, and formatting preferences):\n\
         {user_instructions}\n\n\
         Requirements for the final summary:\n\
         - Produce a dense, polished document — not a terse recap or a sparse outline.\n\
         - Cover all major subjects discussed in the transcript with enough depth that a reader \
         who has not heard the original audio would understand the goals, reasoning, evidence, and outcomes.\n\
         - Preserve specific details that matter: names, numbers, dates, technical terms, examples, constraints, and decisions.\n\
         - Explain how topics relate to one another instead of listing them in isolation.\n\
         - When the transcript is technical, keep the technical content explicit and accurate rather than generalizing it away.\n\
         - If there are debates, alternatives, uncertainties, or tradeoffs, describe them clearly.\n\
         - Maintain logical flow between topics: use transitions and group related ideas together.\n\
         - Respect the user's language, structural, and formatting preferences exactly.\n\
         - Output ONLY the summary text. Do not add meta-commentary or labels like \"Summary:\".\n\n\
         Chunk notes:\n{chunk_notes}"
    )
}

fn is_context_window_error(error: &ApplicationError) -> bool {
    match error {
        ApplicationError::PostProcessing(message) => {
            let text = message.to_lowercase();
            text.contains("context window")
                || text.contains("model context window")
                || text.contains("context length")
                || text.contains("prompt is too long")
        }
        _ => false,
    }
}

async fn ask_with_overflow_fallback(
    enhancer: &dyn TranscriptEnhancer,
    candidates: Vec<String>,
) -> Result<String, ApplicationError> {
    ask_with_overflow_fallback_for_operation(enhancer, candidates, SUMMARY_CONTEXT_OVERFLOW_MESSAGE)
        .await
}

async fn ask_with_overflow_fallback_for_operation(
    enhancer: &dyn TranscriptEnhancer,
    candidates: Vec<String>,
    overflow_message: &str,
) -> Result<String, ApplicationError> {
    let mut last_context_error: Option<ApplicationError> = None;

    for candidate in candidates {
        match enhancer.ask(&candidate).await {
            Ok(answer) => {
                let trimmed = answer.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
            Err(error) => {
                if is_context_window_error(&error) {
                    last_context_error = Some(error);
                    continue;
                }
                return Err(error);
            }
        }
    }

    if last_context_error.is_some() {
        return Err(ApplicationError::PostProcessing(
            overflow_message.to_string(),
        ));
    }

    Err(ApplicationError::PostProcessing(
        "empty response from AI provider".to_string(),
    ))
}

#[tauri::command]
pub async fn read_audio_file(payload: ReadAudioFilePayload) -> Result<Vec<u8>, CommandError> {
    tokio::fs::read(&payload.path)
        .await
        .map_err(|e| CommandError::new("audio", format!("failed to read audio file: {e}")))
}

#[tauri::command]
pub async fn read_artifact_audio(
    state: State<'_, AppState>,
    payload: ReadArtifactAudioPayload,
) -> Result<Vec<u8>, CommandError> {
    state
        .artifact_service
        .read_audio_bytes(&payload.artifact_id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("audio", "artifact audio is not available"))
}

#[derive(Debug, Deserialize)]
pub struct TrimRegion {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Deserialize)]
pub struct WriteTrimmedAudioPayload {
    pub artifact_id: Option<String>,
    pub input_path: Option<String>,
    pub regions: Vec<TrimRegion>,
}

#[derive(Debug, Serialize)]
pub struct WriteTrimmedAudioResponse {
    pub path: String,
    pub duration_seconds: f64,
    pub file_size_bytes: u64,
}

#[tauri::command]
pub async fn write_trimmed_audio(
    state: State<'_, AppState>,
    payload: WriteTrimmedAudioPayload,
) -> Result<WriteTrimmedAudioResponse, CommandError> {
    use sbobino_infrastructure::background_process::tokio_background_command;

    if payload.regions.is_empty() {
        return Err(CommandError::new("trim", "no regions selected"));
    }

    // Resolve the bundled ffmpeg binary path
    let settings = state
        .settings_service
        .get()
        .await
        .map_err(|e| CommandError::new("trim", format!("failed to load settings: {e}")))?;
    let ffmpeg_path = state
        .runtime_factory
        .resolve_binary_path(&settings.transcription.ffmpeg_path, "ffmpeg");

    let temp_dir = std::env::temp_dir();
    let source_path = if let Some(artifact_id) = payload.artifact_id.as_deref() {
        let bytes = state
            .artifact_service
            .read_audio_bytes(artifact_id)
            .await
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::new("trim", "artifact audio is not available"))?;
        let temp_input = temp_dir.join(format!("sbobino_trim_source_{artifact_id}.wav"));
        tokio::fs::write(&temp_input, bytes).await.map_err(|e| {
            CommandError::new(
                "trim",
                format!("failed to write temporary trim source: {e}"),
            )
        })?;
        temp_input
    } else {
        let Some(input_path) = payload.input_path.as_deref() else {
            return Err(CommandError::new("trim", "missing input source for trim"));
        };
        let input = Path::new(input_path);
        if !input.exists() {
            return Err(CommandError::new(
                "trim",
                format!("input file not found: {input_path}"),
            ));
        }
        input.to_path_buf()
    };
    let input = source_path.as_path();

    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("trimmed");
    let output_filename = format!("sbobino_trim_{}_{}.wav", stem, Uuid::new_v4());
    let output_path = temp_dir.join(&output_filename);

    let mut sorted_regions = payload.regions;
    sorted_regions.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if sorted_regions.len() == 1 {
        // Single region: direct ffmpeg extraction
        let region = &sorted_regions[0];
        let result = tokio_background_command(&ffmpeg_path)
            .kill_on_drop(true)
            .arg("-y")
            .arg("-i")
            .arg(input)
            .arg("-ss")
            .arg(format!("{:.3}", region.start))
            .arg("-to")
            .arg(format!("{:.3}", region.end))
            .arg("-ar")
            .arg("16000")
            .arg("-ac")
            .arg("1")
            .arg("-c:a")
            .arg("pcm_s16le")
            .arg(&output_path)
            .output()
            .await
            .map_err(|e| CommandError::new("trim", format!("ffmpeg failed to start: {e}")))?;

        if !result.status.success() {
            return Err(CommandError::new(
                "trim",
                format!(
                    "ffmpeg trim failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                ),
            ));
        }
    } else {
        // Multiple regions: extract each, then concatenate
        let mut part_paths = Vec::new();

        for (i, region) in sorted_regions.iter().enumerate() {
            let part_filename = format!("sbobino_part_{}_{}_{}.wav", stem, i, Uuid::new_v4());
            let part_path = temp_dir.join(&part_filename);

            let result = tokio_background_command(&ffmpeg_path)
                .kill_on_drop(true)
                .arg("-y")
                .arg("-i")
                .arg(input)
                .arg("-ss")
                .arg(format!("{:.3}", region.start))
                .arg("-to")
                .arg(format!("{:.3}", region.end))
                .arg("-ar")
                .arg("16000")
                .arg("-ac")
                .arg("1")
                .arg("-c:a")
                .arg("pcm_s16le")
                .arg(&part_path)
                .output()
                .await
                .map_err(|e| CommandError::new("trim", format!("ffmpeg failed to start: {e}")))?;

            if !result.status.success() {
                // Clean up any parts created so far
                for p in &part_paths {
                    let _ = tokio::fs::remove_file(p).await;
                }
                return Err(CommandError::new(
                    "trim",
                    format!(
                        "ffmpeg trim failed on region {}: {}",
                        i,
                        String::from_utf8_lossy(&result.stderr)
                    ),
                ));
            }

            part_paths.push(part_path);
        }

        // Build concat file list
        let concat_filename = format!("sbobino_concat_{}.txt", Uuid::new_v4());
        let concat_path = temp_dir.join(&concat_filename);
        let concat_content: String = part_paths
            .iter()
            .map(|p| format!("file '{}'", p.to_string_lossy().replace('\'', "'\\''")))
            .collect::<Vec<_>>()
            .join("\n");

        tokio::fs::write(&concat_path, &concat_content)
            .await
            .map_err(|e| CommandError::new("trim", format!("failed to write concat list: {e}")))?;

        let result = tokio_background_command(&ffmpeg_path)
            .kill_on_drop(true)
            .arg("-y")
            .arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(&concat_path)
            .arg("-c")
            .arg("copy")
            .arg(&output_path)
            .output()
            .await
            .map_err(|e| {
                CommandError::new("trim", format!("ffmpeg concat failed to start: {e}"))
            })?;

        // Clean up temp files
        let _ = tokio::fs::remove_file(&concat_path).await;
        for p in &part_paths {
            let _ = tokio::fs::remove_file(p).await;
        }

        if !result.status.success() {
            return Err(CommandError::new(
                "trim",
                format!(
                    "ffmpeg concat failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                ),
            ));
        }
    }

    let (duration_seconds, file_size_bytes) = trimmed_audio_output_metadata(&output_path)?;
    validate_trimmed_audio_output(duration_seconds, file_size_bytes)?;

    Ok(WriteTrimmedAudioResponse {
        path: output_path.to_string_lossy().to_string(),
        duration_seconds,
        file_size_bytes,
    })
}

fn trimmed_audio_output_metadata(path: &Path) -> Result<(f64, u64), CommandError> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| CommandError::new("trim", format!("failed to inspect trimmed audio: {e}")))?;
    let file_size_bytes = metadata.len();

    let reader = hound::WavReader::open(path)
        .map_err(|e| CommandError::new("trim", format!("trimmed audio is unreadable: {e}")))?;
    let spec = reader.spec();
    if spec.sample_rate == 0 {
        return Err(CommandError::new(
            "trim",
            "trimmed audio has invalid sample rate".to_string(),
        ));
    }

    let duration_seconds = f64::from(reader.duration()) / f64::from(spec.sample_rate);
    Ok((duration_seconds, file_size_bytes))
}

fn validate_trimmed_audio_output(
    duration_seconds: f64,
    file_size_bytes: u64,
) -> Result<(), CommandError> {
    if file_size_bytes == 0 {
        return Err(CommandError::new(
            "trim",
            "trimmed audio file is empty".to_string(),
        ));
    }

    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return Err(CommandError::new(
            "trim",
            "trimmed audio duration is invalid".to_string(),
        ));
    }

    if duration_seconds < MIN_TRIMMED_AUDIO_DURATION_SECONDS {
        return Err(CommandError::new(
            "trim",
            format!(
                "trimmed audio is too short ({duration_seconds:.2}s). Select at least {:.1}s before retranscribing.",
                MIN_TRIMMED_AUDIO_DURATION_SECONDS,
            ),
        ));
    }

    Ok(())
}

fn export_txt(path: &Path, transcription: &str) -> Result<(), CommandError> {
    std::fs::write(path, transcription)
        .map_err(|e| CommandError::new("export", format!("failed to export txt: {e}")))
}

fn export_md(path: &Path, content: &str) -> Result<(), CommandError> {
    std::fs::write(path, content)
        .map_err(|e| CommandError::new("export", format!("failed to export markdown: {e}")))
}

fn normalized_export_speaker_label(segment: &ExportSegment) -> Option<&str> {
    segment
        .speaker_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn normalized_export_speaker_id(segment: &ExportSegment) -> Option<String> {
    normalize_optional_text(segment.speaker_id.clone())
        .map(normalize_speaker_color_key)
        .or_else(|| normalized_export_speaker_label(segment).map(normalize_speaker_color_key))
}

fn normalize_speaker_color_key(value: impl AsRef<str>) -> String {
    let candidate = value
        .as_ref()
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    let normalized = candidate.trim_matches('_').to_string();
    if normalized.is_empty() {
        "speaker".to_string()
    } else {
        normalized
    }
}

fn sanitize_speaker_color_value(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    if trimmed.len() != 7 || !trimmed.starts_with('#') {
        return None;
    }
    if !trimmed
        .chars()
        .skip(1)
        .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    Some(trimmed.to_ascii_uppercase())
}

fn default_speaker_color_for_key(key: &str) -> String {
    let hash = key.bytes().fold(0_u64, |accumulator, value| {
        accumulator.wrapping_mul(31).wrapping_add(value as u64)
    });
    SPEAKER_COLOR_PALETTE[(hash as usize) % SPEAKER_COLOR_PALETTE.len()].to_string()
}

fn resolve_export_speaker_color(
    segment: &ExportSegment,
    speaker_colors: &BTreeMap<String, String>,
) -> Option<String> {
    let speaker_key = normalized_export_speaker_id(segment)?;
    if let Some(color) = speaker_colors
        .get(&speaker_key)
        .and_then(sanitize_speaker_color_value)
    {
        return Some(color);
    }

    Some(default_speaker_color_for_key(&speaker_key))
}

fn parse_hex_rgb(color: &str) -> Option<(u8, u8, u8)> {
    let normalized = sanitize_speaker_color_value(color)?;
    let value = u32::from_str_radix(&normalized[1..], 16).ok()?;
    Some((
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ))
}

fn render_export_segment_line(segment: &ExportSegment, include_speaker_names: bool) -> String {
    let line = segment.line.trim();
    if !include_speaker_names {
        return line.to_string();
    }
    match normalized_export_speaker_label(segment) {
        Some(speaker_label) => format!("{speaker_label}: {line}"),
        None => line.to_string(),
    }
}

fn render_csv_content(
    language: &str,
    segments: &[ExportSegment],
    include_speaker_names: bool,
) -> String {
    let header = localized_export_csv_header(language, include_speaker_names);
    let rows = if segments.is_empty() {
        vec![if include_speaker_names {
            "00:00;00:00;\"\";\"\"".to_string()
        } else {
            "00:00;00:00;\"\"".to_string()
        }]
    } else {
        let timings = resolve_segment_timings(segments);
        segments
            .iter()
            .zip(timings)
            .map(|(segment, timing)| {
                let base = format!(
                    "{};{};{}",
                    format_mm_ss_millis(timing.start_millis),
                    format_mm_ss_millis(timing.end_millis),
                    quote_csv_cell(segment.line.trim()),
                );
                if !include_speaker_names {
                    return base;
                }
                let speaker = normalized_export_speaker_label(segment).unwrap_or_default();
                format!("{base};{}", quote_csv_cell(speaker))
            })
            .collect::<Vec<_>>()
    };

    format!("{header}\n{}", rows.join("\n"))
}

fn quote_csv_cell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn export_docx(path: &Path, document: &ExportDocument) -> Result<(), CommandError> {
    let mut doc = Docx::new()
        .add_paragraph(Paragraph::new().add_run(Run::new().add_text(&document.title)))
        .add_paragraph(Paragraph::new());

    for (index, section) in document.sections.iter().enumerate() {
        if index > 0 {
            doc = doc.add_paragraph(Paragraph::new());
        }

        doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(&section.title)));

        if let Some(styled_lines) = &section.styled_lines {
            for line in styled_lines {
                let mut run = Run::new().add_text(&line.text);
                if let Some(color) = line
                    .speaker_color
                    .as_deref()
                    .and_then(sanitize_speaker_color_value)
                {
                    run = run.color(color.trim_start_matches('#'));
                }
                doc = doc.add_paragraph(Paragraph::new().add_run(run));
            }
        } else {
            for line in section.body.lines() {
                doc = doc.add_paragraph(Paragraph::new().add_run(Run::new().add_text(line)));
            }
        }
    }

    let file = File::create(path)
        .map_err(|e| CommandError::new("export", format!("failed to create docx file: {e}")))?;

    doc.build()
        .pack(file)
        .map_err(|e| CommandError::new("export", format!("failed to write docx: {e}")))
}

fn export_html(path: &Path, language: &str, document: &ExportDocument) -> Result<(), CommandError> {
    let escaped_title = escape_html(&document.title);
    let sections_html = document
        .sections
        .iter()
        .map(|section| {
            let content_html = if let Some(styled_lines) = &section.styled_lines {
                styled_lines
                    .iter()
                    .map(|line| match line.speaker_color.as_deref() {
                        Some(color) => format!(
                            "<span style=\"color:{}\">{}</span>",
                            escape_html(color),
                            escape_html(&line.text)
                        ),
                        None => escape_html(&line.text),
                    })
                    .collect::<Vec<_>>()
                    .join("<br/>\n")
            } else {
                escape_html(&section.body).replace('\n', "<br/>\n")
            };
            format!(
                "<section class=\"section\"><h2>{}</h2><div class=\"content\">{}</div></section>",
                escape_html(&section.title),
                content_html
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let html = format!(
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\" />\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n<title>{}</title>\n<style>\nbody{{font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:2rem;color:#1f2430;background:#f8fafc;}}\nmain{{max-width:880px;margin:0 auto;padding:1.5rem 1.75rem;background:#fff;border:1px solid #dbe2ee;border-radius:14px;}}\nh1{{font-size:1.35rem;margin:0 0 1rem;}}\n.section + .section{{margin-top:1.75rem;padding-top:1.25rem;border-top:1px solid #e2e8f0;}}\nh2{{font-size:1rem;margin:0 0 0.75rem;}}\n.content{{line-height:1.6;font-size:1rem;word-break:break-word;}}\n</style>\n</head>\n<body>\n<main>\n<h1>{}</h1>\n{}\n</main>\n</body>\n</html>\n",
        language, escaped_title, escaped_title, sections_html
    );

    std::fs::write(path, html)
        .map_err(|e| CommandError::new("export", format!("failed to export html: {e}")))
}

fn render_json_content(prepared: &PreparedArtifactExport) -> Result<String, CommandError> {
    let serialized_segments =
        resolved_export_segments(&prepared.segments, prepared.options.include_speaker_names);
    let artifact = &prepared.artifact;
    let payload = json!({
        "id": artifact.id,
        "job_id": artifact.job_id,
        "title": artifact.title,
        "kind": artifact.kind.as_str(),
        "source_label": artifact.source_label,
        "source_origin": artifact.source_origin,
        "created_at": artifact.created_at.to_rfc3339(),
        "updated_at": artifact.updated_at.to_rfc3339(),
        "style": prepared.style,
        "options": {
            "include_timestamps": prepared.options.include_timestamps,
            "grouping": prepared.grouping,
            "include_speaker_names": prepared.options.include_speaker_names
        },
        "document_title": prepared.document.title,
        "sections": prepared.document.sections.iter().map(|section| {
            json!({
                "title": section.title,
                "content": section.body,
            })
        }).collect::<Vec<_>>(),
        "content": prepared.content,
        "summary": prepared.summary,
        "faqs": prepared.faqs,
        "segments": serialized_segments,
        "metadata": artifact.metadata,
    });

    serde_json::to_string_pretty(&payload)
        .map_err(|e| CommandError::new("export", format!("failed to encode json export: {e}")))
}

fn export_pdf(path: &Path, document: &ExportDocument) -> Result<(), CommandError> {
    let mut doc = PdfDocument::new(&document.title);
    let font = load_pdf_font(&mut doc)?;
    let mut pages = Vec::new();
    let (mut ops, mut y) = start_pdf_page_ops(Some(&document.title), &font);
    let colored_lines = render_document_body_styled_lines(document);

    if colored_lines.is_empty() {
        write_pdf_line(&mut ops, "No content available for export.", y, None, &font);
    } else {
        for line in colored_lines {
            for wrapped_line in wrap_pdf_text_line(&line.text, PDF_BODY_MAX_CHARS) {
                if y < PDF_BOTTOM_Y {
                    ops.push(Op::EndTextSection);
                    pages.push(PdfPage::new(Mm(210.0), Mm(297.0), ops));
                    (ops, y) = start_pdf_page_ops(None, &font);
                }

                write_pdf_line(
                    &mut ops,
                    &wrapped_line,
                    y,
                    line.speaker_color.as_deref(),
                    &font,
                );
                y -= PDF_LINE_HEIGHT;
            }
        }
    }

    ops.push(Op::EndTextSection);
    pages.push(PdfPage::new(Mm(210.0), Mm(297.0), ops));
    doc.with_pages(pages);

    let mut warnings = Vec::new();
    let bytes = doc.save(
        &printpdf::PdfSaveOptions {
            optimize: true,
            ..Default::default()
        },
        &mut warnings,
    );

    let mut writer = BufWriter::new(
        File::create(path)
            .map_err(|e| CommandError::new("export", format!("failed to create pdf file: {e}")))?,
    );

    std::io::Write::write_all(&mut writer, &bytes)
        .map_err(|e| CommandError::new("export", format!("failed to write pdf: {e}")))
}

fn load_pdf_font(doc: &mut PdfDocument) -> Result<FontId, CommandError> {
    let mut warnings = Vec::new();
    let font = ParsedFont::from_bytes(NOTO_SANS_FONT_BYTES, 0, &mut warnings).ok_or_else(|| {
        CommandError::new(
            "export",
            "failed to parse the bundled Noto Sans font for PDF export",
        )
    })?;
    Ok(doc.add_font(&font))
}

fn start_pdf_page_ops(title: Option<&str>, font: &FontId) -> (Vec<Op>, f32) {
    let mut ops = vec![Op::StartTextSection];
    let mut body_y = PDF_NEW_PAGE_BODY_START_Y;

    if let Some(title) = title {
        ops.push(Op::SetFontSize {
            size: Pt(20.0),
            font: font.clone(),
        });
        let mut title_y = PDF_TITLE_Y;
        for title_line in wrap_pdf_text_line(title, 72) {
            ops.push(Op::SetTextMatrix {
                matrix: TextMatrix::Translate(Pt(PDF_LEFT_X), Pt(title_y)),
            });
            ops.push(Op::WriteText {
                items: vec![TextItem::Text(title_line)],
                font: font.clone(),
            });
            title_y -= 24.0;
        }
        body_y = (title_y - 6.0).min(PDF_BODY_START_Y);
    }

    ops.push(Op::SetFontSize {
        size: Pt(11.0),
        font: font.clone(),
    });

    (ops, body_y)
}

fn write_pdf_line(
    ops: &mut Vec<Op>,
    line: &str,
    y: f32,
    speaker_color: Option<&str>,
    font: &FontId,
) {
    ops.push(Op::SetTextMatrix {
        matrix: TextMatrix::Translate(Pt(PDF_LEFT_X), Pt(y)),
    });
    if let Some((red, green, blue)) = speaker_color.and_then(parse_hex_rgb) {
        ops.push(Op::SetFillColor {
            col: Color::Rgb(Rgb::new(
                red as f32 / 255.0,
                green as f32 / 255.0,
                blue as f32 / 255.0,
                None,
            )),
        });
    } else {
        ops.push(Op::SetFillColor {
            col: Color::Rgb(Rgb::new(0.12, 0.14, 0.19, None)),
        });
    }
    ops.push(Op::WriteText {
        items: vec![TextItem::Text(line.to_string())],
        font: font.clone(),
    });
}

fn wrap_pdf_text_line(line: &str, max_chars: usize) -> Vec<String> {
    if line.trim().is_empty() {
        return vec![String::new()];
    }

    let mut wrapped = Vec::new();
    let mut current = String::new();

    for word in line.split_whitespace() {
        if word.chars().count() > max_chars {
            if !current.is_empty() {
                wrapped.push(current);
                current = String::new();
            }
            let chars = word.chars().collect::<Vec<_>>();
            wrapped.extend(
                chars
                    .chunks(max_chars)
                    .map(|chunk| chunk.iter().collect::<String>()),
            );
            continue;
        }

        let next_len = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };

        if next_len > max_chars && !current.is_empty() {
            wrapped.push(current);
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }

    if !current.is_empty() {
        wrapped.push(current);
    }

    wrapped
}

fn normalize_export_language(value: Option<&str>) -> &'static str {
    match value.unwrap_or("en").trim() {
        "it" => "it",
        "es" => "es",
        "de" => "de",
        _ => "en",
    }
}

fn localized_export_fallback_title(language: &str) -> &'static str {
    match language {
        "it" => "Trascrizione",
        "es" => "Transcripción",
        "de" => "Transkript",
        _ => "Transcript",
    }
}

fn localized_export_document_title(language: &str, title: &str) -> String {
    let title = if title.trim().is_empty() {
        localized_export_fallback_title(language)
    } else {
        title.trim()
    };

    match language {
        "it" => format!("Trascrizione di {title}"),
        "es" => format!("Transcripción de {title}"),
        "de" => format!("Transkript von {title}"),
        _ => format!("Transcript of {title}"),
    }
}

fn localized_export_primary_section_title(language: &str, style: ExportStyle) -> &'static str {
    match style {
        ExportStyle::Segments => match language {
            "it" => "Segmenti",
            "es" => "Segmentos",
            "de" => "Segmente",
            _ => "Segments",
        },
        _ => match language {
            "it" => "Trascrizione",
            "es" => "Transcripción",
            "de" => "Transkript",
            _ => "Transcript",
        },
    }
}

fn localized_export_summary_title(language: &str) -> &'static str {
    match language {
        "it" => "Riassunto",
        "es" => "Resumen",
        "de" => "Zusammenfassung",
        _ => "Summary",
    }
}

fn localized_export_faq_title(language: &str) -> &'static str {
    match language {
        "it" => "Domande frequenti",
        "es" => "Preguntas frecuentes",
        "de" => "Häufige Fragen",
        _ => "FAQs",
    }
}

fn localized_export_csv_header(language: &str, include_speaker_names: bool) -> &'static str {
    match (language, include_speaker_names) {
        ("it", true) => "Timestamp inizio;Timestamp fine;Trascrizione;Speaker",
        ("it", false) => "Timestamp inizio;Timestamp fine;Trascrizione",
        ("es", true) => "Marca de tiempo inicial;Marca de tiempo final;Transcripción;Hablante",
        ("es", false) => "Marca de tiempo inicial;Marca de tiempo final;Transcripción",
        ("de", true) => "Start-Zeitstempel;End-Zeitstempel;Transkript;Sprecher",
        ("de", false) => "Start-Zeitstempel;End-Zeitstempel;Transkript",
        (_, true) => "Start Timestamp;End Timestamp;Transcript;Speaker",
        (_, false) => "Start Timestamp;End Timestamp;Transcript",
    }
}

fn localized_generated_pack_title(language: &str, kind: GeneratedArtifactPackKind) -> &'static str {
    match kind {
        GeneratedArtifactPackKind::StudyPack => match language {
            "it" => "Pacchetto studio",
            "es" => "Paquete de estudio",
            "de" => "Study Pack",
            _ => "Study Pack",
        },
        GeneratedArtifactPackKind::MeetingIntelligence => match language {
            "it" => "Meeting intelligence",
            "es" => "Inteligencia de reuniones",
            "de" => "Meeting Intelligence",
            _ => "Meeting Intelligence",
        },
    }
}

fn parse_generated_pack_from_metadata(
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> Option<GeneratedArtifactPack> {
    let raw = metadata.get(key)?;
    let parsed = serde_json::from_str::<GeneratedArtifactPack>(raw).ok()?;
    if parsed.body_markdown.trim().is_empty() {
        return None;
    }
    Some(parsed)
}

fn build_generated_pack_sections(
    language: &str,
    metadata: &BTreeMap<String, String>,
) -> Vec<ExportDocumentSection> {
    [
        (
            STUDY_PACK_METADATA_KEY,
            GeneratedArtifactPackKind::StudyPack,
        ),
        (
            MEETING_PACK_METADATA_KEY,
            GeneratedArtifactPackKind::MeetingIntelligence,
        ),
    ]
    .into_iter()
    .filter_map(|(key, kind)| {
        let pack = parse_generated_pack_from_metadata(metadata, key)?;
        Some(ExportDocumentSection {
            title: localized_generated_pack_title(language, kind).to_string(),
            body: pack.body_markdown.trim().to_string(),
            styled_lines: None,
        })
    })
    .collect()
}

fn build_primary_section_styled_lines(
    segments: &[ExportSegment],
    _transcription: &str,
    style: ExportStyle,
    include_timestamps: bool,
    include_speaker_names: bool,
    speaker_colors: &BTreeMap<String, String>,
) -> Option<Vec<ExportStyledLine>> {
    if segments.is_empty() {
        return None;
    }

    let lines = match style {
        ExportStyle::Segments => segments
            .iter()
            .map(|segment| ExportStyledLine {
                text: if include_timestamps {
                    format!(
                        "[{}] {}",
                        segment.time,
                        render_export_segment_line(segment, include_speaker_names)
                    )
                } else {
                    render_export_segment_line(segment, include_speaker_names)
                },
                speaker_color: resolve_export_speaker_color(segment, speaker_colors),
            })
            .collect::<Vec<_>>(),
        ExportStyle::Transcript if include_timestamps => segments
            .iter()
            .map(|segment| ExportStyledLine {
                text: format!(
                    "[{}] {}",
                    segment.time,
                    render_export_segment_line(segment, include_speaker_names)
                ),
                speaker_color: resolve_export_speaker_color(segment, speaker_colors),
            })
            .collect::<Vec<_>>(),
        ExportStyle::Subtitles => segments
            .iter()
            .zip(resolve_segment_timings(segments))
            .enumerate()
            .flat_map(|(index, (segment, timing))| {
                let mut cue_lines = vec![
                    ExportStyledLine {
                        text: (index + 1).to_string(),
                        speaker_color: None,
                    },
                    ExportStyledLine {
                        text: format!(
                            "{} --> {}",
                            format_srt_time(timing.start_millis),
                            format_srt_time(timing.end_millis)
                        ),
                        speaker_color: None,
                    },
                    ExportStyledLine {
                        text: render_export_segment_line(segment, include_speaker_names),
                        speaker_color: resolve_export_speaker_color(segment, speaker_colors),
                    },
                ];

                if index + 1 < segments.len() {
                    cue_lines.push(ExportStyledLine {
                        text: String::new(),
                        speaker_color: None,
                    });
                }
                cue_lines
            })
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };

    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

#[allow(clippy::too_many_arguments)]
fn build_export_document(
    language: &str,
    title: &str,
    transcription: &str,
    summary: &str,
    faqs: &str,
    metadata: &BTreeMap<String, String>,
    segments: &[ExportSegment],
    style: ExportStyle,
    include_timestamps: bool,
    include_speaker_names: bool,
    speaker_colors: &BTreeMap<String, String>,
) -> ExportDocument {
    let mut sections = vec![ExportDocumentSection {
        title: localized_export_primary_section_title(language, style).to_string(),
        body: build_export_content(
            transcription,
            segments,
            style,
            include_timestamps,
            include_speaker_names,
        ),
        styled_lines: build_primary_section_styled_lines(
            segments,
            transcription,
            style,
            include_timestamps,
            include_speaker_names,
            speaker_colors,
        ),
    }];

    if !summary.trim().is_empty() {
        sections.push(ExportDocumentSection {
            title: localized_export_summary_title(language).to_string(),
            body: summary.trim().to_string(),
            styled_lines: None,
        });
    }

    if !faqs.trim().is_empty() {
        sections.push(ExportDocumentSection {
            title: localized_export_faq_title(language).to_string(),
            body: faqs.trim().to_string(),
            styled_lines: None,
        });
    }
    sections.extend(build_generated_pack_sections(language, metadata));

    ExportDocument {
        title: localized_export_document_title(language, title),
        sections,
    }
}

fn render_plain_text_document(document: &ExportDocument) -> String {
    let mut blocks = vec![document.title.trim().to_string()];
    blocks.extend(document.sections.iter().filter_map(|section| {
        let body = section.body.trim();
        if body.is_empty() {
            None
        } else {
            Some(format!("{}\n{}", section.title.trim(), body))
        }
    }));
    blocks.join("\n\n")
}

fn render_markdown_document(document: &ExportDocument) -> String {
    let mut blocks = vec![format!("# {}", document.title.trim())];
    blocks.extend(document.sections.iter().filter_map(|section| {
        let body = section.body.trim();
        if body.is_empty() {
            None
        } else {
            Some(format!("## {}\n\n{}", section.title.trim(), body))
        }
    }));
    blocks.join("\n\n")
}

fn render_document_body_styled_lines(document: &ExportDocument) -> Vec<ExportStyledLine> {
    let mut lines = Vec::new();

    for (index, section) in document.sections.iter().enumerate() {
        if index == 0 {
            lines.push(ExportStyledLine {
                text: String::new(),
                speaker_color: None,
            });
        } else {
            lines.push(ExportStyledLine {
                text: String::new(),
                speaker_color: None,
            });
            lines.push(ExportStyledLine {
                text: String::new(),
                speaker_color: None,
            });
        }

        lines.push(ExportStyledLine {
            text: section.title.clone(),
            speaker_color: None,
        });

        if let Some(styled_lines) = &section.styled_lines {
            lines.extend(styled_lines.iter().cloned());
        } else {
            lines.extend(section.body.lines().map(|line| ExportStyledLine {
                text: line.to_string(),
                speaker_color: None,
            }));
        }
    }

    lines
}

fn build_export_segments(artifact: &TranscriptArtifact, transcription: &str) -> Vec<ExportSegment> {
    let timeline_segments = parse_timeline_context_segments(artifact)
        .into_iter()
        .filter_map(|segment| {
            let text = segment.text.trim();
            let time = segment.time_label.unwrap_or_default();
            if text.is_empty() || time.trim().is_empty() {
                return None;
            }
            Some(ExportSegment {
                time,
                line: text.to_string(),
                start_seconds: segment.start_seconds.map(f64::from),
                end_seconds: segment.end_seconds.map(f64::from),
                speaker_id: segment.speaker_id,
                speaker_label: segment.speaker_label,
            })
        })
        .collect::<Vec<_>>();

    if timeline_segments.is_empty() {
        build_segments_from_text(transcription)
    } else {
        timeline_segments
    }
}

fn build_segments_from_text(transcription: &str) -> Vec<ExportSegment> {
    transcription
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(index, line)| {
            let seconds = (index as u32) * 4;
            let mm = seconds / 60;
            let ss = seconds % 60;
            ExportSegment {
                time: format!("{:02}:{:02}", mm, ss),
                line: line.to_string(),
                start_seconds: Some(f64::from(seconds)),
                end_seconds: Some(f64::from(seconds + 4)),
                speaker_id: None,
                speaker_label: None,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedSegmentTiming {
    start_millis: u64,
    end_millis: u64,
}

fn valid_segment_seconds(value: Option<f64>) -> Option<f64> {
    value.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

fn segment_start_seconds(segment: &ExportSegment) -> f64 {
    valid_segment_seconds(segment.start_seconds)
        .unwrap_or_else(|| parse_timestamp_to_seconds(&segment.time))
}

fn seconds_to_millis(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

fn resolve_segment_timings(segments: &[ExportSegment]) -> Vec<ResolvedSegmentTiming> {
    let starts = segments
        .iter()
        .map(segment_start_seconds)
        .collect::<Vec<_>>();

    segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let start = starts[index];
            let explicit_end = valid_segment_seconds(segment.end_seconds)
                .filter(|end_seconds| *end_seconds > start);
            let next_start = starts
                .get(index + 1)
                .copied()
                .filter(|next_seconds| *next_seconds > start);
            let end = explicit_end.or(next_start).unwrap_or(start + 4.0);
            let start_millis = seconds_to_millis(start);
            let end_millis = seconds_to_millis(end).max(start_millis.saturating_add(1));
            ResolvedSegmentTiming {
                start_millis,
                end_millis,
            }
        })
        .collect()
}

fn resolved_export_segments(
    segments: &[ExportSegment],
    include_speaker_names: bool,
) -> Vec<ExportSegment> {
    segments
        .iter()
        .zip(resolve_segment_timings(segments))
        .map(|(segment, timing)| ExportSegment {
            time: segment.time.clone(),
            line: segment.line.clone(),
            start_seconds: Some(timing.start_millis as f64 / 1000.0),
            end_seconds: Some(timing.end_millis as f64 / 1000.0),
            speaker_id: include_speaker_names
                .then(|| segment.speaker_id.clone())
                .flatten(),
            speaker_label: include_speaker_names
                .then(|| segment.speaker_label.clone())
                .flatten(),
        })
        .collect()
}

fn format_mm_ss_millis(total_millis: u64) -> String {
    let total_seconds = total_millis / 1000;
    let mm = total_seconds / 60;
    let ss = total_seconds % 60;
    format!("{:02}:{:02}", mm, ss)
}

fn parse_timestamp_to_seconds(value: &str) -> f64 {
    let mut parts = value.trim().split(':').collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() > 3 {
        return 0.0;
    }

    if parts.len() == 2 {
        parts.insert(0, "0");
    }

    let hh = parts[0].parse::<f64>().unwrap_or(0.0);
    let mm = parts[1].parse::<f64>().unwrap_or(0.0);
    let ss = parts[2].parse::<f64>().unwrap_or(0.0);

    (hh * 3600.0 + mm * 60.0 + ss).max(0.0)
}

fn format_srt_time(total_millis: u64) -> String {
    let hh = total_millis / 3_600_000;
    let mm = (total_millis % 3_600_000) / 60_000;
    let ss = (total_millis % 60_000) / 1000;
    let millis = total_millis % 1000;
    format!("{:02}:{:02}:{:02},{:03}", hh, mm, ss, millis)
}

fn format_vtt_time(total_millis: u64) -> String {
    let hh = total_millis / 3_600_000;
    let mm = (total_millis % 3_600_000) / 60_000;
    let ss = (total_millis % 60_000) / 1000;
    let millis = total_millis % 1000;
    format!("{:02}:{:02}:{:02}.{:03}", hh, mm, ss, millis)
}

fn build_markdown_subtitles_content(
    segments: &[ExportSegment],
    transcription: &str,
    include_speaker_names: bool,
) -> String {
    if segments.is_empty() {
        return transcription.trim().to_string();
    }

    segments
        .iter()
        .map(|segment| {
            format!(
                "{}\n{}",
                render_export_segment_line(segment, include_speaker_names),
                segment.time
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_vtt_content(
    segments: &[ExportSegment],
    transcription: &str,
    include_speaker_names: bool,
) -> String {
    if segments.is_empty() {
        return format!("WEBVTT\n\n{}", transcription.trim());
    }

    let cues = segments
        .iter()
        .zip(resolve_segment_timings(segments))
        .map(|(segment, timing)| {
            format!(
                "{} --> {}\n{}",
                format_vtt_time(timing.start_millis),
                format_vtt_time(timing.end_millis),
                render_export_segment_line(segment, include_speaker_names)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!("WEBVTT\n\n{cues}")
}

fn build_export_content(
    transcription: &str,
    segments: &[ExportSegment],
    style: ExportStyle,
    include_timestamps: bool,
    include_speaker_names: bool,
) -> String {
    let normalized_transcription = transcription.trim();

    match style {
        ExportStyle::Subtitles => {
            if segments.is_empty() {
                return normalized_transcription.to_string();
            }

            segments
                .iter()
                .zip(resolve_segment_timings(segments))
                .enumerate()
                .map(|(index, (segment, timing))| {
                    format!(
                        "{}\n{} --> {}\n{}",
                        index + 1,
                        format_srt_time(timing.start_millis),
                        format_srt_time(timing.end_millis),
                        render_export_segment_line(segment, include_speaker_names)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        }
        ExportStyle::Segments => {
            if segments.is_empty() {
                return normalized_transcription.to_string();
            }

            segments
                .iter()
                .map(|segment| {
                    if include_timestamps {
                        format!(
                            "[{}] {}",
                            segment.time,
                            render_export_segment_line(segment, include_speaker_names)
                        )
                    } else {
                        render_export_segment_line(segment, include_speaker_names)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        ExportStyle::Transcript => {
            if !include_timestamps || segments.is_empty() {
                normalized_transcription.to_string()
            } else {
                segments
                    .iter()
                    .map(|segment| {
                        format!(
                            "[{}] {}",
                            segment.time,
                            render_export_segment_line(segment, include_speaker_names)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io::Read,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    use async_trait::async_trait;
    use chrono::Utc;
    use sbobino_application::dto::SummaryFaq;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use super::{
        add_conversation_context, build_artifact_context_transcript, build_chat_context_candidates,
        build_chunk_note_prompt, build_confidence_aware_optimize_prompt,
        build_direct_summary_prompt, build_export_content, build_export_document,
        build_export_segments, build_summary_instructions, build_summary_synthesis_prompt,
        chunk_text_by_words, extract_low_confidence_spans, has_timeline_manual_edits,
        is_context_window_error, manual_optimization_groups, next_timeline_manual_edit_metadata,
        optimize_source_language_groups, optimize_with_rag, render_markdown_document,
        render_plain_text_document, resolve_ai_output_language, run_cancellable,
        summarize_with_rag, timeline_segments_for_diarization, trimmed_audio_output_metadata,
        validate_trimmed_audio_output, ApplicationError, ArtifactAiContextOptions,
        ArtifactChatMessage, ArtifactKind, ExportStyle, SourceLanguageOptimizationGroup,
        SummarizeArtifactPayload, TranscriptArtifact, TranscriptEnhancer,
        MIN_TRIMMED_AUDIO_DURATION_SECONDS,
    };

    struct TrackingEnhancer {
        optimize_calls: AtomicUsize,
        ask_calls: AtomicUsize,
        active_calls: AtomicUsize,
        max_active_calls: AtomicUsize,
        prompts: Mutex<Vec<String>>,
        optimize_languages: Mutex<Vec<String>>,
        prefer_single_pass: bool,
        chunk_concurrency_limit: usize,
        optimize_context_limit_chars: Option<usize>,
        optimize_direct_prompt_char_budget: usize,
        fail_direct_attempts: AtomicUsize,
        hallucinate_optimize: bool,
        hallucinate_merge: bool,
        hallucinate_long: bool,
        substantial_rewrite_optimize: bool,
    }

    impl TrackingEnhancer {
        fn new(
            prefer_single_pass: bool,
            chunk_concurrency_limit: usize,
            fail_direct_attempts: usize,
        ) -> Self {
            Self {
                optimize_calls: AtomicUsize::new(0),
                ask_calls: AtomicUsize::new(0),
                active_calls: AtomicUsize::new(0),
                max_active_calls: AtomicUsize::new(0),
                prompts: Mutex::new(Vec::new()),
                optimize_languages: Mutex::new(Vec::new()),
                prefer_single_pass,
                chunk_concurrency_limit,
                optimize_context_limit_chars: None,
                optimize_direct_prompt_char_budget: 3_200,
                fail_direct_attempts: AtomicUsize::new(fail_direct_attempts),
                hallucinate_optimize: false,
                hallucinate_merge: false,
                hallucinate_long: false,
                substantial_rewrite_optimize: false,
            }
        }

        fn with_optimize_context_limit(
            prefer_single_pass: bool,
            chunk_concurrency_limit: usize,
            optimize_direct_prompt_char_budget: usize,
            optimize_context_limit_chars: usize,
        ) -> Self {
            Self {
                optimize_context_limit_chars: Some(optimize_context_limit_chars),
                optimize_direct_prompt_char_budget,
                ..Self::new(prefer_single_pass, chunk_concurrency_limit, 0)
            }
        }

        fn with_hallucinations(
            prefer_single_pass: bool,
            chunk_concurrency_limit: usize,
            fail_direct_attempts: usize,
            hallucinate_optimize: bool,
            hallucinate_merge: bool,
        ) -> Self {
            Self {
                hallucinate_optimize,
                hallucinate_merge,
                substantial_rewrite_optimize: false,
                ..Self::new(
                    prefer_single_pass,
                    chunk_concurrency_limit,
                    fail_direct_attempts,
                )
            }
        }

        fn with_long_hallucinations(
            prefer_single_pass: bool,
            chunk_concurrency_limit: usize,
            fail_direct_attempts: usize,
            hallucinate_optimize: bool,
            hallucinate_merge: bool,
        ) -> Self {
            // Like with_hallucinations, but the optimize branch
            // appends a longer tail so the additive change exceeds
            // MAX_CONTEXTUAL_INSERT_TOKENS and the safety net
            // reverts it.
            Self {
                hallucinate_optimize,
                hallucinate_merge,
                substantial_rewrite_optimize: false,
                hallucinate_long: true,
                ..Self::new(
                    prefer_single_pass,
                    chunk_concurrency_limit,
                    fail_direct_attempts,
                )
            }
        }

        fn with_substantial_rewrite(
            prefer_single_pass: bool,
            chunk_concurrency_limit: usize,
            fail_direct_attempts: usize,
        ) -> Self {
            Self {
                substantial_rewrite_optimize: true,
                ..Self::new(
                    prefer_single_pass,
                    chunk_concurrency_limit,
                    fail_direct_attempts,
                )
            }
        }

        fn record_peak_concurrency(&self, observed: usize) {
            let mut current = self.max_active_calls.load(Ordering::SeqCst);
            while observed > current {
                match self.max_active_calls.compare_exchange(
                    current,
                    observed,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
        }
    }

    #[async_trait]
    impl TranscriptEnhancer for TrackingEnhancer {
        async fn optimize(
            &self,
            text: &str,
            language_code: &str,
        ) -> Result<String, ApplicationError> {
            self.optimize_calls.fetch_add(1, Ordering::SeqCst);
            self.optimize_languages
                .lock()
                .expect("optimize language lock poisoned")
                .push(language_code.to_string());
            if self
                .optimize_context_limit_chars
                .is_some_and(|limit| text.chars().count() > limit)
            {
                return Err(ApplicationError::PostProcessing(
                    "Foundation bridge error: Exceeded model context window size".to_string(),
                ));
            }
            if self.hallucinate_optimize {
                if self.hallucinate_long {
                    Ok(format!(
                        "{text} extra trailing words here and a final sentence that should be rejected"
                    ))
                } else {
                    Ok(format!("{text} added commentary"))
                }
            } else if self.substantial_rewrite_optimize {
                Ok(simulate_substantial_transcript_rewrite(text))
            } else {
                Ok(text.to_string())
            }
        }

        async fn summarize_and_faq(
            &self,
            text: &str,
            _language_code: &str,
        ) -> Result<SummaryFaq, ApplicationError> {
            Ok(SummaryFaq {
                summary: text.to_string(),
                faqs: String::new(),
            })
        }

        async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
            self.ask_calls.fetch_add(1, Ordering::SeqCst);
            self.prompts
                .lock()
                .expect("prompt log lock poisoned")
                .push(prompt.to_string());

            let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.record_peak_concurrency(active);

            for _ in 0..6 {
                tokio::task::yield_now().await;
            }

            self.active_calls.fetch_sub(1, Ordering::SeqCst);

            if prompt.contains("Full transcript:")
                && self.fail_direct_attempts.load(Ordering::SeqCst) > 0
            {
                self.fail_direct_attempts.fetch_sub(1, Ordering::SeqCst);
                return Err(ApplicationError::PostProcessing(
                    "Foundation bridge error: Exceeded model context window size".to_string(),
                ));
            }

            if prompt.contains("Chunk notes:") || prompt.contains("Full transcript:") {
                Ok("final summary".to_string())
            } else if prompt.contains("Optimized transcript sections:") {
                if self.hallucinate_merge {
                    Ok("merged optimized transcript with extra conclusion".to_string())
                } else {
                    Ok(prompt
                        .split("Optimized transcript sections:\n")
                        .nth(1)
                        .unwrap_or_default()
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("[Section "))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_string())
                }
            } else {
                Ok("chunk note".to_string())
            }
        }

        fn prefers_single_pass_summary(&self) -> bool {
            self.prefer_single_pass
        }

        fn summary_chunk_concurrency_limit(&self) -> usize {
            self.chunk_concurrency_limit
        }

        fn prefers_single_pass_optimize(&self) -> bool {
            self.prefer_single_pass
        }

        fn optimize_chunk_concurrency_limit(&self) -> usize {
            self.chunk_concurrency_limit
        }

        fn optimize_direct_prompt_char_budget(&self) -> usize {
            self.optimize_direct_prompt_char_budget
        }
    }

    impl Default for TrackingEnhancer {
        fn default() -> Self {
            Self::new(false, 3, 0)
        }
    }

    fn simulate_substantial_transcript_rewrite(text: &str) -> String {
        // Simulate what an LLM following the new optimize prompt would
        // do: drop common filler, capitalize the first letter of the
        // chunk, and add a trailing period if the chunk has no terminal
        // punctuation. Deterministic so tests can assert exact output.
        let lowered = text.to_lowercase();
        let filler_words = [
            "uh", "ehm", "allora", "diciamo", "cioe", "cioè", "insomma", "tipo", "beh", "mmh",
            "mhh", "ah",
        ];

        let mut tokens: Vec<String> = Vec::new();
        for word in lowered.split_whitespace() {
            let stripped = word.trim_end_matches(|c: char| !c.is_alphanumeric());
            if filler_words.contains(&stripped) {
                continue;
            }
            tokens.push(word.to_string());
        }

        let joined = tokens.join(" ");
        let trimmed = joined.trim().to_string();
        if trimmed.is_empty() {
            return text.to_string();
        }

        let mut chars: Vec<char> = trimmed.chars().collect();
        if let Some(first) = chars.first_mut() {
            *first = first.to_ascii_uppercase();
        }
        let capitalized: String = chars.into_iter().collect();

        let ends_with_terminal = capitalized
            .chars()
            .last()
            .map(|c| matches!(c, '.' | '?' | '!' | ':' | ';'))
            .unwrap_or(false);
        if ends_with_terminal {
            capitalized
        } else {
            format!("{capitalized}.")
        }
    }

    fn sample_artifact(text: &str) -> TranscriptArtifact {
        TranscriptArtifact {
            id: "id-1".to_string(),
            job_id: "job-1".to_string(),
            title: "Sample".to_string(),
            kind: ArtifactKind::File,
            source_label: "/tmp/sample.wav".to_string(),
            source_origin: sbobino_domain::ArtifactSourceOrigin::Imported,
            audio_available: true,
            audio_backfill_status: sbobino_domain::ArtifactAudioBackfillStatus::Imported,
            revision: 1,
            raw_transcript: text.to_string(),
            optimized_transcript: String::new(),
            summary: String::new(),
            faqs: String::new(),
            metadata: BTreeMap::new(),
            parent_artifact_id: None,
            processing_engine: Some("whisper_cpp".to_string()),
            processing_model: Some("base".to_string()),
            processing_language: Some("en".to_string()),
            audio_duration_seconds: Some(42.0),
            audio_byte_size: Some(2048),
            source_external_path: None,
            whisper_options_json: None,
            diarization_settings_json: None,
            ai_provider_snapshot_json: None,
            source_fingerprint_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn auto_ai_language_uses_detected_duration_then_interface_fallback() {
        let mut artifact = sample_artifact("Ciao hello");
        artifact.metadata.insert(
            "detected_languages".to_string(),
            json!([
                {"code": "it", "duration_seconds": 8.0, "character_count": 10},
                {"code": "en", "duration_seconds": 2.0, "character_count": 40}
            ])
            .to_string(),
        );
        assert_eq!(resolve_ai_output_language(&artifact, "auto", "de"), "it");

        let legacy = sample_artifact("legacy transcript");
        assert_eq!(resolve_ai_output_language(&legacy, "auto", "de"), "de");
        assert_eq!(resolve_ai_output_language(&legacy, "it-IT", "de"), "it");
    }

    #[tokio::test]
    async fn manual_optimization_preserves_contiguous_timeline_language_groups() {
        let submitted = "ciao mondo\ncome stai\nhello world";
        let mut artifact = sample_artifact(submitted);
        artifact.metadata.insert(
            "timeline_v2".to_string(),
            json!({
                "version": 2,
                "segments": [
                    {"text": "ciao mondo", "language_code": "it-IT"},
                    {"text": "come stai", "language_code": "it"},
                    {"text": "hello world", "language_code": "en-US"}
                ]
            })
            .to_string(),
        );

        let groups = manual_optimization_groups(&artifact, submitted);
        assert_eq!(
            groups,
            vec![
                SourceLanguageOptimizationGroup {
                    language_code: "it".to_string(),
                    text: "ciao mondo\ncome stai".to_string(),
                },
                SourceLanguageOptimizationGroup {
                    language_code: "en".to_string(),
                    text: "hello world".to_string(),
                },
            ]
        );

        let enhancer = TrackingEnhancer::default();
        let optimized = optimize_source_language_groups(&enhancer, &groups)
            .await
            .expect("manual groups should optimize");
        assert_eq!(
            *enhancer
                .optimize_languages
                .lock()
                .expect("optimize language lock poisoned"),
            vec!["it".to_string(), "en".to_string()]
        );
        assert_eq!(optimized, submitted);
        assert!(!optimized.contains("[source_language="));
    }

    #[tokio::test]
    async fn manual_optimization_uses_auto_for_unmatched_edited_text() {
        let mut artifact = sample_artifact("ciao mondo\nhello world");
        artifact.metadata.insert(
            "timeline_v2".to_string(),
            json!({
                "version": 2,
                "segments": [
                    {"text": "ciao mondo", "language_code": "it"},
                    {"text": "hello world", "language_code": "en"}
                ]
            })
            .to_string(),
        );
        let submitted = "user edited transcript with different wording";
        let groups = manual_optimization_groups(&artifact, submitted);
        assert_eq!(
            groups,
            vec![SourceLanguageOptimizationGroup {
                language_code: "auto".to_string(),
                text: submitted.to_string(),
            }]
        );

        let enhancer = TrackingEnhancer::default();
        optimize_source_language_groups(&enhancer, &groups)
            .await
            .expect("unmatched manual text should optimize");
        assert_eq!(
            *enhancer
                .optimize_languages
                .lock()
                .expect("optimize language lock poisoned"),
            vec!["auto".to_string()]
        );
    }

    #[tokio::test]
    async fn manual_optimization_uses_auto_for_case_only_timeline_edit() {
        let mut artifact = sample_artifact("Ciao mondo\nHello world");
        artifact.metadata.insert(
            "timeline_v2".to_string(),
            json!({
                "version": 2,
                "segments": [
                    {"text": "Ciao mondo", "language_code": "it"},
                    {"text": "Hello world", "language_code": "en"}
                ]
            })
            .to_string(),
        );
        let submitted = "ciao mondo\nhello world";
        let groups = manual_optimization_groups(&artifact, submitted);
        assert_eq!(
            groups,
            vec![SourceLanguageOptimizationGroup {
                language_code: "auto".to_string(),
                text: submitted.to_string(),
            }]
        );

        let enhancer = TrackingEnhancer::default();
        let optimized = optimize_source_language_groups(&enhancer, &groups)
            .await
            .expect("case-only edit should optimize");
        assert_eq!(optimized, submitted);
        assert_eq!(
            *enhancer
                .optimize_languages
                .lock()
                .expect("optimize language lock poisoned"),
            vec!["auto".to_string()]
        );
    }

    fn sample_artifact_with_timeline(text: &str) -> TranscriptArtifact {
        let mut artifact = sample_artifact(text);
        artifact.metadata.insert(
            "timeline_v2".to_string(),
            json!({
                "version": 2,
                "segments": [
                    {
                        "text": "Alice opens the meeting.",
                        "start_seconds": 12.4,
                        "speaker_id": "speaker_1",
                        "speaker_label": "Alice"
                    },
                    {
                        "text": "Bob confirms the next step.",
                        "start_seconds": 24.9,
                        "speaker_id": "speaker_2",
                        "speaker_label": "Bob"
                    }
                ]
            })
            .to_string(),
        );
        artifact
    }

    fn sample_artifact_with_confidence_timeline(text: &str) -> TranscriptArtifact {
        let mut artifact = sample_artifact(text);
        artifact.metadata.insert(
            "timeline_v2".to_string(),
            json!({
                "version": 2,
                "segments": [
                    {
                        "text": "Questo quesito riguarda Keras Tuner e JSON Schema.",
                        "start_seconds": 12.0,
                        "words": [
                            { "text": "Questo", "confidence": 0.94, "start_seconds": 12.0 },
                            { "text": "quesito", "confidence": 0.92, "start_seconds": 12.3 },
                            { "text": "riguarda", "confidence": 0.87, "start_seconds": 12.7 },
                            { "text": "Cheras", "confidence": 0.31, "start_seconds": 13.0 },
                            { "text": "Tuner", "confidence": 0.42, "start_seconds": 13.3 },
                            { "text": "e", "confidence": 0.96, "start_seconds": 13.5 },
                            { "text": "GSM", "confidence": 0.27, "start_seconds": 13.9 },
                            { "text": "Scheme", "confidence": 0.49, "start_seconds": 14.2 }
                        ]
                    }
                ]
            })
            .to_string(),
        );
        artifact
    }

    #[test]
    fn chunker_splits_and_progresses() {
        let input =
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
        let chunks = chunk_text_by_words(input, 20, 2);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|chunk| !chunk.trim().is_empty()));
    }

    #[test]
    fn chat_context_candidates_are_created() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";
        let artifact = sample_artifact(text);
        let candidates = build_chat_context_candidates(
            &artifact,
            "what about gamma and sigma?",
            ArtifactAiContextOptions::default(),
        );
        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .all(|value| value.contains("User question:")));
        assert!(candidates
            .iter()
            .all(|value| value.contains("Reply in the same language as the user's question")));
    }

    #[test]
    fn completed_chat_turns_are_included_in_the_next_question_context() {
        let candidates =
            vec!["Transcript snippets:\nmeeting\nUser question:\nWhat next?".to_string()];
        let messages = vec![
            ArtifactChatMessage::new("id-1", "user", "What was approved?", "typed", "complete"),
            ArtifactChatMessage::new(
                "id-1",
                "assistant",
                "The launch was approved.",
                "typed",
                "complete",
            ),
            ArtifactChatMessage::new("id-1", "assistant", "temporary failure", "typed", "error"),
            ArtifactChatMessage::new("id-1", "user", "incomplete question", "typed", "complete"),
        ];

        let enriched = add_conversation_context(candidates, &messages, None);

        assert!(enriched[0].contains("What was approved?"));
        assert!(enriched[0].contains("The launch was approved."));
        assert!(!enriched[0].contains("temporary failure"));
        assert!(!enriched[0].contains("incomplete question"));
        assert!(enriched[0].ends_with("User question:\nWhat next?"));
    }

    #[test]
    fn concurrent_completed_chat_turns_keep_correct_question_answer_pairs() {
        let candidates =
            vec!["Transcript snippets:\nmeeting\nUser question:\nWhat next?".to_string()];
        let messages = vec![
            ArtifactChatMessage::new("id-1", "user", "Question one", "typed", "complete"),
            ArtifactChatMessage::new("id-1", "user", "Question two", "typed", "complete"),
            ArtifactChatMessage::new("id-1", "assistant", "Answer one", "typed", "complete"),
            ArtifactChatMessage::new("id-1", "assistant", "Answer two", "typed", "complete"),
        ];

        let enriched = add_conversation_context(candidates, &messages, None);

        assert!(enriched[0].contains("user: Question one"));
        assert!(enriched[0].contains("assistant: Answer one"));
        assert!(enriched[0].contains("user: Question two"));
        assert!(enriched[0].contains("assistant: Answer two"));
    }

    #[test]
    fn timeline_context_respects_timestamp_and_speaker_toggles() {
        let artifact = sample_artifact_with_timeline("fallback transcript");

        let transcript = build_artifact_context_transcript(
            &artifact,
            ArtifactAiContextOptions {
                include_timestamps: true,
                include_speakers: true,
            },
        );
        assert!(transcript.contains("[00:12] Alice: Alice opens the meeting."));
        assert!(transcript.contains("[00:24] Bob: Bob confirms the next step."));

        let transcript_without_labels = build_artifact_context_transcript(
            &artifact,
            ArtifactAiContextOptions {
                include_timestamps: false,
                include_speakers: false,
            },
        );
        assert!(!transcript_without_labels.contains("[00:12]"));
        assert!(!transcript_without_labels.contains("Alice:"));
        assert!(transcript_without_labels.contains("Alice opens the meeting."));
    }

    #[test]
    fn export_segments_use_timeline_and_keep_one_line_per_segment() {
        let artifact = sample_artifact_with_timeline("fallback transcript");
        let segments = build_export_segments(&artifact, "fallback transcript");

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].time, "00:12");
        assert_eq!(segments[0].line, "Alice opens the meeting.");
        assert_eq!(segments[0].speaker_id.as_deref(), Some("speaker_1"));

        let content = build_export_content(
            "fallback transcript",
            &segments,
            ExportStyle::Segments,
            true,
            true,
        );

        assert_eq!(segments[0].speaker_label.as_deref(), Some("Alice"));
        assert!(content.contains("[00:12] Alice: Alice opens the meeting."));
        assert!(content.contains("[00:24] Bob: Bob confirms the next step."));
        assert!(!content.contains("[00:12]\nAlice opens the meeting."));
    }

    #[test]
    fn export_document_styles_segment_lines_with_speaker_colors() {
        let artifact = sample_artifact_with_timeline("fallback transcript");
        let segments = build_export_segments(&artifact, "fallback transcript");
        let mut speaker_colors = BTreeMap::new();
        speaker_colors.insert("speaker_1".to_string(), "#123456".to_string());

        let document = build_export_document(
            "en",
            &artifact.title,
            &artifact.raw_transcript,
            &artifact.summary,
            &artifact.faqs,
            &artifact.metadata,
            &segments,
            ExportStyle::Segments,
            true,
            true,
            &speaker_colors,
        );

        let styled_lines = document.sections[0]
            .styled_lines
            .as_ref()
            .expect("primary section should expose styled lines");
        assert_eq!(
            styled_lines[0].text,
            "[00:12] Alice: Alice opens the meeting."
        );
        assert_eq!(styled_lines[0].speaker_color.as_deref(), Some("#123456"));
        assert!(styled_lines[1]
            .speaker_color
            .as_deref()
            .expect("speaker color fallback should exist")
            .starts_with('#'));
    }

    #[test]
    fn export_with_content_override_and_empty_segments_generates_segments_from_override() {
        let artifact = sample_artifact_with_timeline("fallback transcript");
        let base_transcription = "Optimized first line.\nOptimized second line.";
        let payload_segments = Some(Vec::<super::ExportSegment>::new());

        let segments = match payload_segments {
            Some(entries) if !entries.is_empty() => entries,
            Some(_) => super::build_segments_from_text(base_transcription),
            None => build_export_segments(&artifact, base_transcription),
        };

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].time, "00:00");
        assert_eq!(segments[0].line, "Optimized first line.");
        assert_eq!(segments[1].time, "00:04");
        assert_eq!(segments[1].line, "Optimized second line.");
    }

    #[test]
    fn prepared_raw_transcript_is_previewed_and_written_exactly_once() {
        let mut artifact = sample_artifact("Clean edited transcript.");
        artifact.title = "Meeting".to_string();
        artifact.summary = "Short summary.".to_string();

        let document = build_export_document(
            "en",
            &artifact.title,
            &artifact.raw_transcript,
            &artifact.summary,
            &artifact.faqs,
            &artifact.metadata,
            &[],
            ExportStyle::Transcript,
            false,
            false,
            &BTreeMap::new(),
        );
        let prepared = super::PreparedArtifactExport {
            artifact: artifact.clone(),
            format: super::ExportFormat::Txt,
            language: "en",
            style: ExportStyle::Transcript,
            options: super::ExportOptions::default(),
            grouping: super::ExportGrouping::None,
            transcription: artifact.raw_transcript.clone(),
            summary: artifact.summary.clone(),
            faqs: artifact.faqs.clone(),
            segments: Vec::new(),
            content: artifact.raw_transcript.clone(),
            document,
        };
        let preview =
            super::render_prepared_artifact_preview(&prepared).expect("preview should render");
        let temp = tempdir().expect("tempdir");
        let destination = temp.path().join("transcript.txt");
        super::write_prepared_artifact_export(&destination, &prepared)
            .expect("export should write");
        let exported = std::fs::read_to_string(destination).expect("exported text");

        assert_eq!(preview.content, exported);
        assert_eq!(exported.matches("Transcript of Meeting").count(), 1);
        assert_eq!(exported.matches("Clean edited transcript.").count(), 1);
        assert_eq!(exported.matches("Summary\nShort summary.").count(), 1);
    }

    #[test]
    fn subtitle_fallback_ends_at_the_next_segment_start() {
        let segments = vec![
            super::ExportSegment {
                time: "00:00".to_string(),
                line: "One.".to_string(),
                start_seconds: None,
                end_seconds: None,
                speaker_id: None,
                speaker_label: None,
            },
            super::ExportSegment {
                time: "00:05".to_string(),
                line: "Two.".to_string(),
                start_seconds: None,
                end_seconds: None,
                speaker_id: None,
                speaker_label: None,
            },
        ];

        let content =
            build_export_content("One. Two.", &segments, ExportStyle::Subtitles, true, false);

        assert!(content.contains("00:00:00,000 --> 00:00:05,000"));
        assert!(content.contains("00:00:05,000 --> 00:00:09,000"));
        assert!(!content.contains("00:00:11,000"));
    }

    #[test]
    fn subtitle_explicit_end_preserves_fractional_overlap() {
        let segments = vec![
            super::ExportSegment {
                time: "00:00".to_string(),
                line: "Overlapping first cue.".to_string(),
                start_seconds: Some(0.125),
                end_seconds: Some(8.5),
                speaker_id: None,
                speaker_label: None,
            },
            super::ExportSegment {
                time: "00:05".to_string(),
                line: "Second cue.".to_string(),
                start_seconds: Some(5.0),
                end_seconds: Some(6.25),
                speaker_id: None,
                speaker_label: None,
            },
        ];

        let srt = build_export_content(
            "Overlapping first cue. Second cue.",
            &segments,
            ExportStyle::Subtitles,
            true,
            false,
        );
        let vtt = super::build_vtt_content(&segments, "", false);

        assert!(srt.contains("00:00:00,125 --> 00:00:08,500"));
        assert!(srt.contains("00:00:05,000 --> 00:00:06,250"));
        assert!(vtt.contains("00:00:00.125 --> 00:00:08.500"));
    }

    #[test]
    fn csv_is_localized_escaped_and_uses_resolved_bounds() {
        let segments = vec![super::ExportSegment {
            time: "01:00".to_string(),
            line: "He said \"yes\"".to_string(),
            start_seconds: Some(60.25),
            end_seconds: Some(62.75),
            speaker_id: Some("speaker_1".to_string()),
            speaker_label: Some("A \"B\"".to_string()),
        }];

        let csv = super::render_csv_content("it", &segments, true);

        assert!(csv.starts_with("Timestamp inizio;Timestamp fine;Trascrizione;Speaker\n"));
        assert!(csv.contains("01:00;01:02;\"He said \"\"yes\"\"\";\"A \"\"B\"\"\""));
        assert!(!csv.contains("00:71"));
    }

    #[test]
    fn rich_json_preview_matches_written_output_and_adds_resolved_bounds() {
        let mut artifact = sample_artifact("He said yes.");
        artifact.title = "Meeting".to_string();
        artifact.summary = "Persisted summary".to_string();
        let segments = vec![super::ExportSegment {
            time: "00:00".to_string(),
            line: "He said yes.".to_string(),
            start_seconds: Some(0.125),
            end_seconds: Some(2.75),
            speaker_id: Some("speaker_1".to_string()),
            speaker_label: Some("Alice".to_string()),
        }];
        let document = build_export_document(
            "en",
            &artifact.title,
            &artifact.raw_transcript,
            "Draft summary",
            "",
            &artifact.metadata,
            &segments,
            ExportStyle::Segments,
            true,
            true,
            &BTreeMap::new(),
        );
        let prepared = super::PreparedArtifactExport {
            artifact,
            format: super::ExportFormat::Json,
            language: "en",
            style: ExportStyle::Segments,
            options: super::ExportOptions {
                include_timestamps: true,
                grouping: Some(super::ExportGrouping::None),
                include_speaker_names: true,
            },
            grouping: super::ExportGrouping::None,
            transcription: "He said yes.".to_string(),
            summary: "Draft summary".to_string(),
            faqs: String::new(),
            content: "[00:00] Alice: He said yes.".to_string(),
            segments,
            document,
        };

        let preview =
            super::render_prepared_artifact_preview(&prepared).expect("JSON preview should render");
        let temp = tempdir().expect("tempdir");
        let destination = temp.path().join("segments.json");
        super::write_prepared_artifact_export(&destination, &prepared)
            .expect("JSON export should write");
        let written = std::fs::read_to_string(destination).expect("JSON output");
        let payload: serde_json::Value = serde_json::from_str(&written).expect("rich JSON object");

        assert_eq!(preview.content, written);
        assert!(payload.is_object());
        assert_eq!(payload["summary"], "Draft summary");
        assert_eq!(payload["segments"][0]["start_seconds"], 0.125);
        assert_eq!(payload["segments"][0]["end_seconds"], 2.75);
        assert_eq!(payload["document_title"], "Transcript of Meeting");
    }

    #[test]
    fn export_format_validation_matches_the_ui_matrix() {
        assert!(super::validate_export_combination(
            ExportStyle::Transcript,
            super::ExportFormat::Pdf,
        )
        .is_ok());
        assert!(super::validate_export_combination(
            ExportStyle::Subtitles,
            super::ExportFormat::Vtt,
        )
        .is_ok());
        assert!(super::validate_export_combination(
            ExportStyle::Segments,
            super::ExportFormat::Json,
        )
        .is_ok());
        assert!(super::validate_export_combination(
            ExportStyle::Transcript,
            super::ExportFormat::Json,
        )
        .is_err());
        assert!(super::validate_export_combination(
            ExportStyle::Subtitles,
            super::ExportFormat::Pdf,
        )
        .is_err());
    }

    #[test]
    fn pdf_wraps_embeds_unicode_font_and_paginates() {
        let temp = tempdir().expect("tempdir");
        let destination = temp.path().join("unicode-long.pdf");
        let unicode_line =
            "Città naïve Überprüfung Ελληνικά Кириллица: una riga lunga mantiene ogni parola.";
        let body = std::iter::repeat_n(unicode_line, 180)
            .collect::<Vec<_>>()
            .join("\n");
        let document = super::ExportDocument {
            title: "Trascrizione Unicode".to_string(),
            sections: vec![super::ExportDocumentSection {
                title: "Trascrizione".to_string(),
                body,
                styled_lines: None,
            }],
        };

        super::export_pdf(&destination, &document).expect("PDF export");
        let bytes = std::fs::read(&destination).expect("PDF bytes");
        let serialized = String::from_utf8_lossy(&bytes);
        let pdf = lopdf::Document::load_mem(&bytes).expect("parse generated PDF");
        let pages = pdf.get_pages().keys().copied().collect::<Vec<_>>();

        assert!(bytes.starts_with(b"%PDF"));
        assert!(serialized.contains("/ToUnicode"));
        assert!(pages.len() >= 2);
        assert!(include_str!("../../resources/licenses/NotoSans-OFL.txt")
            .contains("SIL OPEN FONT LICENSE Version 1.1"));
    }

    #[test]
    fn pdf_line_wrapper_does_not_drop_words() {
        let line = "This transcript line is intentionally long so the PDF renderer wraps it into multiple visual lines without dropping words.";
        let wrapped = super::wrap_pdf_text_line(line, 36);

        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|part| part.chars().count() <= 36));
        assert_eq!(wrapped.join(" "), line);
    }

    #[test]
    fn export_document_localizes_title_and_includes_summary_and_faqs() {
        let mut artifact = sample_artifact("Linea uno");
        artifact.title = "Riunione team".to_string();
        artifact.summary = "Sintesi breve".to_string();
        artifact.faqs = "D: Chi segue?\nR: Marta.".to_string();

        let segments = vec![super::ExportSegment {
            time: "00:00".to_string(),
            line: "Linea uno".to_string(),
            start_seconds: None,
            end_seconds: None,
            speaker_id: None,
            speaker_label: None,
        }];

        let document = build_export_document(
            "it",
            &artifact.title,
            &artifact.raw_transcript,
            &artifact.summary,
            &artifact.faqs,
            &artifact.metadata,
            &segments,
            ExportStyle::Segments,
            true,
            false,
            &BTreeMap::new(),
        );

        assert_eq!(document.title, "Trascrizione di Riunione team");
        assert_eq!(document.sections[0].title, "Segmenti");
        assert_eq!(document.sections[1].title, "Riassunto");
        assert_eq!(document.sections[2].title, "Domande frequenti");

        let plain_text = render_plain_text_document(&document);
        assert!(plain_text.contains("Trascrizione di Riunione team"));
        assert!(plain_text.contains("Segmenti\n[00:00] Linea uno"));
        assert!(plain_text.contains("Riassunto\nSintesi breve"));
        assert!(plain_text.contains("Domande frequenti\nD: Chi segue?\nR: Marta."));
    }

    #[test]
    fn export_writers_create_all_supported_formats() {
        let temp = tempdir().expect("tempdir");
        let mut artifact = sample_artifact_with_timeline("fallback transcript");
        artifact.summary = "Short summary".to_string();
        artifact.faqs = "Q: Next?\nA: Follow up.".to_string();
        let segments = build_export_segments(&artifact, "fallback transcript");
        let mut speaker_colors = BTreeMap::new();
        speaker_colors.insert("speaker_1".to_string(), "#123456".to_string());
        let document = build_export_document(
            "en",
            &artifact.title,
            &artifact.raw_transcript,
            &artifact.summary,
            &artifact.faqs,
            &artifact.metadata,
            &segments,
            ExportStyle::Segments,
            true,
            true,
            &speaker_colors,
        );
        let export_content = build_export_content(
            &artifact.raw_transcript,
            &segments,
            ExportStyle::Segments,
            true,
            true,
        );
        let prepared = super::PreparedArtifactExport {
            artifact: artifact.clone(),
            format: super::ExportFormat::Json,
            language: "en",
            style: ExportStyle::Segments,
            options: super::ExportOptions {
                include_timestamps: true,
                grouping: Some(super::ExportGrouping::None),
                include_speaker_names: true,
            },
            grouping: super::ExportGrouping::None,
            transcription: artifact.raw_transcript.clone(),
            summary: artifact.summary.clone(),
            faqs: artifact.faqs.clone(),
            segments: segments.clone(),
            content: export_content.clone(),
            document: document.clone(),
        };

        let txt_path = temp.path().join("transcript.txt");
        super::export_txt(&txt_path, &render_plain_text_document(&document)).expect("txt export");
        assert!(std::fs::read_to_string(&txt_path)
            .expect("txt contents")
            .contains("[00:12] Alice: Alice opens the meeting."));

        let md_path = temp.path().join("transcript.md");
        super::export_md(&md_path, &render_markdown_document(&document)).expect("md export");
        assert!(std::fs::read_to_string(&md_path)
            .expect("md contents")
            .contains("## Segments"));

        let csv_path = temp.path().join("segments.csv");
        super::export_txt(&csv_path, &super::render_csv_content("en", &segments, true))
            .expect("csv export");
        assert!(std::fs::read_to_string(&csv_path)
            .expect("csv contents")
            .contains("Transcript;Speaker"));

        let html_path = temp.path().join("transcript.html");
        super::export_html(&html_path, "en", &document).expect("html export");
        assert!(std::fs::read_to_string(&html_path)
            .expect("html contents")
            .contains("<!doctype html>"));
        assert!(std::fs::read_to_string(&html_path)
            .expect("html contents")
            .contains("<span style=\"color:#123456\">[00:12] Alice:"));

        let json_path = temp.path().join("transcript.json");
        super::export_txt(
            &json_path,
            &super::render_json_content(&prepared).expect("json rendering"),
        )
        .expect("json export");
        let json_payload =
            std::fs::read_to_string(&json_path).expect("json contents should be readable");
        assert!(json_payload.contains("\"style\": \"segments\""));
        assert!(json_payload.contains("\"speaker_label\": \"Alice\""));

        let docx_path = temp.path().join("transcript.docx");
        super::export_docx(&docx_path, &document).expect("docx export");
        let mut archive =
            zip::ZipArchive::new(std::fs::File::open(&docx_path).expect("open generated DOCX"))
                .expect("DOCX zip archive");
        let mut document_xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("DOCX document XML")
            .read_to_string(&mut document_xml)
            .expect("read DOCX document XML");
        assert!(document_xml.contains("Alice opens the meeting."));
        assert!(document_xml.contains("Short summary"));

        let pdf_path = temp.path().join("transcript.pdf");
        super::export_pdf(&pdf_path, &document).expect("pdf export");
        assert!(std::fs::metadata(&pdf_path).expect("pdf metadata").len() > 0);

        let srt_path = temp.path().join("subtitles.srt");
        super::export_txt(
            &srt_path,
            &build_export_content(
                &artifact.raw_transcript,
                &segments,
                ExportStyle::Subtitles,
                true,
                true,
            ),
        )
        .expect("srt export");
        assert!(std::fs::read_to_string(&srt_path)
            .expect("srt contents")
            .contains("00:00:12,400 --> 00:00:24,900"));

        let vtt_path = temp.path().join("subtitles.vtt");
        super::export_txt(
            &vtt_path,
            &super::build_vtt_content(&segments, &artifact.raw_transcript, true),
        )
        .expect("vtt export");
        assert!(std::fs::read_to_string(&vtt_path)
            .expect("vtt contents")
            .starts_with("WEBVTT"));
    }

    #[test]
    fn summary_instructions_keep_required_controls_even_with_custom_prompt() {
        let instructions = build_summary_instructions(
            &SummarizeArtifactPayload {
                id: "artifact-1".to_string(),
                language: "it".to_string(),
                context: ArtifactAiContextOptions {
                    include_timestamps: false,
                    include_speakers: true,
                },
                sections: true,
                bullet_points: false,
                action_items: true,
                key_points_only: true,
                custom_prompt: Some("Focus on hiring decisions.".to_string()),
            },
            "it",
        );

        assert!(instructions.contains("The entire output must be in Italian."));
        assert!(instructions.contains("Do not include timestamps in the final summary."));
        assert!(instructions.contains("Attribute statements to named speakers"));
        assert!(instructions.contains("Focus on hiring decisions."));
    }

    #[test]
    fn summary_instructions_default_to_detailed_prose() {
        let instructions = build_summary_instructions(
            &SummarizeArtifactPayload {
                id: "artifact-2".to_string(),
                language: "en".to_string(),
                context: ArtifactAiContextOptions {
                    include_timestamps: false,
                    include_speakers: false,
                },
                sections: true,
                bullet_points: false,
                action_items: true,
                key_points_only: false,
                custom_prompt: None,
            },
            "en",
        );

        assert!(instructions.contains("Write a detailed, self-contained brief in English."));
        assert!(instructions.contains("cover all major topics with supporting details, technical explanations, examples, numbers"));
        assert!(instructions.contains("Do not settle for a terse recap"));
        assert!(instructions.contains("Do not include timestamps in the final summary."));
    }

    #[test]
    fn extract_low_confidence_spans_prioritizes_suspect_regions() {
        let artifact = sample_artifact_with_confidence_timeline(
            "Questo quesito riguarda Cheras Tuner e GSM Scheme.",
        );

        let spans = extract_low_confidence_spans(&artifact);

        assert!(!spans.is_empty());
        assert_eq!(spans[0].suspect_text, "Cheras Tuner");
        assert!(spans.iter().any(|span| span.suspect_text == "Cheras Tuner"));
        assert!(spans.iter().any(|span| span.suspect_text == "GSM Scheme"));
        assert!(spans
            .iter()
            .all(|span| span.excerpt.contains(&span.suspect_text)));
    }

    #[test]
    fn confidence_aware_optimize_prompt_includes_low_confidence_hints() {
        let artifact = sample_artifact_with_confidence_timeline(
            "Questo quesito riguarda Cheras Tuner e GSM Scheme.",
        );

        let prompt = build_confidence_aware_optimize_prompt(
            &artifact,
            Some("Preserve technical terminology.".to_string()),
        )
        .expect("prompt should be generated");

        assert!(prompt.contains("Preserve technical terminology."));
        assert!(prompt.contains("Confidence-aware guidance"));
        assert!(prompt.contains("Cheras Tuner"));
        assert!(prompt.contains("GSM Scheme"));
    }

    #[test]
    fn summary_prompts_require_dense_coverage_for_direct_and_chunked_paths() {
        let direct_prompt =
            build_direct_summary_prompt("Technical transcript", "Write in English with sections.");
        assert!(direct_prompt.contains("dense, polished document"));
        assert!(direct_prompt.contains("technical terms, examples, constraints, and decisions"));

        let chunk_prompt =
            build_chunk_note_prompt(1, 3, "Write in English with sections.", "Chunk transcript");
        assert!(chunk_prompt.contains("technical terminology"));
        assert!(chunk_prompt.contains("Examples, evidence, or concrete scenarios"));

        let synthesis_prompt = build_summary_synthesis_prompt(
            "Chunk 1 notes:\nDetails",
            "Write in English with sections.",
        );
        assert!(synthesis_prompt.contains("dense, polished document"));
        assert!(synthesis_prompt.contains("technical terms, examples, constraints, and decisions"));
    }

    #[test]
    fn detects_context_window_errors() {
        let error = ApplicationError::PostProcessing(
            "Foundation bridge error: Exceeded model context window size".to_string(),
        );
        assert!(is_context_window_error(&error));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_uses_single_pass_for_short_transcripts() {
        let enhancer = TrackingEnhancer::default();
        let transcript = "Alice reviews the roadmap and confirms the launch checklist is complete.";

        let optimized = optimize_with_rag(&enhancer, transcript, "en")
            .await
            .expect("optimization should succeed");

        assert_eq!(optimized, transcript);
        assert_eq!(enhancer.optimize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_chunks_large_transcripts_and_merges_them() {
        let enhancer = Arc::new(TrackingEnhancer::new(true, 1, 0));
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(450);

        let optimized = optimize_with_rag(enhancer.as_ref(), &transcript, "en")
            .await
            .expect("optimization should succeed");

        assert!(!optimized.trim().is_empty());
        assert!(enhancer.optimize_calls.load(Ordering::SeqCst) > 1);
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 0);
        assert!(!optimized.contains("[Section"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_retries_smaller_chunks_after_context_window() {
        let enhancer = TrackingEnhancer::with_optimize_context_limit(true, 1, 5_500, 1_300);
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(230);

        let optimized = optimize_with_rag(&enhancer, &transcript, "en")
            .await
            .expect("optimization should retry with smaller chunks and succeed");

        assert!(!optimized.trim().is_empty());
        assert!(enhancer.optimize_calls.load(Ordering::SeqCst) > 2);
        assert!(!optimized.contains("[Section"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_returns_specific_overflow_after_all_chunk_budgets_fail() {
        let enhancer = TrackingEnhancer::with_optimize_context_limit(true, 1, 5_500, 20);
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(20);

        let error = optimize_with_rag(&enhancer, &transcript, "en")
            .await
            .expect_err("optimization should fail after exhausting smaller chunks");

        assert!(matches!(error, ApplicationError::PostProcessing(_)));
        assert!(error.to_string().contains("optimizing the transcript"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_preserves_distributed_short_connectives_through_chunking() {
        // The 7th anchor invites the LLM to insert short connectives
        // throughout the transcript, not just at the end. When a
        // long transcript is chunked, each chunk's optimization can
        // independently add a short connective. The safety net's
        // early-rejection branch must let these DISTRIBUTED small
        // additions survive the final stitched safety net step
        // (not just single tail additions).
        //
        // The previous safety net used `is_token_subsequence` to
        // detect additive changes. That check fires whenever the
        // source is a subsequence of the candidate, which is true
        // for any additive change (tail or distributed). It would
        // revert the distributed additions at the final stitched
        // step because the accumulated additions exceed
        // MAX_CONTEXTUAL_INSERT_TOKENS.
        //
        // The fix replaces the subsequence check with a true
        // `is_tail_addition` check: the candidate must start with
        // the source tokens, and any extra tokens must come AFTER
        // the source. Distributed additions (connectives in the
        // middle of each chunk) are NOT tail additions, so the
        // early-rejection branch falls through to the multiset-
        // overlap and bigram-overlap checks, which accept the
        // small distributed edits and still reject truly off-topic
        // content.
        let enhancer = TrackingEnhancer::with_hallucinations(false, 1, 0, true, false);
        // Use a long transcript that forces chunking (>2600 chars
        // target). The enhancer appends " added commentary" (2
        // tokens) to EACH chunk, so the distributed additions
        // accumulate in the merged result.
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(450);
        let source_word_count = transcript.split_whitespace().count();

        let optimized = optimize_with_rag(&enhancer, &transcript, "en")
            .await
            .expect("optimization should succeed");

        // Verify the transcript was actually chunked (multiple
        // optimize calls). Without chunking, the test would be
        // trivially true via the single-pass path.
        let chunk_count = enhancer.optimize_calls.load(Ordering::SeqCst);
        assert!(
            chunk_count > 1,
            "transcript should have been chunked into multiple calls, got {chunk_count}"
        );

        // The distributed " added commentary" connective must
        // survive the final stitched safety net step. Each chunk
        // gets one appendage, so the merged result has at least
        // `chunk_count` occurrences (the merge may dedupe
        // overlapping tails, but cannot remove all of them).
        let added_count = optimized.matches("added commentary").count();
        assert!(
            added_count >= 2,
            "distributed connectives must survive chunking + merge + safety net,              got {added_count} occurrences of the additive phrase in {chunk_count} chunks"
        );

        // No source word should be lost. The merged result is
        // made of substrings of the source plus the additive
        // tokens, so every source word must still appear in the
        // optimized output.
        let optimized_lower = optimized.to_lowercase();
        for word in transcript.split_whitespace() {
            let needle = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            if needle.is_empty() {
                continue;
            }
            assert!(
                optimized_lower.contains(&needle),
                "source word {needle:?} should be preserved in optimized output"
            );
        }

        // The optimized output should be slightly longer than the
        // source (each chunk added 2 tokens).
        let optimized_word_count = optimized.split_whitespace().count();
        assert!(
            optimized_word_count > source_word_count,
            "optimized output should be longer than source ({optimized_word_count} vs {source_word_count})              when each chunk adds 2 tokens"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_accepts_short_addition_from_enhancer() {
        // The 7th anchor in the strengthened prompt invites the LLM to
        // make an implicit logical connection explicit by inserting a
        // short connective. MAX_CONTEXTUAL_INSERT_TOKENS now allows
        // the small additive change (" added commentary" = 2 added
        // tokens) to flow through the safety net so the optimization
        // is preserved end-to-end.
        let enhancer = TrackingEnhancer::with_hallucinations(false, 1, 0, true, false);
        let transcript = "Alice reviews the roadmap and confirms the launch checklist is complete.";

        let optimized = optimize_with_rag(&enhancer, transcript, "en")
            .await
            .expect("optimization should succeed");

        assert_ne!(optimized, transcript);
        assert!(optimized.contains("added commentary"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_rejects_large_addition_from_enhancer() {
        // The safety net still reverts large additive changes. Here
        // the enhancer appends 4 extra tokens, which exceeds
        // MAX_CONTEXTUAL_INSERT_TOKENS = 2 and the early-rejection
        // branch fires. The transcript must come back unchanged.
        let enhancer = TrackingEnhancer::with_long_hallucinations(false, 1, 0, true, false);
        let transcript = "Alice reviews the roadmap and confirms the launch checklist is complete.";

        let optimized = optimize_with_rag(&enhancer, transcript, "en")
            .await
            .expect("optimization should succeed");

        assert_eq!(optimized, transcript);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_accepts_substantial_rewrite_for_short_transcript() {
        // Short transcript goes through the single-pass path. The
        // simulated substantial rewrite must flow through to the final
        // result (i.e. the relaxed safety net accepts it).
        let enhancer = TrackingEnhancer::with_substantial_rewrite(false, 1, 0);
        let transcript =
            "uh allora io dico che è importante capire il problema prima di iniziare a programmare";

        let optimized = optimize_with_rag(&enhancer, transcript, "it")
            .await
            .expect("optimization should succeed");

        assert_eq!(enhancer.optimize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 0);
        // Filler words ("uh", "allora") must be removed by the rewrite.
        // The simulation does not drop the self-reference phrase
        // "io dico che" (an LLM following the new prompt would); we
        // just assert the safety net allowed the rewrite through.
        assert!(!optimized.contains(" uh "));
        assert!(!optimized.contains(" allora "));
        assert!(!optimized.starts_with("uh"));
        assert_ne!(optimized, transcript);
        // Capitalization and trailing punctuation are added.
        let first_char = optimized.chars().next().expect("non-empty");
        assert!(first_char.is_ascii_uppercase());
        let last_char = optimized.chars().last().expect("non-empty");
        assert!(matches!(last_char, '.' | '?' | '!'));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn optimize_with_rag_chunks_large_transcripts_with_substantial_rewrite() {
        // Large transcript: each chunk gets the simulated substantial
        // rewrite, then they are merged. The relaxed safety net must
        // accept the merged result even though the rewrites no longer
        // share exact token overlap with the source.
        let enhancer = Arc::new(TrackingEnhancer::with_substantial_rewrite(true, 1, 0));
        let transcript = "uh alice opens the meeting. ehm bob reviews the launch checklist.             allora we need to confirm the deployment plan for next quarter.             tipo the next step is to update the documentation and share it with the team.             beh the deadline is end of next week and we have to ship."
            .repeat(120);

        let optimized = optimize_with_rag(enhancer.as_ref(), &transcript, "en")
            .await
            .expect("optimization should succeed");

        assert!(!optimized.trim().is_empty());
        assert!(enhancer.optimize_calls.load(Ordering::SeqCst) > 1);
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 0);
        // Filler ("uh", "ehm", "allora", "tipo", "beh") should be removed
        // from the merged result.
        assert!(!optimized.contains(" uh "));
        assert!(!optimized.contains(" ehm "));
        assert!(!optimized.contains(" allora "));
        assert!(!optimized.contains(" tipo "));
        assert!(!optimized.contains(" beh "));
        assert!(!optimized.contains("[Section"));
    }

    #[test]
    fn confidence_aware_optimize_prompt_preserves_substantial_rewrite_guidance() {
        // Verify the confidence-aware guidance text tells the LLM to
        // apply the same level of substantive cleanup everywhere (not
        // only inside the suspect spans), and that it carries the same
        // anti-timid framing and concrete before/after example that
        // the adapter prompts use.
        let artifact = sample_artifact_with_confidence_timeline(
            "Questo quesito riguarda Cheras Tuner e GSM Scheme.",
        );

        let prompt =
            build_confidence_aware_optimize_prompt(&artifact, Some("User prompt".to_string()))
                .expect("prompt should be generated");

        assert!(prompt.contains("substantive"));
        assert!(prompt.contains("syntactic, logical, and contextual cleanup"));
        assert!(prompt.contains("apply the same level of substantive"));
        assert!(prompt.contains("fully cleaned transcript, not a mostly-original transcript"));
        assert!(prompt.contains("prioritize aggressive local repair"));
        assert!(prompt.contains("Cheras Tuner"));
        // Anti-timid framing: the confidence-aware section must reinforce
        // that the suspect spans are priorities, not a fence, and that
        // leaving 90% of the original words in place is not optimization.
        assert!(
            prompt.contains("Do not be timid"),
            "confidence-aware section must contain anti-timid framing"
        );
        assert!(
            prompt.contains("90% of the original words"),
            "confidence-aware section must anchor the expected level of change"
        );
        assert!(
            prompt.contains("the speaker's TONE"),
            "confidence-aware section must preserve the speaker's tone"
        );
        assert!(
            prompt.contains("careful editor"),
            "confidence-aware section must invoke the careful-editor framing"
        );
        assert!(
            prompt.contains("Example of the expected level of rewriting"),
            "confidence-aware section must include the concrete before/after example"
        );
        // The 2nd example demonstrates the connective case enabled by the
        // is_tail_addition relaxation: a short connective (1 token)
        // inserted to make an implicit logical relationship explicit.
        assert!(
            prompt.contains("Another example of the expected level of rewriting"),
            "confidence-aware section must include the connective example"
        );
        assert!(
            prompt.contains("perché il team ha lavorato bene"),
            "confidence-aware section must show the connective output"
        );
        assert!(
            prompt.contains("Another example of topic-aware rewriting"),
            "confidence-aware section must include the 3rd example marker"
        );
        assert!(
            prompt.contains("i requisiti del progetto software"),
            "confidence-aware section must demonstrate the topic-aware rewrite"
        );
    }

    #[test]
    fn confidence_aware_optimize_prompt_anchors_topic_and_contextual_logic() {
        // Regression guard: the confidence-aware guidance text must also
        // ask the LLM to understand the topic of the transcript and to
        // make implicit logical connections explicit when the surrounding
        // context makes the intended meaning clear. Without this anchor,
        // the confidence-aware path would still be able to fall back to a
        // "mostly-original transcript with edits only in highlighted
        // places" output, even though the user's stated goal is broader.
        let artifact = sample_artifact_with_confidence_timeline(
            "Questo quesito riguarda Cheras Tuner e GSM Scheme.",
        );

        let prompt =
            build_confidence_aware_optimize_prompt(&artifact, Some("User prompt".to_string()))
                .expect("prompt should be generated");

        assert!(
            prompt.contains("Understand the topic being discussed"),
            "confidence-aware section must ask the LLM to understand the topic"
        );
        assert!(
            prompt.contains("surrounding context makes the intended meaning clear"),
            "confidence-aware section must anchor the topic-aware disambiguation rule"
        );
        assert!(
            prompt.contains("make that connection explicit"),
            "confidence-aware section must anchor the explicit-connective rule"
        );
        assert!(
            prompt.contains("vague references like 'la cosa di cui parlavamo'"),
            "confidence-aware section must anchor the topic-aware substitution rule"
        );
    }

    #[test]
    fn confidence_aware_optimize_prompt_demonstrates_short_connective_case() {
        // The 2nd example in the confidence-aware prompt demonstrates
        // the connective case enabled by the is_tail_addition
        // relaxation. See the parallel tests in the adapter files for
        // the full rationale.
        let artifact = sample_artifact_with_confidence_timeline(
            "Questo quesito riguarda Cheras Tuner e GSM Scheme.",
        );

        let prompt =
            build_confidence_aware_optimize_prompt(&artifact, Some("User prompt".to_string()))
                .expect("prompt should be generated");

        assert!(
            prompt.contains("Another example of the expected level of rewriting"),
            "confidence-aware section must include the 2nd example marker"
        );
        assert!(
            prompt.contains("perché il team ha lavorato bene"),
            "confidence-aware section must demonstrate the connective output"
        );
        assert!(
            prompt.contains("Another example of topic-aware rewriting"),
            "confidence-aware section must include the 3rd example marker"
        );
        assert!(
            prompt.contains("i requisiti del progetto software"),
            "confidence-aware section must demonstrate the topic-aware rewrite"
        );
    }

    #[test]
    fn confidence_aware_optimize_prompt_demonstrates_topic_aware_rewrite() {
        // The 3rd example in the confidence-aware prompt demonstrates
        // the topic-aware rewrite case. See the parallel tests in the
        // adapter files for the full rationale.
        let artifact = sample_artifact_with_confidence_timeline(
            "Questo quesito riguarda Cheras Tuner e GSM Scheme.",
        );

        let prompt =
            build_confidence_aware_optimize_prompt(&artifact, Some("User prompt".to_string()))
                .expect("prompt should be generated");

        assert!(
            prompt.contains("Another example of topic-aware rewriting"),
            "confidence-aware section must include the 3rd example marker"
        );
        assert!(
            prompt.contains("i requisiti del progetto software"),
            "confidence-aware section must demonstrate the topic-aware rewrite"
        );
    }

    #[test]
    fn confidence_aware_optimize_prompt_without_spans_falls_back_to_user_prompt() {
        // Without low-confidence spans the helper returns the user prompt
        // verbatim, leaving the substantial-rewrite language to be
        // appended by the adapter as Additional cleanup rules.
        let artifact = sample_artifact("Plain transcript with no timeline.");

        let prompt = build_confidence_aware_optimize_prompt(
            &artifact,
            Some("User prompt only.".to_string()),
        )
        .expect("prompt should be generated");

        assert_eq!(prompt, "User prompt only.");
        assert!(!prompt.contains("Confidence-aware guidance"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_with_rag_uses_single_pass_for_short_transcripts() {
        let enhancer = TrackingEnhancer::default();

        let summary = summarize_with_rag(
            &enhancer,
            "Alice reviews the roadmap and confirms the launch checklist is complete.",
            "Write a concise English summary.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 1);

        let prompts = enhancer.prompts.lock().expect("prompt log lock poisoned");
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Full transcript:"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_with_rag_processes_chunk_notes_with_bounded_concurrency() {
        let enhancer = Arc::new(TrackingEnhancer::default());
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(450);

        let summary = summarize_with_rag(
            enhancer.as_ref(),
            &transcript,
            "Write a detailed English summary with sections.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        assert!(enhancer.ask_calls.load(Ordering::SeqCst) >= 3);
        assert!(enhancer.max_active_calls.load(Ordering::SeqCst) > 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_with_rag_prefers_single_pass_for_foundation_style_enhancer() {
        let enhancer = TrackingEnhancer::new(true, 1, 0);
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(450);

        let summary = summarize_with_rag(
            &enhancer,
            &transcript,
            "Write a detailed English summary with sections.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 1);

        let prompts = enhancer.prompts.lock().expect("prompt log lock poisoned");
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Full transcript:"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_with_rag_falls_back_to_chunking_after_direct_context_error() {
        let enhancer = Arc::new(TrackingEnhancer::new(true, 1, 1));
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(450);

        let summary = summarize_with_rag(
            enhancer.as_ref(),
            &transcript,
            "Write a detailed English summary with sections.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        assert!(enhancer.ask_calls.load(Ordering::SeqCst) >= 4);
        assert_eq!(enhancer.max_active_calls.load(Ordering::SeqCst), 1);

        let prompts = enhancer.prompts.lock().expect("prompt log lock poisoned");
        assert!(prompts
            .first()
            .is_some_and(|prompt| prompt.contains("Full transcript:")));
        assert!(prompts
            .iter()
            .any(|prompt| prompt.contains("Transcript chunk:")));
    }

    #[test]
    fn trimmed_audio_output_metadata_reports_duration_and_file_size() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("trimmed.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::create(&path, spec).expect("create wav writer");
        for _ in 0..32_000 {
            writer.write_sample::<i16>(0).expect("write sample");
        }
        writer.finalize().expect("finalize wav");

        let (duration_seconds, file_size_bytes) =
            trimmed_audio_output_metadata(&path).expect("metadata should parse");

        assert!((duration_seconds - 2.0).abs() < 0.02);
        assert!(file_size_bytes > 0);
    }

    #[test]
    fn validate_trimmed_audio_output_rejects_empty_and_too_short_files() {
        let empty_error = validate_trimmed_audio_output(1.0, 0)
            .expect_err("empty trimmed file should be rejected");
        assert!(empty_error.message.contains("trimmed audio file is empty"));

        let short_error =
            validate_trimmed_audio_output(MIN_TRIMMED_AUDIO_DURATION_SECONDS - 0.1, 128)
                .expect_err("too-short trimmed file should be rejected");
        assert!(short_error.message.contains("trimmed audio is too short"));
    }

    #[test]
    fn diarization_timeline_segments_clear_existing_speakers_before_rerun() {
        let artifact = sample_artifact_with_timeline("Alice opens. Bob confirms.");

        let segments = timeline_segments_for_diarization(&artifact)
            .expect("timeline should parse for diarization rerun");

        assert_eq!(segments.len(), 2);
        assert!(segments.iter().all(|segment| segment.speaker_id.is_none()));
        assert!(segments
            .iter()
            .all(|segment| segment.speaker_label.is_none()));
    }

    #[test]
    fn manual_timeline_edit_metadata_is_versioned_and_counted() {
        let first = next_timeline_manual_edit_metadata(None);
        let second = next_timeline_manual_edit_metadata(Some(&first));
        let first_value: serde_json::Value = serde_json::from_str(&first).expect("first metadata");
        let second_value: serde_json::Value =
            serde_json::from_str(&second).expect("second metadata");

        assert_eq!(
            first_value["version"].as_str(),
            Some("timeline_manual_edits_v1")
        );
        assert_eq!(first_value["manual_edit_count"].as_u64(), Some(1));
        assert_eq!(second_value["manual_edit_count"].as_u64(), Some(2));
        assert!(second_value["last_edited_at"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
    }

    #[test]
    fn manual_timeline_edit_provenance_is_the_diarization_guard() {
        let mut artifact = sample_artifact("transcript");
        assert!(!has_timeline_manual_edits(&artifact));

        artifact.metadata.insert(
            "timeline_manual_edits_v1".to_string(),
            next_timeline_manual_edit_metadata(None),
        );
        assert!(has_timeline_manual_edits(&artifact));
    }

    #[tokio::test]
    async fn run_cancellable_returns_cancelled_without_awaiting_pending_operation() {
        let token = CancellationToken::new();
        token.cancel();

        let result = run_cancellable(&token, async {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Ok::<(), ApplicationError>(())
        })
        .await;

        assert!(matches!(result, Err(ApplicationError::Cancelled)));
    }
}
