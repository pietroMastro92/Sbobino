use serde::{Deserialize, Serialize};

use crate::{TimedSegment, TimedWord};

/// Metadata keys are versioned deliberately so that older artifacts can be
/// opened without requiring a migration and newer clients can reject only a
/// report format they do not understand.
pub const SEGMENT_REPAIR_METADATA_KEY: &str = "segment_repair_v1";
pub const SPEAKER_QUALITY_METADATA_KEY: &str = "speaker_quality_v1";
pub const TIMELINE_MANUAL_EDITS_METADATA_KEY: &str = "timeline_manual_edits_v1";

const MAX_SUSPICIOUS_SHORT_TURN_SECONDS: f32 = 1.5;
const MIN_DUPLICATE_WORDS: usize = 4;
const MIN_DUPLICATE_CHARS: usize = 12;
const MAX_DUPLICATE_GAP_SECONDS: f32 = 1.5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualityReportStatus {
    Completed,
    Unchanged,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentRepairReport {
    pub version: String,
    pub status: QualityReportStatus,
    pub input_segment_count: usize,
    pub output_segment_count: usize,
    pub collapsed_repeated_segment_count: usize,
    pub timestamp_repair_count: usize,
    pub changed: bool,
}

impl SegmentRepairReport {
    pub fn unavailable() -> Self {
        Self {
            version: SEGMENT_REPAIR_METADATA_KEY.to_string(),
            status: QualityReportStatus::Unavailable,
            input_segment_count: 0,
            output_segment_count: 0,
            collapsed_repeated_segment_count: 0,
            timestamp_repair_count: 0,
            changed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerQualityWarning {
    pub kind: SpeakerQualityWarningKind,
    /// Zero-based indexes into the final persisted `timeline_v2` segments.
    pub segment_indexes: Vec<usize>,
    pub speaker_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeakerQualityWarningKind {
    ShortFlip,
    RapidTurn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeakerQualityReport {
    pub version: String,
    pub status: QualityReportStatus,
    pub warning_count: usize,
    pub warnings: Vec<SpeakerQualityWarning>,
}

impl SpeakerQualityReport {
    pub fn unavailable() -> Self {
        Self {
            version: SPEAKER_QUALITY_METADATA_KEY.to_string(),
            status: QualityReportStatus::Unavailable,
            warning_count: 0,
            warnings: Vec::new(),
        }
    }
}

/// Apply only structural cleanup that is directly evidenced by the supplied
/// timeline. Repeated adjacent segments are collapsed by the existing domain
/// helper, while speaker fields are never inferred or rewritten.
pub fn repair_segments(segments: &[TimedSegment]) -> (Vec<TimedSegment>, SegmentRepairReport) {
    if segments.is_empty() {
        return (Vec::new(), SegmentRepairReport::unavailable());
    }

    let mut timestamp_repair_count = 0;
    let normalized = segments
        .iter()
        .map(|segment| {
            let (next, repaired) = repair_segment_timestamps(segment);
            timestamp_repair_count += repaired as usize;
            next
        })
        .collect::<Vec<_>>();
    let repaired = collapse_repeated_segments_preserving_speakers(&normalized);
    let collapsed_repeated_segment_count = normalized.len().saturating_sub(repaired.len());
    let changed = timestamp_repair_count > 0 || collapsed_repeated_segment_count > 0;

    (
        repaired.clone(),
        SegmentRepairReport {
            version: SEGMENT_REPAIR_METADATA_KEY.to_string(),
            status: if changed {
                QualityReportStatus::Completed
            } else {
                QualityReportStatus::Unchanged
            },
            input_segment_count: segments.len(),
            output_segment_count: repaired.len(),
            collapsed_repeated_segment_count,
            timestamp_repair_count,
            changed,
        },
    )
}

/// Produce warning-only speaker quality evidence. This function intentionally
/// returns no modified segments: a suspicious attribution must remain visible
/// for review instead of being silently replaced.
pub fn speaker_quality_report(segments: &[TimedSegment]) -> SpeakerQualityReport {
    if segments.is_empty() {
        return SpeakerQualityReport::unavailable();
    }

    let mut warnings = Vec::new();
    for index in 1..segments.len().saturating_sub(1) {
        let Some(previous_speaker) = speaker_key(&segments[index - 1]) else {
            continue;
        };
        let Some(current_speaker) = speaker_key(&segments[index]) else {
            continue;
        };
        let Some(next_speaker) = speaker_key(&segments[index + 1]) else {
            continue;
        };
        let Some(current_duration) = segment_duration(&segments[index]) else {
            continue;
        };

        if previous_speaker == next_speaker
            && previous_speaker != current_speaker
            && current_duration <= MAX_SUSPICIOUS_SHORT_TURN_SECONDS
        {
            warnings.push(SpeakerQualityWarning {
                kind: SpeakerQualityWarningKind::ShortFlip,
                segment_indexes: vec![index - 1, index, index + 1],
                speaker_ids: vec![previous_speaker, current_speaker, next_speaker],
                duration_seconds: Some(current_duration),
            });
        }
    }

    for index in 0..segments.len().saturating_sub(1) {
        let Some(left_speaker) = speaker_key(&segments[index]) else {
            continue;
        };
        let Some(right_speaker) = speaker_key(&segments[index + 1]) else {
            continue;
        };
        if left_speaker == right_speaker {
            continue;
        }
        let Some(left_duration) = segment_duration(&segments[index]) else {
            continue;
        };
        let Some(right_duration) = segment_duration(&segments[index + 1]) else {
            continue;
        };
        if left_duration <= MAX_SUSPICIOUS_SHORT_TURN_SECONDS
            && right_duration <= MAX_SUSPICIOUS_SHORT_TURN_SECONDS
        {
            let already_reported = warnings.iter().any(|warning: &SpeakerQualityWarning| {
                warning.kind == SpeakerQualityWarningKind::ShortFlip
                    && warning
                        .segment_indexes
                        .windows(2)
                        .any(|pair| pair == [index, index + 1])
            });
            if !already_reported {
                warnings.push(SpeakerQualityWarning {
                    kind: SpeakerQualityWarningKind::RapidTurn,
                    segment_indexes: vec![index, index + 1],
                    speaker_ids: vec![left_speaker, right_speaker],
                    duration_seconds: Some(left_duration + right_duration),
                });
            }
        }
    }

    SpeakerQualityReport {
        version: SPEAKER_QUALITY_METADATA_KEY.to_string(),
        status: QualityReportStatus::Completed,
        warning_count: warnings.len(),
        warnings,
    }
}

fn repair_segment_timestamps(segment: &TimedSegment) -> (TimedSegment, bool) {
    let word_bounds = word_bounds(&segment.words);
    let mut repaired = segment.clone();
    let mut changed = false;

    let start = finite_non_negative(repaired.start_seconds);
    let end = finite_non_negative(repaired.end_seconds);
    if start != repaired.start_seconds {
        repaired.start_seconds = start;
        changed = true;
    }
    if end != repaired.end_seconds {
        repaired.end_seconds = end;
        changed = true;
    }

    if let Some((word_start, word_end)) = word_bounds {
        let has_valid_segment_bounds = matches!((repaired.start_seconds, repaired.end_seconds),
            (Some(start), Some(end)) if end > start);
        if !has_valid_segment_bounds {
            if repaired.start_seconds != Some(word_start) {
                repaired.start_seconds = Some(word_start);
                changed = true;
            }
            if repaired.end_seconds != Some(word_end) {
                repaired.end_seconds = Some(word_end);
                changed = true;
            }
        }
    }

    (repaired, changed)
}

fn collapse_repeated_segments_preserving_speakers(segments: &[TimedSegment]) -> Vec<TimedSegment> {
    let mut collapsed = Vec::new();
    for segment in segments {
        let text = collapse_whitespace(&segment.text);
        if text.is_empty() {
            continue;
        }

        let mut next = segment.clone();
        next.text = text;
        if let Some(previous) = collapsed.last_mut() {
            if should_collapse_segment_pair(previous, &next) {
                previous.end_seconds =
                    merge_optional_seconds(previous.end_seconds, next.end_seconds);
                if previous.start_seconds.is_none() {
                    previous.start_seconds = next.start_seconds;
                }
                if previous.language_code.is_none() {
                    previous.language_code = next.language_code.clone();
                    previous.language_confidence = next.language_confidence;
                }
                // Deliberately do not copy speaker_id or speaker_label from
                // `next`: a structural repair must never invent attribution.
                continue;
            }
        }
        collapsed.push(next);
    }
    collapsed
}

fn should_collapse_segment_pair(left: &TimedSegment, right: &TimedSegment) -> bool {
    if !is_substantive_duplicate_candidate(&left.text)
        || !is_substantive_duplicate_candidate(&right.text)
    {
        return false;
    }
    if normalized_optional(left.language_code.as_deref())
        != normalized_optional(right.language_code.as_deref())
    {
        return false;
    }
    if duplicate_key(&left.text) != duplicate_key(&right.text) {
        return false;
    }
    if normalized_optional(left.speaker_id.as_deref())
        != normalized_optional(right.speaker_id.as_deref())
        || normalized_optional(left.speaker_label.as_deref())
            != normalized_optional(right.speaker_label.as_deref())
    {
        return false;
    }

    match (left.end_seconds, right.start_seconds) {
        (Some(left_end), Some(right_start)) if left_end.is_finite() && right_start.is_finite() => {
            right_start <= left_end + MAX_DUPLICATE_GAP_SECONDS
        }
        _ => true,
    }
}

fn merge_optional_seconds(left: Option<f32>, right: Option<f32>) -> Option<f32> {
    match (left, right) {
        (Some(left), Some(right)) if left.is_finite() && right.is_finite() => Some(left.max(right)),
        (Some(left), _) if left.is_finite() => Some(left),
        (_, Some(right)) if right.is_finite() => Some(right),
        _ => None,
    }
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn duplicate_key(value: &str) -> String {
    collapse_whitespace(value)
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|character: char| {
                    character.is_whitespace()
                        || matches!(
                            character,
                            '.' | ','
                                | ';'
                                | ':'
                                | '!'
                                | '?'
                                | '"'
                                | '\''
                                | '`'
                                | '('
                                | ')'
                                | '['
                                | ']'
                                | '{'
                                | '}'
                                | '“'
                                | '”'
                                | '‘'
                                | '’'
                        )
                })
                .to_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_substantive_duplicate_candidate(value: &str) -> bool {
    value.split_whitespace().count() >= MIN_DUPLICATE_WORDS
        || value.chars().count() >= MIN_DUPLICATE_CHARS
}

fn normalized_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(str::to_lowercase)
}

fn word_bounds(words: &[TimedWord]) -> Option<(f32, f32)> {
    let start = words
        .iter()
        .filter_map(|word| finite_non_negative(word.start_seconds))
        .min_by(|left, right| left.total_cmp(right))?;
    let end = words
        .iter()
        .filter_map(|word| finite_non_negative(word.end_seconds))
        .max_by(|left, right| left.total_cmp(right))?;
    (end > start).then_some((start, end))
}

fn finite_non_negative(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

fn speaker_key(segment: &TimedSegment) -> Option<String> {
    segment
        .speaker_id
        .as_deref()
        .or(segment.speaker_label.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn segment_duration(segment: &TimedSegment) -> Option<f32> {
    let start = finite_non_negative(segment.start_seconds)?;
    let end = finite_non_negative(segment.end_seconds)?;
    (end > start).then_some(end - start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(text: &str, start: f32, end: f32, speaker: &str) -> TimedSegment {
        TimedSegment {
            text: text.to_string(),
            start_seconds: Some(start),
            end_seconds: Some(end),
            speaker_id: Some(speaker.to_string()),
            ..TimedSegment::default()
        }
    }

    #[test]
    fn repair_report_collapses_repeated_segments_and_preserves_speaker() {
        let input = vec![
            segment("hello hello hello hello", 0.0, 1.0, "A"),
            segment("hello hello hello hello", 1.1, 2.0, "A"),
        ];

        let (repaired, report) = repair_segments(&input);

        assert_eq!(repaired.len(), 1);
        assert_eq!(repaired[0].speaker_id.as_deref(), Some("A"));
        assert_eq!(report.version, SEGMENT_REPAIR_METADATA_KEY);
        assert_eq!(report.collapsed_repeated_segment_count, 1);
        assert_eq!(report.input_segment_count, 2);
        assert_eq!(report.output_segment_count, 1);
        assert!(report.changed);
    }

    #[test]
    fn repair_report_uses_word_bounds_for_invalid_segment_timestamps() {
        let input = vec![TimedSegment {
            text: "word".to_string(),
            start_seconds: Some(4.0),
            end_seconds: Some(2.0),
            words: vec![TimedWord {
                text: "word".to_string(),
                start_seconds: Some(0.5),
                end_seconds: Some(1.25),
                ..TimedWord::default()
            }],
            ..TimedSegment::default()
        }];

        let (repaired, report) = repair_segments(&input);

        assert_eq!(repaired[0].start_seconds, Some(0.5));
        assert_eq!(repaired[0].end_seconds, Some(1.25));
        assert_eq!(report.timestamp_repair_count, 1);
    }

    #[test]
    fn speaker_quality_reports_short_aba_flip_with_stable_indexes() {
        let segments = vec![
            segment("first", 0.0, 2.0, "A"),
            segment("short", 2.0, 2.4, "B"),
            segment("again", 2.4, 4.0, "A"),
        ];

        let report = speaker_quality_report(&segments);

        assert_eq!(report.version, SPEAKER_QUALITY_METADATA_KEY);
        assert_eq!(report.warning_count, 1);
        assert_eq!(
            report.warnings[0].kind,
            SpeakerQualityWarningKind::ShortFlip
        );
        assert_eq!(report.warnings[0].segment_indexes, vec![0, 1, 2]);
        assert_eq!(report.warnings[0].speaker_ids, vec!["A", "B", "A"]);
    }

    #[test]
    fn speaker_quality_never_warns_without_speakers_or_valid_duration() {
        let report = speaker_quality_report(&[
            TimedSegment {
                text: "a".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(1.0),
                ..TimedSegment::default()
            },
            segment("b", 1.0, 1.4, "B"),
        ]);

        assert_eq!(report.warning_count, 0);
        assert!(report.warnings.is_empty());
    }
}
