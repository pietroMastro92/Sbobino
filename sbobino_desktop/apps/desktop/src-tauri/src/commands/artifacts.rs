use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use docx_rs::{Docx, Paragraph, Run};
use printpdf::{ops::PdfPage, text::TextItem, units::Pt, BuiltinFont, Mm, Op, PdfDocument};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use sbobino_application::{ApplicationError, ArtifactQuery, TranscriptEnhancer};
use sbobino_domain::{ArtifactKind, TranscriptArtifact};

use crate::{error::CommandError, state::AppState};

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
}

#[derive(Debug, Deserialize)]
pub struct SummarizeArtifactPayload {
    pub id: String,
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct OptimizeArtifactPayload {
    pub id: String,
    pub text: String,
}

const CHAT_CONTEXT_BUDGETS: &[(usize, usize)] = &[(8, 7600), (6, 5200), (4, 3400), (2, 2000)];
const CHAT_CHUNK_TARGET_CHARS: usize = 900;
const CHAT_CHUNK_OVERLAP_WORDS: usize = 24;
const SUMMARY_CHUNK_TARGET_CHARS: usize = 4000;
const SUMMARY_CHUNK_OVERLAP_WORDS: usize = 30;
const SUMMARY_SYNTHESIS_BUDGETS: &[usize] = &[12_000, 8_000, 5_000, 3_000];

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
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
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
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_timestamps: false,
            grouping: Some(ExportGrouping::None),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ExportArtifactPayload {
    pub id: String,
    pub format: ExportFormat,
    pub destination_path: String,
    pub style: Option<ExportStyle>,
    pub options: Option<ExportOptions>,
    pub segments: Option<Vec<ExportSegment>>,
    pub content_override: Option<String>,
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
    let segments = payload
        .segments
        .filter(|entries| !entries.is_empty())
        .unwrap_or_else(|| build_segments_from_text(&base_transcription));
    let export_content = build_export_content(
        &base_transcription,
        &segments,
        style,
        options.include_timestamps,
    );

    match payload.format {
        ExportFormat::Txt => export_txt(destination_path, &export_content)?,
        ExportFormat::Docx => export_docx(destination_path, &export_content)?,
        ExportFormat::Html => export_html(destination_path, &artifact.title, &export_content)?,
        ExportFormat::Pdf => export_pdf(destination_path, &export_content)?,
        ExportFormat::Json => export_json(
            destination_path,
            &artifact,
            style,
            grouping,
            options.include_timestamps,
            &segments,
            &export_content,
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
        .build_active_enhancer()
        .map_err(|e| CommandError::new("runtime_factory", e))?
        .ok_or_else(|| {
            CommandError::new(
                "missing_ai_provider",
                "No AI provider is configured in Settings > AI Services.",
            )
        })?;

    let prompt = payload.prompt.trim();
    if prompt.is_empty() {
        return Err(CommandError::new(
            "validation",
            "chat prompt cannot be empty",
        ));
    }

    let candidates = build_chat_context_candidates(&artifact, prompt);
    ask_with_overflow_fallback(enhancer.as_ref(), candidates)
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

    let enhancer = state
        .runtime_factory
        .build_active_enhancer()
        .map_err(|e| CommandError::new("runtime_factory", e))?
        .ok_or_else(|| {
            CommandError::new(
                "missing_ai_provider",
                "No AI provider is configured in Settings > AI Services.",
            )
        })?;

    let text = payload.text.trim();
    if text.is_empty() {
        return Err(CommandError::new(
            "validation",
            "cannot optimize empty text",
        ));
    }

    // Default to the artifact's original language, or "en" if not directly available (though the enhancer usually handles empty language code gracefully or ignores it)
    enhancer
        .optimize(text, "")
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

    let enhancer = state
        .runtime_factory
        .build_active_enhancer()
        .map_err(|e| CommandError::new("runtime_factory", e))?
        .ok_or_else(|| {
            CommandError::new(
                "missing_ai_provider",
                "No AI provider is configured in Settings > AI Services.",
            )
        })?;

    let transcript = effective_transcript(&artifact);
    if transcript.trim().is_empty() {
        return Err(CommandError::new(
            "empty_content",
            "no transcription available to summarize",
        ));
    }

    let user_prompt = payload.prompt.trim();
    let instructions = if user_prompt.is_empty() {
        "Summarize this transcript clearly and concisely."
    } else {
        user_prompt
    };

    summarize_with_rag(enhancer.as_ref(), &transcript, instructions)
        .await
        .map_err(CommandError::from)
}

fn effective_transcript(artifact: &TranscriptArtifact) -> String {
    let optimized = artifact.optimized_transcript.trim();
    if !optimized.is_empty() {
        return optimized.to_string();
    }
    artifact.raw_transcript.trim().to_string()
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

fn build_chat_context_candidates(artifact: &TranscriptArtifact, prompt: &str) -> Vec<String> {
    let transcript = effective_transcript(artifact);
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

            format!(
                "You are an assistant for transcript analysis.\n\
                 Answer using the provided transcript snippets. If you cannot infer the answer, state what is missing.\n\n\
                 Artifact title: {title}\n\n\
                 Existing summary:\n{summary}\n\n\
                 Existing FAQs:\n{faqs}\n\n\
                 Transcript snippets:\n{packed}\n\
                 User question:\n{normalized_prompt}"
            )
        })
        .collect()
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

    let mut chunk_notes = Vec::with_capacity(chunks.len());
    let total = chunks.len();

    for (index, chunk) in chunks.iter().enumerate() {
        let chunk_prompt = format!(
            "You are preparing intermediate notes for a transcript summary.\n\
             Follow the user instructions exactly.\n\n\
             User instructions:\n{user_instructions}\n\n\
             Analyze chunk {}/{} and return concise notes only.\n\
             Include key facts, decisions, action items, names, and numeric details when present.\n\
             Keep output compact and avoid filler.\n\n\
             Transcript chunk:\n{}",
            index + 1,
            total,
            chunk
        );

        let note = ask_with_overflow_fallback(
            enhancer,
            vec![
                chunk_prompt.clone(),
                truncate_chars(&chunk_prompt, 2600),
                truncate_chars(&chunk_prompt, 1900),
            ],
        )
        .await?;

        chunk_notes.push(format!("Chunk {} notes:\n{}", index + 1, note.trim()));
    }

    let merged_notes = chunk_notes.join("\n\n");
    let candidates = SUMMARY_SYNTHESIS_BUDGETS
        .iter()
        .map(|budget| {
            let clipped_notes = truncate_chars(&merged_notes, *budget);
            format!(
                "You are creating the final transcript summary from chunk notes.\n\
                 Follow user instructions exactly.\n\n\
                 User instructions:\n{user_instructions}\n\n\
                 Produce the final summary now.\n\
                 Return only the final text.\n\n\
                 Chunk notes:\n{clipped_notes}"
            )
        })
        .collect::<Vec<_>>();

    ask_with_overflow_fallback(enhancer, candidates).await
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
            "Exceeded model context window size. The app now uses chunked retrieval, but this request is still too large. Try a shorter custom prompt or fewer summary constraints."
                .to_string(),
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

fn export_txt(path: &Path, transcription: &str) -> Result<(), CommandError> {
    std::fs::write(path, transcription)
        .map_err(|e| CommandError::new("export", format!("failed to export txt: {e}")))
}

fn export_docx(path: &Path, transcription: &str) -> Result<(), CommandError> {
    let doc =
        Docx::new().add_paragraph(Paragraph::new().add_run(Run::new().add_text(transcription)));

    let file = File::create(path)
        .map_err(|e| CommandError::new("export", format!("failed to create docx file: {e}")))?;

    doc.build()
        .pack(file)
        .map_err(|e| CommandError::new("export", format!("failed to write docx: {e}")))
}

fn export_html(path: &Path, title: &str, transcription: &str) -> Result<(), CommandError> {
    let escaped_title = escape_html(title);
    let escaped_transcription = escape_html(transcription).replace('\n', "<br/>\n");
    let html = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\" />\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n<title>{}</title>\n<style>\nbody{{font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;margin:2rem;color:#1f2430;background:#f8fafc;}}\nmain{{max-width:880px;margin:0 auto;padding:1.5rem 1.75rem;background:#fff;border:1px solid #dbe2ee;border-radius:14px;}}\nh1{{font-size:1.35rem;margin:0 0 1rem;}}\n.content{{line-height:1.6;font-size:1rem;word-break:break-word;}}\n</style>\n</head>\n<body>\n<main>\n<h1>{}</h1>\n<div class=\"content\">{}</div>\n</main>\n</body>\n</html>\n",
        escaped_title, escaped_title, escaped_transcription
    );

    std::fs::write(path, html)
        .map_err(|e| CommandError::new("export", format!("failed to export html: {e}")))
}

fn export_json(
    path: &Path,
    artifact: &TranscriptArtifact,
    style: ExportStyle,
    grouping: ExportGrouping,
    include_timestamps: bool,
    segments: &[ExportSegment],
    content: &str,
) -> Result<(), CommandError> {
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
            "grouping": grouping
        },
        "content": content,
        "summary": artifact.summary,
        "faqs": artifact.faqs,
        "segments": segments,
        "metadata": artifact.metadata,
    });

    let serialized = serde_json::to_string_pretty(&payload)
        .map_err(|e| CommandError::new("export", format!("failed to encode json export: {e}")))?;

    std::fs::write(path, serialized)
        .map_err(|e| CommandError::new("export", format!("failed to export json: {e}")))
}

fn export_pdf(path: &Path, transcription: &str) -> Result<(), CommandError> {
    let mut doc = PdfDocument::new("Transcription");
    let mut pages = Vec::new();
    let mut ops = start_pdf_page_ops(true);
    let mut y = 780.0_f32;

    if transcription.trim().is_empty() {
        write_pdf_line(&mut ops, "No content available for export.", y);
    } else {
        for line in transcription.lines() {
            if y < 42.0 {
                ops.push(Op::EndTextSection);
                pages.push(PdfPage::new(Mm(210.0), Mm(297.0), ops));
                ops = start_pdf_page_ops(false);
                y = 810.0;
            }

            write_pdf_line(&mut ops, line, y);
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

fn start_pdf_page_ops(with_title: bool) -> Vec<Op> {
    let mut ops = vec![Op::StartTextSection];

    if with_title {
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
            items: vec![TextItem::Text("Transcription".to_string())],
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

fn write_pdf_line(ops: &mut Vec<Op>, line: &str, y: f32) {
    ops.push(Op::SetTextCursor {
        pos: printpdf::graphics::Point {
            x: Pt(28.0),
            y: Pt(y),
        },
    });
    ops.push(Op::WriteTextBuiltinFont {
        items: vec![TextItem::Text(line.to_string())],
        font: BuiltinFont::Helvetica,
    });
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
            }
        })
        .collect()
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

fn build_export_content(
    transcription: &str,
    segments: &[ExportSegment],
    style: ExportStyle,
    include_timestamps: bool,
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
                    let end_seconds = start_seconds + 4;
                    format!(
                        "{}\n{} --> {}\n{}",
                        index + 1,
                        format_srt_time(start_seconds),
                        format_srt_time(end_seconds),
                        segment.line.trim()
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
                        format!("[{}] {}", segment.time, segment.line.trim())
                    } else {
                        segment.line.trim().to_string()
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
                    .map(|segment| format!("[{}] {}", segment.time, segment.line.trim()))
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
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::{
        build_chat_context_candidates, chunk_text_by_words, is_context_window_error,
        ApplicationError, ArtifactKind, TranscriptArtifact,
    };

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
        let candidates = build_chat_context_candidates(&artifact, "what about gamma and sigma?");
        assert!(!candidates.is_empty());
        assert!(candidates
            .iter()
            .all(|value| value.contains("User question:")));
    }

    #[test]
    fn detects_context_window_errors() {
        let error = ApplicationError::PostProcessing(
            "Foundation bridge error: Exceeded model context window size".to_string(),
        );
        assert!(is_context_window_error(&error));
    }
}
