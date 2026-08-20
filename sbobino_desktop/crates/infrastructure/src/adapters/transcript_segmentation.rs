use sbobino_domain::{collapse_consecutive_repeated_segments, TimedSegment, TimedWord};

const GAP_BREAK_SECONDS: f32 = 1.25;
const MAX_TIMED_SEGMENT_CHARS: usize = 140;
const MIN_TIMED_SEGMENT_CHARS: usize = 48;
const MAX_TIMED_SEGMENT_DURATION_SECONDS: f32 = 12.0;
const MIN_UNTIMED_SEGMENT_WORDS: usize = 6;
const MIN_TERMINAL_SEGMENT_WORDS: usize = 3;
const MAX_UNTIMED_SEGMENT_WORDS: usize = 24;
const MAX_UNTIMED_SEGMENT_CHARS: usize = 160;

pub fn normalize_transcript_segments(
    transcript: &str,
    segments: &[TimedSegment],
    total_audio_seconds: Option<f32>,
) -> Vec<TimedSegment> {
    let cleaned_segments = sanitize_segments(segments);
    if cleaned_segments.is_empty() {
        return infer_segments_from_text(transcript, total_audio_seconds);
    }

    if cleaned_segments
        .iter()
        .any(|segment| segment.start_seconds.is_some() || segment.end_seconds.is_some())
    {
        return clamp_timeline_monotonic(merge_timed_segments(cleaned_segments));
    }

    // Model-provided language markers are utterance boundaries. Keep them
    // intact instead of letting the plain-text sentence splitter merge short
    // neighbouring pieces and lose their per-language metadata.
    if cleaned_segments
        .iter()
        .any(|segment| segment.language_code.is_some())
    {
        return infer_timings_for_language_marked_segments(cleaned_segments, total_audio_seconds);
    }

    let normalized_text = cleaned_segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let mut inferred = infer_segments_from_text(&normalized_text, total_audio_seconds);
    for segment in &mut inferred {
        if let Some(source) = cleaned_segments
            .iter()
            .find(|source| source.text.trim() == segment.text.trim())
        {
            segment.language_code = source.language_code.clone();
            segment.language_confidence = source.language_confidence;
        }
    }
    inferred
}

/// Model timestamps and overlapped ASR windows can disagree by a few frames.
/// Persist a non-overlapping timeline without changing text or segment order.
fn clamp_timeline_monotonic(mut segments: Vec<TimedSegment>) -> Vec<TimedSegment> {
    let mut segment_cursor = 0.0_f32;

    for segment in &mut segments {
        let start = segment
            .start_seconds
            .unwrap_or(segment_cursor)
            .max(segment_cursor);
        let end = segment.end_seconds.unwrap_or(start).max(start);
        segment.start_seconds = Some(start);
        segment.end_seconds = Some(end);

        let mut word_cursor = start;
        for word in &mut segment.words {
            let word_start = word.start_seconds.unwrap_or(word_cursor).max(word_cursor);
            let word_end = word.end_seconds.unwrap_or(word_start).max(word_start);
            word.start_seconds = Some(word_start);
            word.end_seconds = Some(word_end);
            word_cursor = word_end;
        }

        segment_cursor = end;
    }

    segments
}

fn infer_timings_for_language_marked_segments(
    mut segments: Vec<TimedSegment>,
    total_audio_seconds: Option<f32>,
) -> Vec<TimedSegment> {
    let Some(total) = total_audio_seconds.filter(|seconds| seconds.is_finite() && *seconds > 0.0)
    else {
        return segments;
    };

    let weights = segments
        .iter()
        .map(|segment| segment.text.split_whitespace().count().max(1) as f32)
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f32>().max(1.0);
    let mut cursor = 0.0_f32;

    for (index, segment) in segments.iter_mut().enumerate() {
        let duration = if index + 1 == weights.len() {
            (total - cursor).max(0.0)
        } else {
            total * (weights[index] / total_weight)
        };
        segment.start_seconds = Some(cursor);
        cursor = (cursor + duration).min(total);
        segment.end_seconds = Some(cursor);
    }

    segments
}

fn sanitize_segments(segments: &[TimedSegment]) -> Vec<TimedSegment> {
    let sanitized = segments
        .iter()
        .filter_map(|segment| {
            let text = segment.text.trim().to_string();
            if text.is_empty() {
                return None;
            }

            let start_seconds = sanitize_time(segment.start_seconds);
            let end_seconds = sanitize_time(segment.end_seconds);
            let (start_seconds, end_seconds) = match (start_seconds, end_seconds) {
                (Some(start), Some(end)) if end < start => (Some(start), Some(start)),
                values => values,
            };

            let words = segment
                .words
                .iter()
                .filter_map(sanitize_word)
                .collect::<Vec<_>>();

            Some(TimedSegment {
                text,
                start_seconds,
                end_seconds,
                speaker_id: segment.speaker_id.clone(),
                speaker_label: segment.speaker_label.clone(),
                language_code: segment.language_code.clone(),
                language_confidence: segment.language_confidence,
                words,
            })
        })
        .collect::<Vec<_>>();

    collapse_consecutive_repeated_segments(&sanitized)
}

fn sanitize_word(word: &TimedWord) -> Option<TimedWord> {
    let text = word.text.trim().to_string();
    if text.is_empty() {
        return None;
    }

    let start_seconds = sanitize_time(word.start_seconds);
    let end_seconds = sanitize_time(word.end_seconds);
    let (start_seconds, end_seconds) = match (start_seconds, end_seconds) {
        (Some(start), Some(end)) if end < start => (Some(start), Some(start)),
        values => values,
    };

    Some(TimedWord {
        text,
        start_seconds,
        end_seconds,
        confidence: word.confidence.filter(|value| value.is_finite()),
    })
}

fn sanitize_time(value: Option<f32>) -> Option<f32> {
    value.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

fn merge_timed_segments(segments: Vec<TimedSegment>) -> Vec<TimedSegment> {
    let mut merged = Vec::<TimedSegment>::new();
    let mut current: Option<TimedSegment> = None;

    for next in segments {
        match current.take() {
            Some(active) => {
                if should_break_before_next(&active, &next) {
                    merged.push(active);
                    current = Some(next);
                } else {
                    current = Some(merge_two_segments(active, next));
                }
            }
            None => current = Some(next),
        }
    }

    if let Some(active) = current {
        merged.push(active);
    }

    merged
}

fn should_break_before_next(current: &TimedSegment, next: &TimedSegment) -> bool {
    match (
        current.language_code.as_deref(),
        next.language_code.as_deref(),
    ) {
        (Some(current_language), Some(next_language))
            if !current_language.eq_ignore_ascii_case(next_language) =>
        {
            return true;
        }
        (Some(_), None) | (None, Some(_)) => return true,
        _ => {}
    }

    let current_text = current.text.trim();
    let current_chars = current_text.chars().count();
    let combined_chars = current_chars + 1 + next.text.trim().chars().count();

    if current_chars >= MAX_TIMED_SEGMENT_CHARS || combined_chars > MAX_TIMED_SEGMENT_CHARS {
        return true;
    }

    if let (Some(start), Some(end)) = (current.start_seconds, current.end_seconds) {
        if end >= start && end - start >= MAX_TIMED_SEGMENT_DURATION_SECONDS {
            return true;
        }
    }

    if let (Some(end), Some(next_start)) = (current.end_seconds, next.start_seconds) {
        if next_start > end && next_start - end > GAP_BREAK_SECONDS {
            return true;
        }
    }

    if ends_with_strong_boundary(current_text) {
        return true;
    }

    ends_with_soft_boundary(current_text) && current_chars >= MIN_TIMED_SEGMENT_CHARS
}

fn merge_two_segments(left: TimedSegment, right: TimedSegment) -> TimedSegment {
    let text = join_text_parts(&left.text, &right.text);
    let start_seconds = left.start_seconds.or(right.start_seconds);
    let end_seconds = right.end_seconds.or(left.end_seconds);
    let speaker_id = left.speaker_id.or(right.speaker_id);
    let speaker_label = left.speaker_label.or(right.speaker_label);
    let mut words = left.words;
    words.extend(right.words);

    TimedSegment {
        text,
        start_seconds,
        end_seconds,
        speaker_id,
        speaker_label,
        language_code: left.language_code.or(right.language_code),
        language_confidence: left.language_confidence.or(right.language_confidence),
        words,
    }
}

fn infer_segments_from_text(
    transcript: &str,
    total_audio_seconds: Option<f32>,
) -> Vec<TimedSegment> {
    let text_segments = split_plain_text(transcript);
    if text_segments.is_empty() {
        return Vec::new();
    }

    let sanitized_total =
        total_audio_seconds.filter(|seconds| seconds.is_finite() && *seconds > 0.0);
    let weights = text_segments
        .iter()
        .map(|segment| segment.split_whitespace().count().max(1) as f32)
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f32>().max(1.0);

    let mut cursor = 0.0_f32;
    let mut output = Vec::with_capacity(text_segments.len());

    for (index, text) in text_segments.into_iter().enumerate() {
        let (start_seconds, end_seconds) = match sanitized_total {
            Some(total) => {
                let start = cursor;
                let duration = if index + 1 == weights.len() {
                    (total - cursor).max(0.0)
                } else {
                    total * (weights[index] / total_weight)
                };
                cursor = (cursor + duration).min(total);
                (Some(start), Some(cursor))
            }
            None => (None, None),
        };

        output.push(TimedSegment {
            text,
            start_seconds,
            end_seconds,
            speaker_id: None,
            speaker_label: None,
            language_code: None,
            language_confidence: None,
            words: Vec::new(),
        });
    }

    output
}

fn split_plain_text(transcript: &str) -> Vec<String> {
    let normalized = transcript.replace("\r\n", "\n").replace('\r', "\n");
    let mut segments = Vec::<String>::new();
    let mut current = String::new();

    for ch in normalized.chars() {
        if ch == '\n' {
            let current_words = word_count(&current);
            if current.trim().is_empty() {
                flush_segment(&mut segments, &mut current);
                continue;
            }

            if current_words >= MIN_UNTIMED_SEGMENT_WORDS || ends_with_any_boundary(current.trim())
            {
                flush_segment(&mut segments, &mut current);
            } else {
                push_separator(&mut current);
            }
            continue;
        }

        current.push(ch);

        let trimmed = current.trim();
        if trimmed.is_empty() {
            continue;
        }

        let words = word_count(trimmed);
        let chars = trimmed.chars().count();
        if (ends_with_strong_boundary(trimmed) && words >= MIN_TERMINAL_SEGMENT_WORDS)
            || words >= MAX_UNTIMED_SEGMENT_WORDS
            || chars >= MAX_UNTIMED_SEGMENT_CHARS
        {
            flush_segment(&mut segments, &mut current);
        }
    }

    flush_segment(&mut segments, &mut current);

    merge_short_tail_segments(segments)
}

fn merge_short_tail_segments(segments: Vec<String>) -> Vec<String> {
    let mut merged = Vec::<String>::new();

    for segment in segments {
        let segment_words = word_count(&segment);
        let segment_chars = segment.chars().count();

        if segment_words < 4
            && segment_chars < 32
            && !ends_with_strong_boundary(&segment)
            && merged.last().is_some_and(|previous| {
                previous.chars().count() + 1 + segment_chars <= MAX_UNTIMED_SEGMENT_CHARS
            })
        {
            if let Some(previous) = merged.pop() {
                merged.push(join_text_parts(&previous, &segment));
            }
            continue;
        }

        merged.push(segment);
    }

    merged
}

fn flush_segment(segments: &mut Vec<String>, current: &mut String) {
    let normalized = collapse_whitespace(current);
    if !normalized.is_empty() {
        segments.push(normalized);
    }
    current.clear();
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn word_count(value: &str) -> usize {
    value.split_whitespace().count()
}

fn push_separator(buffer: &mut String) {
    if !buffer.chars().last().is_some_and(char::is_whitespace) {
        buffer.push(' ');
    }
}

fn join_text_parts(left: &str, right: &str) -> String {
    let left_trimmed = left.trim();
    let right_trimmed = right.trim();

    if left_trimmed.is_empty() {
        return right_trimmed.to_string();
    }
    if right_trimmed.is_empty() {
        return left_trimmed.to_string();
    }
    if left_trimmed.ends_with('-') {
        return format!("{left_trimmed}{right_trimmed}");
    }

    format!("{left_trimmed} {right_trimmed}")
}

fn ends_with_any_boundary(value: &str) -> bool {
    ends_with_strong_boundary(value) || ends_with_soft_boundary(value)
}

fn ends_with_strong_boundary(value: &str) -> bool {
    value.ends_with('.') || value.ends_with('!') || value.ends_with('?') || value.ends_with('…')
}

fn ends_with_soft_boundary(value: &str) -> bool {
    value.ends_with(',') || value.ends_with(';') || value.ends_with(':')
}

#[cfg(test)]
mod tests {
    use super::normalize_transcript_segments;
    use sbobino_domain::{TimedSegment, TimedWord};

    #[test]
    fn merges_word_level_timed_segments_into_readable_sentences() {
        let segments = vec![
            TimedSegment {
                text: "Hello".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(0.3),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "world.".to_string(),
                start_seconds: Some(0.3),
                end_seconds: Some(0.8),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "Next".to_string(),
                start_seconds: Some(2.4),
                end_seconds: Some(2.8),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "sentence".to_string(),
                start_seconds: Some(2.8),
                end_seconds: Some(3.1),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "here.".to_string(),
                start_seconds: Some(3.1),
                end_seconds: Some(3.6),
                ..TimedSegment::default()
            },
        ];

        let normalized = normalize_transcript_segments("", &segments, Some(10.0));

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].text, "Hello world.");
        assert_eq!(normalized[0].start_seconds, Some(0.0));
        assert_eq!(normalized[0].end_seconds, Some(0.8));
        assert_eq!(normalized[1].text, "Next sentence here.");
        assert_eq!(normalized[1].start_seconds, Some(2.4));
        assert_eq!(normalized[1].end_seconds, Some(3.6));
    }

    #[test]
    fn infers_segment_timing_from_plain_text_when_backend_has_none() {
        let transcript =
            "First sentence here. Second sentence is a bit longer and should stay separate.";

        let normalized = normalize_transcript_segments(transcript, &[], Some(12.0));

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].text, "First sentence here.");
        assert_eq!(
            normalized.last().and_then(|segment| segment.end_seconds),
            Some(12.0)
        );
        assert!(normalized[0]
            .start_seconds
            .zip(normalized[0].end_seconds)
            .is_some_and(|(start, end)| end > start));
    }

    #[test]
    fn rebuilds_segments_from_untimed_backend_lines() {
        let segments = vec![
            TimedSegment {
                text: "First line without timestamps".to_string(),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "continues until the sentence ends here.".to_string(),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "Final short line.".to_string(),
                ..TimedSegment::default()
            },
        ];

        let normalized = normalize_transcript_segments("", &segments, Some(9.0));

        assert_eq!(normalized.len(), 2);
        assert_eq!(
            normalized[0].text,
            "First line without timestamps continues until the sentence ends here."
        );
        assert_eq!(normalized[1].text, "Final short line.");
        assert_eq!(
            normalized.last().and_then(|segment| segment.end_seconds),
            Some(9.0)
        );
    }

    #[test]
    fn keeps_adjacent_confirmed_language_changes_separate() {
        let segments = vec![
            TimedSegment {
                text: "Ciao a tutti.".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(1.0),
                language_code: Some("it".to_string()),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "Hello everyone.".to_string(),
                start_seconds: Some(1.0),
                end_seconds: Some(2.0),
                language_code: Some("en".to_string()),
                ..TimedSegment::default()
            },
        ];

        let normalized = normalize_transcript_segments("", &segments, Some(2.0));

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].language_code.as_deref(), Some("it"));
        assert_eq!(normalized[1].language_code.as_deref(), Some("en"));
    }

    #[test]
    fn preserves_untimed_language_marker_boundaries_when_inferring_timing() {
        let segments = vec![
            TimedSegment {
                text: "Ciao".to_string(),
                language_code: Some("it".to_string()),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "Hello".to_string(),
                language_code: Some("en".to_string()),
                ..TimedSegment::default()
            },
        ];

        let normalized = normalize_transcript_segments("", &segments, Some(4.0));

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].text, "Ciao");
        assert_eq!(normalized[0].language_code.as_deref(), Some("it"));
        assert_eq!(normalized[0].start_seconds, Some(0.0));
        assert_eq!(normalized[0].end_seconds, Some(2.0));
        assert_eq!(normalized[1].text, "Hello");
        assert_eq!(normalized[1].language_code.as_deref(), Some("en"));
        assert_eq!(normalized[1].start_seconds, Some(2.0));
        assert_eq!(normalized[1].end_seconds, Some(4.0));
    }

    #[test]
    fn clamps_overlapped_backend_segments_and_words_without_losing_text() {
        let segments = vec![
            TimedSegment {
                text: "First sentence.".to_string(),
                start_seconds: Some(10.0),
                end_seconds: Some(12.0),
                words: vec![TimedWord {
                    text: "sentence".to_string(),
                    start_seconds: Some(11.5),
                    end_seconds: Some(12.0),
                    confidence: None,
                }],
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "Second sentence.".to_string(),
                start_seconds: Some(11.9),
                end_seconds: Some(12.8),
                words: vec![TimedWord {
                    text: "Second".to_string(),
                    start_seconds: Some(11.9),
                    end_seconds: Some(12.2),
                    confidence: None,
                }],
                ..TimedSegment::default()
            },
        ];

        let normalized = normalize_transcript_segments("", &segments, Some(13.0));

        assert_eq!(normalized.len(), 2);
        assert_eq!(normalized[0].text, "First sentence.");
        assert_eq!(normalized[1].text, "Second sentence.");
        assert_eq!(normalized[1].start_seconds, Some(12.0));
        assert_eq!(normalized[1].words[0].start_seconds, Some(12.0));
        assert_eq!(normalized[1].words[0].end_seconds, Some(12.2));
    }
}
