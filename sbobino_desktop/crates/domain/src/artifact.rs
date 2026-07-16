use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::DomainError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChatMessage {
    pub id: String,
    pub artifact_id: String,
    pub role: String,
    pub text: String,
    pub origin: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl ArtifactChatMessage {
    pub fn new(
        artifact_id: impl Into<String>,
        role: impl Into<String>,
        text: impl Into<String>,
        origin: impl Into<String>,
        status: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            artifact_id: artifact_id.into(),
            role: role.into(),
            text: text.into(),
            origin: origin.into(),
            status: status.into(),
            provider: None,
            model: None,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    File,
    Realtime,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Realtime => "realtime",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSourceOrigin {
    #[default]
    Imported,
    Trimmed,
    Realtime,
    LegacyExternal,
}

impl ArtifactSourceOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Trimmed => "trimmed",
            Self::Realtime => "realtime",
            Self::LegacyExternal => "legacy_external",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAudioBackfillStatus {
    #[default]
    Imported,
    PendingBackfill,
    Missing,
}

impl ArtifactAudioBackfillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::PendingBackfill => "pending_backfill",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TimedWord {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TimedSegment {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f32>,
    // Hook for future diarization support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    // Hook for future diarization support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<TimedWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SpeakerTurn {
    pub speaker_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_label: Option<String>,
    pub start_seconds: f32,
    pub end_seconds: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct TranscriptionOutput {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<TimedSegment>,
}

impl TranscriptionOutput {
    pub fn from_text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            segments: Vec::new(),
        }
    }

    pub fn timeline_v2_metadata_json(&self) -> String {
        timeline_v2_json_from_segments(&self.segments)
    }
}

fn timeline_v2_json_from_segments(segments: &[TimedSegment]) -> String {
    let mut output = String::from("{\"version\":2,\"segments\":[");

    for (segment_index, segment) in segments.iter().enumerate() {
        if segment_index > 0 {
            output.push(',');
        }

        output.push('{');
        output.push_str("\"text\":");
        push_json_string(&mut output, &segment.text);

        if let Some(start) = segment.start_seconds.filter(|value| value.is_finite()) {
            output.push_str(",\"start_seconds\":");
            output.push_str(&format_json_number(start));
        }
        if let Some(end) = segment.end_seconds.filter(|value| value.is_finite()) {
            output.push_str(",\"end_seconds\":");
            output.push_str(&format_json_number(end));
        }
        if let Some(speaker_id) = segment.speaker_id.as_deref() {
            output.push_str(",\"speaker_id\":");
            push_json_string(&mut output, speaker_id);
        }
        if let Some(speaker_label) = segment.speaker_label.as_deref() {
            output.push_str(",\"speaker_label\":");
            push_json_string(&mut output, speaker_label);
        }

        output.push_str(",\"words\":[");
        for (word_index, word) in segment.words.iter().enumerate() {
            if word_index > 0 {
                output.push(',');
            }

            output.push('{');
            output.push_str("\"text\":");
            push_json_string(&mut output, &word.text);

            if let Some(start) = word.start_seconds.filter(|value| value.is_finite()) {
                output.push_str(",\"start_seconds\":");
                output.push_str(&format_json_number(start));
            }
            if let Some(end) = word.end_seconds.filter(|value| value.is_finite()) {
                output.push_str(",\"end_seconds\":");
                output.push_str(&format_json_number(end));
            }
            if let Some(confidence) = word.confidence.filter(|value| value.is_finite()) {
                output.push_str(",\"confidence\":");
                output.push_str(&format_json_number(confidence));
            }

            output.push('}');
        }
        output.push_str("]}");
    }

    output.push_str("]}");
    output
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000C}' => output.push_str("\\f"),
            ch if ch <= '\u{001F}' => {
                let escaped = format!("\\u{:04X}", ch as u32);
                output.push_str(&escaped);
            }
            _ => output.push(ch),
        }
    }
    output.push('"');
}

fn format_json_number(value: f32) -> String {
    let mut rendered = format!("{value:.6}");
    while rendered.contains('.') && rendered.ends_with('0') {
        rendered.pop();
    }
    if rendered.ends_with('.') {
        rendered.push('0');
    }
    rendered
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptArtifact {
    pub id: String,
    pub job_id: String,
    pub title: String,
    pub kind: ArtifactKind,
    pub source_label: String,
    pub source_origin: ArtifactSourceOrigin,
    pub audio_available: bool,
    pub audio_backfill_status: ArtifactAudioBackfillStatus,
    pub revision: i64,
    pub raw_transcript: String,
    pub optimized_transcript: String,
    pub summary: String,
    pub faqs: String,
    pub metadata: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_duration_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_byte_size: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip)]
    pub source_external_path: Option<String>,
    #[serde(skip)]
    pub whisper_options_json: Option<String>,
    #[serde(skip)]
    pub diarization_settings_json: Option<String>,
    #[serde(skip)]
    pub ai_provider_snapshot_json: Option<String>,
    #[serde(skip)]
    pub source_fingerprint_json: Option<String>,
}

impl TranscriptArtifact {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        job_id: impl Into<String>,
        title: impl Into<String>,
        kind: ArtifactKind,
        source_label: impl Into<String>,
        source_origin: ArtifactSourceOrigin,
        raw_transcript: impl Into<String>,
        optimized_transcript: impl Into<String>,
        summary: impl Into<String>,
        faqs: impl Into<String>,
        metadata: BTreeMap<String, String>,
    ) -> Result<Self, DomainError> {
        let raw_transcript = raw_transcript.into();
        if raw_transcript.trim().is_empty() {
            return Err(DomainError::EmptyTranscript);
        }

        let optimized_transcript = optimized_transcript.into();
        let now = Utc::now();
        let source_label = source_label.into();
        let title = title.into();
        let title = if title.trim().is_empty() {
            source_label.clone()
        } else {
            title
        };

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            job_id: job_id.into(),
            title,
            kind,
            source_label,
            source_origin,
            audio_available: false,
            audio_backfill_status: ArtifactAudioBackfillStatus::default(),
            revision: 0,
            raw_transcript,
            optimized_transcript: if optimized_transcript.trim().is_empty() {
                String::new()
            } else {
                optimized_transcript
            },
            summary: summary.into(),
            faqs: faqs.into(),
            metadata,
            parent_artifact_id: None,
            processing_engine: None,
            processing_model: None,
            processing_language: None,
            audio_duration_seconds: None,
            audio_byte_size: None,
            created_at: now,
            updated_at: now,
            source_external_path: None,
            whisper_options_json: None,
            diarization_settings_json: None,
            ai_provider_snapshot_json: None,
            source_fingerprint_json: None,
        })
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    pub fn set_source_external_path(&mut self, path: impl Into<String>) {
        self.source_external_path = Some(path.into());
    }
}
