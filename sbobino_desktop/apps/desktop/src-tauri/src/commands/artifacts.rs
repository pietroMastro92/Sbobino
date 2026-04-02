use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use chrono::Utc;
use docx_rs::{Docx, Paragraph, Run};
use futures_util::stream::{self, StreamExt};
use printpdf::{
    ops::PdfPage, text::TextItem, units::Pt, BuiltinFont, Color, Mm, Op, PdfDocument, Rgb,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;
use uuid::Uuid;

use sbobino_application::{ApplicationError, ArtifactQuery, TranscriptEnhancer};
use sbobino_domain::{
    constrain_transcript_edit, merge_optimized_transcript_sections,
    minimize_transcript_repetitions, ArtifactKind, PromptTask, TranscriptArtifact,
};

use crate::{
    ai_support::{missing_ai_provider_command_error, run_with_enhancer_fallback},
    commands::emotion_analysis::{
        analyze_emotions_with_enhancers, EmotionAnalysisInput, EmotionAnalysisOptions,
    },
    error::CommandError,
    state::AppState,
};

const MIN_TRIMMED_AUDIO_DURATION_SECONDS: f64 = 1.5;
const SPEAKER_COLOR_PALETTE: &[&str] = &[
    "#4F7CFF", "#EC6A5E", "#27A376", "#B06BF2", "#D88B15", "#1293A5", "#E255A1", "#6C7A2D",
];

fn default_true() -> bool {
    true
}

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

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ArtifactAiContextOptions {
    #[serde(default = "default_true")]
    pub include_timestamps: bool,
    #[serde(default)]
    pub include_speakers: bool,
}

impl Default for ArtifactAiContextOptions {
    fn default() -> Self {
        Self {
            include_timestamps: true,
            include_speakers: false,
        }
    }
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
}

#[derive(Debug, Deserialize)]
pub struct ChatArtifactPayload {
    pub id: String,
    pub prompt: String,
    #[serde(flatten)]
    pub context: ArtifactAiContextOptions,
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

const CHAT_CONTEXT_BUDGETS: &[(usize, usize)] = &[(8, 7600), (6, 5200), (4, 3400), (2, 2000)];
const CHAT_CHUNK_TARGET_CHARS: usize = 900;
const CHAT_CHUNK_OVERLAP_WORDS: usize = 24;
const OPTIMIZE_CHUNK_TARGET_CHARS: usize = 2600;
const OPTIMIZE_CHUNK_OVERLAP_WORDS: usize = 28;
const OPTIMIZE_CHUNK_CONCURRENCY_LIMIT: usize = 3;
const SUMMARY_CHUNK_TARGET_CHARS: usize = 4000;
const SUMMARY_CHUNK_OVERLAP_WORDS: usize = 30;
const SUMMARY_CHUNK_CONCURRENCY_LIMIT: usize = 3;
const SUMMARY_SYNTHESIS_BUDGETS: &[usize] = &[12_000, 8_000, 5_000, 3_000];
const LOW_CONFIDENCE_WORD_THRESHOLD: f32 = 0.58;
const LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD: f32 = 0.72;
const LOW_CONFIDENCE_CONTEXT_RADIUS_WORDS: usize = 3;
const MAX_LOW_CONFIDENCE_PROMPT_SPANS: usize = 10;
const SUMMARY_CONTEXT_OVERFLOW_MESSAGE: &str =
    "Exceeded model context window size. The app now uses chunked retrieval, but this request is still too large. Try a shorter custom prompt or fewer summary constraints.";

#[derive(Debug, Clone, Deserialize, Default)]
struct TimelineV2Document {
    #[serde(default)]
    segments: Vec<TimelineV2Segment>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TimelineV2Segment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start_seconds: Option<f32>,
    #[serde(default)]
    end_seconds: Option<f32>,
    #[serde(default)]
    speaker_id: Option<String>,
    #[serde(default)]
    speaker_label: Option<String>,
    #[serde(default)]
    words: Vec<TimelineV2Word>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct TimelineV2Word {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start_seconds: Option<f32>,
    #[serde(default)]
    end_seconds: Option<f32>,
    #[serde(default)]
    confidence: Option<f32>,
}

#[derive(Debug, Clone)]
struct TimelineContextSegment {
    text: String,
    time_label: Option<String>,
    speaker_id: Option<String>,
    speaker_label: Option<String>,
}

#[derive(Debug, Clone)]
struct LowConfidenceSpan {
    suspect_text: String,
    excerpt: String,
    avg_confidence: f32,
    time_label: Option<String>,
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

#[derive(Debug, Clone, Copy, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct ReadAudioFilePayload {
    pub path: String,
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
    state
        .artifact_service
        .update_timeline_v2(&payload.id, &payload.timeline_v2)
        .await
        .map_err(CommandError::from)
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
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let base_transcription = payload
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

    let style = payload.style.unwrap_or(ExportStyle::Transcript);
    let options = payload.options.unwrap_or_default();
    let grouping = options.grouping.unwrap_or(ExportGrouping::None);
    let language = normalize_export_language(payload.language.as_deref());
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let speaker_colors = settings.transcription.speaker_diarization.speaker_colors;
    let segments = match payload.segments {
        Some(entries) if !entries.is_empty() => entries,
        Some(_) if payload.content_override.is_some() => {
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
        &artifact.summary,
        &artifact.faqs,
        &segments,
        style,
        options.include_timestamps,
        options.include_speaker_names,
        &speaker_colors,
    );

    match payload.format {
        ExportFormat::Txt => export_txt(
            destination_path,
            &render_plain_text_document(&export_document),
        )?,
        ExportFormat::Docx => export_docx(destination_path, &export_document)?,
        ExportFormat::Html => export_html(destination_path, language, &export_document)?,
        ExportFormat::Pdf => export_pdf(destination_path, &export_document)?,
        ExportFormat::Json => export_json(
            destination_path,
            &artifact,
            &export_document,
            style,
            grouping,
            options.include_timestamps,
            options.include_speaker_names,
            &segments,
            &export_content,
        )?,
        ExportFormat::Csv => {
            export_csv(destination_path, &segments, options.include_speaker_names)?
        }
        ExportFormat::Md => {
            let content = if style == ExportStyle::Subtitles {
                build_markdown_subtitles_content(
                    &segments,
                    &base_transcription,
                    options.include_speaker_names,
                )
            } else {
                render_markdown_document(&export_document)
            };
            export_md(destination_path, &content)?
        }
        ExportFormat::Srt => export_txt(destination_path, &export_content)?,
        ExportFormat::Vtt => export_txt(
            destination_path,
            &build_vtt_content(
                &segments,
                &base_transcription,
                options.include_speaker_names,
            ),
        )?,
    }

    Ok(ExportArtifactResponse {
        path: destination_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn chat_artifact(
    state: State<'_, AppState>,
    payload: ChatArtifactPayload,
) -> Result<String, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::new("not_found", "artifact not found"))?;

    let enhancer = state
        .runtime_factory
        .build_enhancer_candidates()
        .map_err(|e| CommandError::new("runtime_factory", e))?;
    if enhancer.is_empty() {
        let reason = state
            .runtime_factory
            .ai_capability_status()
            .ok()
            .and_then(|status| status.unavailable_reason);
        return Err(missing_ai_provider_command_error(reason.as_deref()));
    }

    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return Err(CommandError::new(
            "validation",
            "chat prompt cannot be empty",
        ));
    }

    let candidates = build_chat_context_candidates(&artifact, prompt, payload.context);
    run_with_enhancer_fallback(&enhancer, "chat", |active_enhancer| {
        let candidates = candidates.clone();
        Box::pin(async move { ask_with_overflow_fallback(active_enhancer, candidates).await })
    })
    .await
    .map_err(CommandError::from)
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

    let language_code = artifact
        .metadata
        .get("language")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string();

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let optimize_prompt_override = build_confidence_aware_optimize_prompt(
        &artifact,
        settings.prompt_for_task(PromptTask::Optimize),
    );

    let enhancers = state
        .runtime_factory
        .build_enhancer_candidates_with_overrides(None, optimize_prompt_override, None)
        .map_err(|e| CommandError::new("runtime_factory", e))?;

    if enhancers.is_empty() {
        return Ok(text);
    }

    run_with_enhancer_fallback(&enhancers, "optimize transcript", |enhancer| {
        let text = text.clone();
        let language_code = language_code.clone();
        Box::pin(async move { optimize_with_rag(enhancer, &text, &language_code).await })
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

    let transcript = build_artifact_context_transcript(&artifact, payload.context);
    if transcript.trim().is_empty() {
        return Err(CommandError::new(
            "empty_content",
            "no transcription available to summarize",
        ));
    }

    let instructions = build_summary_instructions(&payload);

    run_with_enhancer_fallback(&enhancers, "summarize transcript", |enhancer| {
        let transcript = transcript.clone();
        let instructions = instructions.clone();
        Box::pin(async move { summarize_with_rag(enhancer, &transcript, &instructions).await })
    })
    .await
    .map_err(CommandError::from)
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

    let transcript = build_artifact_context_transcript(&artifact, payload.context);
    if transcript.trim().is_empty() {
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
            transcript,
            timeline_v2_json: artifact.metadata.get("timeline_v2").cloned(),
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

fn effective_transcript(artifact: &TranscriptArtifact) -> String {
    let optimized = artifact.optimized_transcript.trim();
    if !optimized.is_empty() {
        return optimized.to_string();
    }
    artifact.raw_transcript.trim().to_string()
}

fn build_artifact_context_transcript(
    artifact: &TranscriptArtifact,
    context: ArtifactAiContextOptions,
) -> String {
    let timeline_segments = parse_timeline_context_segments(artifact);
    if timeline_segments.is_empty() {
        return effective_transcript(artifact);
    }

    timeline_segments
        .iter()
        .map(|segment| render_timeline_context_segment(segment, context))
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_timeline_context_segments(artifact: &TranscriptArtifact) -> Vec<TimelineContextSegment> {
    let raw = artifact
        .metadata
        .get("timeline_v2")
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    if raw.is_empty() {
        return Vec::new();
    }

    let parsed = match serde_json::from_str::<TimelineV2Document>(raw) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    parsed
        .segments
        .into_iter()
        .filter_map(|segment| {
            let text = segment.text.trim();
            if text.is_empty() {
                return None;
            }

            let time_label = resolve_timeline_segment_seconds(&segment).map(format_mm_ss);
            let speaker_id = normalize_optional_text(segment.speaker_id);
            let speaker_label =
                normalize_optional_text(segment.speaker_label).or_else(|| speaker_id.clone());

            Some(TimelineContextSegment {
                text: text.to_string(),
                time_label,
                speaker_id,
                speaker_label,
            })
        })
        .collect()
}

fn parse_timeline_document(artifact: &TranscriptArtifact) -> Option<TimelineV2Document> {
    let raw = artifact
        .metadata
        .get("timeline_v2")
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    if raw.is_empty() {
        return None;
    }

    serde_json::from_str::<TimelineV2Document>(raw).ok()
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
        "Confidence-aware guidance: Whisper provided word-level confidence scores. Treat the suspect spans below as soft evidence about where ASR mistakes are most likely. Be more willing to locally repair garbled or nonsensical wording inside these spans when the surrounding context makes the intended term highly likely. Outside these spans, stay conservative. If a suspect span is still ambiguous, keep the original wording."
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
        "Low-confidence suspect spans from the original Whisper transcript:\n{low_confidence_lines}"
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

fn resolve_timeline_segment_seconds(segment: &TimelineV2Segment) -> Option<f32> {
    if let Some(start) = segment.start_seconds.filter(|value| value.is_finite()) {
        return Some(start.max(0.0));
    }
    if let Some(end) = segment.end_seconds.filter(|value| value.is_finite()) {
        return Some(end.max(0.0));
    }

    for word in &segment.words {
        if let Some(start) = word.start_seconds.filter(|value| value.is_finite()) {
            return Some(start.max(0.0));
        }
        if let Some(end) = word.end_seconds.filter(|value| value.is_finite()) {
            return Some(end.max(0.0));
        }
    }

    None
}

fn render_timeline_context_segment(
    segment: &TimelineContextSegment,
    context: ArtifactAiContextOptions,
) -> String {
    let mut prefix = String::new();

    if context.include_timestamps {
        if let Some(time_label) = segment.time_label.as_deref() {
            prefix.push_str(&format!("[{time_label}] "));
        }
    }

    if context.include_speakers {
        if let Some(speaker_label) = segment.speaker_label.as_deref() {
            prefix.push_str(speaker_label);
            prefix.push_str(": ");
        }
    }

    format!("{prefix}{}", segment.text.trim())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn format_mm_ss(seconds: f32) -> String {
    let total_seconds = seconds.floor().max(0.0) as u32;
    let mm = total_seconds / 60;
    let ss = total_seconds % 60;
    format!("{mm:02}:{ss:02}")
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

fn build_summary_instructions(payload: &SummarizeArtifactPayload) -> String {
    let mut lines = vec![
        format!(
            "Write a detailed, self-contained brief in {}.",
            language_display_name(&payload.language)
        ),
        format!(
            "The entire output must be in {}.",
            language_display_name(&payload.language)
        ),
        "Produce only the final summary text. Do not add meta-commentary about the summarization process.".to_string(),
        "Assume the reader has not listened to the recording. The summary must stand on its own and preserve the substance of the discussion.".to_string(),
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

    let chunks = chunk_text_by_words(
        &cleaned,
        OPTIMIZE_CHUNK_TARGET_CHARS,
        OPTIMIZE_CHUNK_OVERLAP_WORDS,
    );

    if chunks.is_empty() {
        return Err(ApplicationError::Validation(
            "cannot optimize an empty transcript".to_string(),
        ));
    }

    if chunks.len() == 1 {
        return enhancer
            .optimize(&cleaned, language_code)
            .await
            .map(|optimized| constrain_transcript_edit(&cleaned, &optimized));
    }

    let concurrency_limit = enhancer
        .summary_chunk_concurrency_limit()
        .max(1)
        .min(OPTIMIZE_CHUNK_CONCURRENCY_LIMIT);
    let chunk_concurrency = chunks.len().clamp(1, concurrency_limit);

    let current_sections = stream::iter(chunks.into_iter())
        .map(|chunk| async move {
            enhancer
                .optimize(&chunk, language_code)
                .await
                .map(|optimized| constrain_transcript_edit(&chunk, &optimized))
        })
        .buffered(chunk_concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    let stitched = merge_optimized_transcript_sections(
        &current_sections,
        (OPTIMIZE_CHUNK_OVERLAP_WORDS / 2).max(4),
    );
    if stitched.trim().is_empty() {
        return Ok(cleaned);
    }

    let reduced = constrain_transcript_edit(&cleaned, &stitched);
    if reduced.trim().is_empty() {
        Ok(cleaned)
    } else {
        Ok(reduced)
    }
}

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
        .max(1)
        .min(SUMMARY_CHUNK_CONCURRENCY_LIMIT);
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

#[derive(Debug, Deserialize)]
pub struct TrimRegion {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Deserialize)]
pub struct WriteTrimmedAudioPayload {
    pub input_path: String,
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
    use tokio::process::Command;

    if payload.regions.is_empty() {
        return Err(CommandError::new("trim", "no regions selected"));
    }

    let input = Path::new(&payload.input_path);
    if !input.exists() {
        return Err(CommandError::new(
            "trim",
            format!("input file not found: {}", payload.input_path),
        ));
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
        let result = Command::new(&ffmpeg_path)
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

            let result = Command::new(&ffmpeg_path)
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

        let result = Command::new(&ffmpeg_path)
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
        .and_then(|value| sanitize_speaker_color_value(value))
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

fn export_csv(
    path: &Path,
    segments: &[ExportSegment],
    include_speaker_names: bool,
) -> Result<(), CommandError> {
    let header = if include_speaker_names {
        "Start Timestamp;End Timestamp;Transcript;Speaker"
    } else {
        "Start Timestamp;End Timestamp;Transcript"
    };
    let rows = if segments.is_empty() {
        vec![if include_speaker_names {
            "00:00;00:00;\"\";\"\"".to_string()
        } else {
            "00:00;00:00;\"\"".to_string()
        }]
    } else {
        segments
            .iter()
            .map(|segment| {
                let start_seconds = parse_timestamp_to_seconds(&segment.time);
                let end_time = format_mm_ss_u32(start_seconds + 11);
                let base = format!(
                    "{};{};\"{}\"",
                    segment.time,
                    end_time,
                    segment.line.replace('"', "\"\"")
                );
                if !include_speaker_names {
                    return base;
                }
                let speaker = normalized_export_speaker_label(segment)
                    .unwrap_or_default()
                    .replace('"', "\"\"");
                format!("{base};\"{speaker}\"")
            })
            .collect::<Vec<_>>()
    };

    std::fs::write(path, format!("{header}\n{}", rows.join("\n")))
        .map_err(|e| CommandError::new("export", format!("failed to export csv: {e}")))
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

fn export_json(
    path: &Path,
    artifact: &TranscriptArtifact,
    document: &ExportDocument,
    style: ExportStyle,
    grouping: ExportGrouping,
    include_timestamps: bool,
    include_speaker_names: bool,
    segments: &[ExportSegment],
    content: &str,
) -> Result<(), CommandError> {
    let serialized_segments = if include_speaker_names {
        segments.to_vec()
    } else {
        segments
            .iter()
            .map(|segment| ExportSegment {
                time: segment.time.clone(),
                line: segment.line.clone(),
                speaker_id: None,
                speaker_label: None,
            })
            .collect::<Vec<_>>()
    };
    let payload = json!({
        "id": artifact.id,
        "job_id": artifact.job_id,
        "title": artifact.title,
        "kind": artifact.kind.as_str(),
        "input_path": artifact.input_path,
        "created_at": artifact.created_at.to_rfc3339(),
        "updated_at": artifact.updated_at.to_rfc3339(),
        "style": style,
        "options": {
            "include_timestamps": include_timestamps,
            "grouping": grouping,
            "include_speaker_names": include_speaker_names
        },
        "document_title": document.title,
        "sections": document.sections.iter().map(|section| {
            json!({
                "title": section.title,
                "content": section.body,
            })
        }).collect::<Vec<_>>(),
        "content": content,
        "summary": artifact.summary,
        "faqs": artifact.faqs,
        "segments": serialized_segments,
        "metadata": artifact.metadata,
    });

    let serialized = serde_json::to_string_pretty(&payload)
        .map_err(|e| CommandError::new("export", format!("failed to encode json export: {e}")))?;

    std::fs::write(path, serialized)
        .map_err(|e| CommandError::new("export", format!("failed to export json: {e}")))
}

fn export_pdf(path: &Path, document: &ExportDocument) -> Result<(), CommandError> {
    let mut doc = PdfDocument::new(&document.title);
    let mut pages = Vec::new();
    let mut ops = start_pdf_page_ops(Some(&document.title));
    let mut y = 780.0_f32;
    let body_lines = render_document_body_lines(document);
    let colored_lines = render_document_body_styled_lines(document);

    if body_lines.is_empty() {
        write_pdf_line(&mut ops, "No content available for export.", y, None);
    } else {
        for line in colored_lines {
            if y < 42.0 {
                ops.push(Op::EndTextSection);
                pages.push(PdfPage::new(Mm(210.0), Mm(297.0), ops));
                ops = start_pdf_page_ops(None);
                y = 810.0;
            }

            write_pdf_line(&mut ops, &line.text, y, line.speaker_color.as_deref());
            y -= 14.0;
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

fn start_pdf_page_ops(title: Option<&str>) -> Vec<Op> {
    let mut ops = vec![Op::StartTextSection];

    if let Some(title) = title {
        ops.push(Op::SetFontSizeBuiltinFont {
            size: Pt(20.0),
            font: BuiltinFont::HelveticaBold,
        });
        ops.push(Op::SetTextCursor {
            pos: printpdf::graphics::Point {
                x: Pt(28.0),
                y: Pt(810.0),
            },
        });
        ops.push(Op::WriteTextBuiltinFont {
            items: vec![TextItem::Text(title.to_string())],
            font: BuiltinFont::HelveticaBold,
        });

        ops.push(Op::SetFontSizeBuiltinFont {
            size: Pt(11.0),
            font: BuiltinFont::Helvetica,
        });
    } else {
        ops.push(Op::SetFontSizeBuiltinFont {
            size: Pt(11.0),
            font: BuiltinFont::Helvetica,
        });
    }

    ops
}

fn write_pdf_line(ops: &mut Vec<Op>, line: &str, y: f32, speaker_color: Option<&str>) {
    ops.push(Op::SetTextCursor {
        pos: printpdf::graphics::Point {
            x: Pt(28.0),
            y: Pt(y),
        },
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
    ops.push(Op::WriteTextBuiltinFont {
        items: vec![TextItem::Text(line.to_string())],
        font: BuiltinFont::Helvetica,
    });
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
        "es" => "Transcripcion",
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
        "es" => format!("Transcripcion de {title}"),
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
            "es" => "Transcripcion",
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
        "de" => "Haeufige Fragen",
        _ => "FAQs",
    }
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
            .enumerate()
            .flat_map(|(index, segment)| {
                let start_seconds = parse_timestamp_to_seconds(&segment.time);
                let end_seconds = start_seconds + 11;
                let mut cue_lines = vec![
                    ExportStyledLine {
                        text: (index + 1).to_string(),
                        speaker_color: None,
                    },
                    ExportStyledLine {
                        text: format!(
                            "{} --> {}",
                            format_srt_time(start_seconds),
                            format_srt_time(end_seconds)
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

fn build_export_document(
    language: &str,
    title: &str,
    transcription: &str,
    summary: &str,
    faqs: &str,
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

fn render_document_body_lines(document: &ExportDocument) -> Vec<String> {
    let mut lines = Vec::new();

    for (index, section) in document.sections.iter().enumerate() {
        if index == 0 {
            lines.push(String::new());
        } else {
            lines.push(String::new());
            lines.push(String::new());
        }
        lines.push(section.title.clone());
        if let Some(styled_lines) = &section.styled_lines {
            lines.extend(styled_lines.iter().map(|line| line.text.clone()));
        } else {
            lines.extend(section.body.lines().map(|line| line.to_string()));
        }
    }

    lines
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
                speaker_id: None,
                speaker_label: None,
            }
        })
        .collect()
}

fn format_mm_ss_u32(total_seconds: u32) -> String {
    let mm = total_seconds / 60;
    let ss = total_seconds % 60;
    format!("{:02}:{:02}", mm, ss)
}

fn parse_timestamp_to_seconds(value: &str) -> u32 {
    let mut parts = value.trim().split(':').collect::<Vec<_>>();
    if parts.len() < 2 || parts.len() > 3 {
        return 0;
    }

    if parts.len() == 2 {
        parts.insert(0, "0");
    }

    let hh = parts[0].parse::<u32>().unwrap_or(0);
    let mm = parts[1].parse::<u32>().unwrap_or(0);
    let ss = parts[2].parse::<u32>().unwrap_or(0);

    hh * 3600 + mm * 60 + ss
}

fn format_srt_time(seconds: u32) -> String {
    let hh = seconds / 3600;
    let mm = (seconds % 3600) / 60;
    let ss = seconds % 60;
    format!("{:02}:{:02}:{:02},000", hh, mm, ss)
}

fn format_vtt_time(seconds: u32) -> String {
    let hh = seconds / 3600;
    let mm = (seconds % 3600) / 60;
    let ss = seconds % 60;
    format!("{:02}:{:02}:{:02}.000", hh, mm, ss)
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
        .map(|segment| {
            let start_seconds = parse_timestamp_to_seconds(&segment.time);
            let end_seconds = start_seconds + 11;
            format!(
                "{} --> {}\n{}",
                format_vtt_time(start_seconds),
                format_vtt_time(end_seconds),
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
                .enumerate()
                .map(|(index, segment)| {
                    let start_seconds = parse_timestamp_to_seconds(&segment.time);
                    let end_seconds = start_seconds + 11;
                    format!(
                        "{}\n{} --> {}\n{}",
                        index + 1,
                        format_srt_time(start_seconds),
                        format_srt_time(end_seconds),
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

    use super::{
        build_artifact_context_transcript, build_chat_context_candidates, build_chunk_note_prompt,
        build_confidence_aware_optimize_prompt, build_direct_summary_prompt, build_export_content,
        build_export_document, build_export_segments, build_summary_instructions,
        build_summary_synthesis_prompt, chunk_text_by_words, extract_low_confidence_spans,
        is_context_window_error, optimize_with_rag, render_plain_text_document, summarize_with_rag,
        trimmed_audio_output_metadata, validate_trimmed_audio_output, ApplicationError,
        ArtifactAiContextOptions, ArtifactKind, ExportStyle, SummarizeArtifactPayload,
        TranscriptArtifact, TranscriptEnhancer, MIN_TRIMMED_AUDIO_DURATION_SECONDS,
    };

    struct TrackingEnhancer {
        optimize_calls: AtomicUsize,
        ask_calls: AtomicUsize,
        active_calls: AtomicUsize,
        max_active_calls: AtomicUsize,
        prompts: Mutex<Vec<String>>,
        prefer_single_pass: bool,
        chunk_concurrency_limit: usize,
        fail_direct_attempts: AtomicUsize,
        hallucinate_optimize: bool,
        hallucinate_merge: bool,
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
                prefer_single_pass,
                chunk_concurrency_limit,
                fail_direct_attempts: AtomicUsize::new(fail_direct_attempts),
                hallucinate_optimize: false,
                hallucinate_merge: false,
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
            _language_code: &str,
        ) -> Result<String, ApplicationError> {
            self.optimize_calls.fetch_add(1, Ordering::SeqCst);
            if self.hallucinate_optimize {
                Ok(format!("{text} added commentary"))
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
    }

    impl Default for TrackingEnhancer {
        fn default() -> Self {
            Self::new(false, 3, 0)
        }
    }

    fn sample_artifact(text: &str) -> TranscriptArtifact {
        TranscriptArtifact {
            id: "id-1".to_string(),
            job_id: "job-1".to_string(),
            title: "Sample".to_string(),
            kind: ArtifactKind::File,
            input_path: "/tmp/sample.wav".to_string(),
            raw_transcript: text.to_string(),
            optimized_transcript: String::new(),
            summary: String::new(),
            faqs: String::new(),
            metadata: BTreeMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
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
    fn export_document_localizes_title_and_includes_summary_and_faqs() {
        let mut artifact = sample_artifact("Linea uno");
        artifact.title = "Riunione team".to_string();
        artifact.summary = "Sintesi breve".to_string();
        artifact.faqs = "D: Chi segue?\nR: Marta.".to_string();

        let segments = vec![super::ExportSegment {
            time: "00:00".to_string(),
            line: "Linea uno".to_string(),
            speaker_id: None,
            speaker_label: None,
        }];

        let document = build_export_document(
            "it",
            &artifact.title,
            &artifact.raw_transcript,
            &artifact.summary,
            &artifact.faqs,
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
    fn summary_instructions_keep_required_controls_even_with_custom_prompt() {
        let instructions = build_summary_instructions(&SummarizeArtifactPayload {
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
        });

        assert!(instructions.contains("The entire output must be in Italian."));
        assert!(instructions.contains("Do not include timestamps in the final summary."));
        assert!(instructions.contains("Attribute statements to named speakers"));
        assert!(instructions.contains("Focus on hiring decisions."));
    }

    #[test]
    fn summary_instructions_default_to_detailed_prose() {
        let instructions = build_summary_instructions(&SummarizeArtifactPayload {
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
        });

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
    async fn optimize_with_rag_rejects_added_text_from_enhancer() {
        let enhancer = TrackingEnhancer::with_hallucinations(false, 1, 0, true, false);
        let transcript = "Alice reviews the roadmap and confirms the launch checklist is complete.";

        let optimized = optimize_with_rag(&enhancer, transcript, "en")
            .await
            .expect("optimization should succeed");

        assert_eq!(optimized, transcript);
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
}
