use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use serde::{Deserialize, Serialize};

use sbobino_domain::{minimize_transcript_repetitions, TranscriptArtifact};

fn default_true() -> bool {
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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TimelineV2Document {
    #[serde(default)]
    pub segments: Vec<TimelineV2Segment>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TimelineV2Segment {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub start_seconds: Option<f32>,
    #[serde(default)]
    pub end_seconds: Option<f32>,
    #[serde(default)]
    pub speaker_id: Option<String>,
    #[serde(default)]
    pub speaker_label: Option<String>,
    #[serde(default)]
    pub language_code: Option<String>,
    #[serde(default)]
    pub language_confidence: Option<f32>,
    #[serde(default)]
    pub words: Vec<TimelineV2Word>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TimelineV2Word {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub start_seconds: Option<f32>,
    #[serde(default)]
    pub end_seconds: Option<f32>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct PreparedTimelineSegment {
    pub source_index: usize,
    pub text: String,
    pub time_label: Option<String>,
    pub start_seconds: Option<f32>,
    pub end_seconds: Option<f32>,
    pub speaker_id: Option<String>,
    pub speaker_label: Option<String>,
    pub language_code: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PreparedTranscriptContext {
    pub transcript: String,
    pub ai_transcript: String,
    pub transcript_hash: String,
    pub char_count: usize,
    pub word_count: usize,
    pub timeline_segments: Vec<PreparedTimelineSegment>,
}

impl PreparedTranscriptContext {
    pub fn from_transcript(transcript: &str) -> Self {
        let normalized = minimize_transcript_repetitions(transcript);
        let mut hasher = DefaultHasher::new();
        normalized.hash(&mut hasher);

        Self {
            transcript: transcript.to_string(),
            ai_transcript: normalized.clone(),
            transcript_hash: format!("{:016x}", hasher.finish()),
            char_count: normalized.chars().count(),
            word_count: normalized.split_whitespace().count(),
            timeline_segments: Vec::new(),
        }
    }

    pub fn from_artifact(artifact: &TranscriptArtifact, context: ArtifactAiContextOptions) -> Self {
        let transcript = effective_transcript(artifact);
        let timeline_segments = parse_timeline_context_segments(artifact);
        let ai_transcript = if timeline_segments.is_empty() {
            transcript.clone()
        } else {
            timeline_segments
                .iter()
                .map(|segment| render_timeline_context_segment(segment, context))
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let normalized = minimize_transcript_repetitions(&ai_transcript);
        let mut hasher = DefaultHasher::new();
        normalized.hash(&mut hasher);
        let transcript_hash = format!("{:016x}", hasher.finish());

        Self {
            transcript,
            ai_transcript: normalized.clone(),
            transcript_hash,
            char_count: normalized.chars().count(),
            word_count: normalized.split_whitespace().count(),
            timeline_segments,
        }
    }

    pub fn top_salient_excerpts(&self, target_chars: usize, include_speakers: bool) -> String {
        if self.timeline_segments.is_empty() || self.ai_transcript.chars().count() <= target_chars {
            return truncate_chars(&self.ai_transcript, target_chars);
        }

        let mut candidates = self
            .timeline_segments
            .iter()
            .filter_map(|segment| {
                let line = if include_speakers {
                    match segment.speaker_label.as_deref() {
                        Some(label) => format!("{label}: {}", segment.text.trim()),
                        None => segment.text.trim().to_string(),
                    }
                } else {
                    segment.text.trim().to_string()
                };
                if line.is_empty() {
                    None
                } else {
                    Some((segment.text.chars().count(), line))
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

        let mut excerpts = Vec::new();
        let mut used = 0usize;
        for (_, excerpt) in candidates.into_iter().take(8) {
            let addition = excerpt.chars().count() + if excerpts.is_empty() { 0 } else { 2 };
            if !excerpts.is_empty() && used + addition > target_chars {
                break;
            }
            used += addition;
            excerpts.push(excerpt);
        }

        if excerpts.is_empty() {
            truncate_chars(&self.ai_transcript, target_chars)
        } else {
            excerpts.join("\n\n")
        }
    }
}

pub fn effective_transcript(artifact: &TranscriptArtifact) -> String {
    let optimized = artifact.optimized_transcript.trim();
    if optimized.is_empty() {
        artifact.raw_transcript.trim().to_string()
    } else {
        optimized.to_string()
    }
}

pub fn parse_timeline_document(artifact: &TranscriptArtifact) -> Option<TimelineV2Document> {
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

pub fn parse_timeline_context_segments(
    artifact: &TranscriptArtifact,
) -> Vec<PreparedTimelineSegment> {
    let Some(parsed) = parse_timeline_document(artifact) else {
        return Vec::new();
    };

    parsed
        .segments
        .into_iter()
        .enumerate()
        .filter_map(|(source_index, segment)| {
            let text = normalize_optional_text(Some(segment.text.clone())).unwrap_or_else(|| {
                segment
                    .words
                    .iter()
                    .map(|word| word.text.trim())
                    .filter(|word| !word.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
            });
            let text = text.trim();
            if text.is_empty() {
                return None;
            }

            let start_seconds = segment.start_seconds.filter(|value| value.is_finite());
            let end_seconds = segment
                .end_seconds
                .filter(|value| value.is_finite())
                .or_else(|| {
                    segment
                        .words
                        .iter()
                        .find_map(|word| word.end_seconds.filter(|value| value.is_finite()))
                });
            let time_label = start_seconds
                .or_else(|| {
                    segment
                        .words
                        .iter()
                        .find_map(|word| word.start_seconds.filter(|value| value.is_finite()))
                })
                .map(format_mm_ss);
            let speaker_id = normalize_optional_text(segment.speaker_id);
            let speaker_label =
                normalize_optional_text(segment.speaker_label).or_else(|| speaker_id.clone());
            let language_code = normalize_optional_text(segment.language_code);

            Some(PreparedTimelineSegment {
                source_index,
                text: text.to_string(),
                time_label,
                start_seconds,
                end_seconds,
                speaker_id,
                speaker_label,
                language_code,
            })
        })
        .collect()
}

pub fn format_mm_ss(seconds: f32) -> String {
    let total_seconds = seconds.floor().max(0.0) as u32;
    let mm = total_seconds / 60;
    let ss = total_seconds % 60;
    format!("{mm:02}:{ss:02}")
}

pub fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn render_timeline_context_segment(
    segment: &PreparedTimelineSegment,
    context: ArtifactAiContextOptions,
) -> String {
    let mut prefix = String::new();
    if context.include_timestamps {
        if let Some(time_label) = segment.time_label.as_deref() {
            prefix.push('[');
            prefix.push_str(time_label);
            prefix.push_str("] ");
        }
    }
    if context.include_speakers {
        if let Some(speaker_label) = segment.speaker_label.as_deref() {
            prefix.push_str(speaker_label);
            prefix.push_str(": ");
        }
    }
    if let Some(language_code) = segment.language_code.as_deref() {
        prefix.push_str("[source_language=");
        prefix.push_str(language_code);
        prefix.push_str("] ");
    }

    format!("{prefix}{}", segment.text.trim())
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect::<String>()
}
