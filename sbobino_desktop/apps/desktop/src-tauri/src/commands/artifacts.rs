use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use docx_rs::{Docx, Paragraph, Run};
use printpdf::{ops::PdfPage, text::TextItem, units::Pt, BuiltinFont, Mm, Op, PdfDocument};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::State;

use sbobino_application::ArtifactQuery;
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
pub struct ChatArtifactPayload {
    pub id: String,
    pub prompt: String,
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
        .build_gemini_enhancer()
        .map_err(|e| CommandError::new("runtime_factory", e))?
        .ok_or_else(|| {
            CommandError::new(
                "missing_api_key",
                "Gemini API key is not configured in settings.",
            )
        })?;

    let context = format!(
        "You are an assistant for transcript analysis.\n\nTranscript:\n{}\n\nSummary:\n{}\n\nFAQs:\n{}\n\nUser question:\n{}",
        artifact.optimized_transcript,
        artifact.summary,
        artifact.faqs,
        payload.prompt
    );

    enhancer.ask(&context).await.map_err(CommandError::from)
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
