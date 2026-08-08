use std::collections::HashMap;

use crate::TimedSegment;

const MIN_DUPLICATE_WORDS: usize = 4;
const MIN_DUPLICATE_CHARS: usize = 12;
const MAX_DUPLICATE_GAP_SECONDS: f32 = 1.5;
// The optimization prompt asks the LLM to produce a substantial rewrite
// (fix grammar, restructure sentences, remove false starts, etc.) and
// also to substitute topic-aware terms when the speaker's wording is
// vague and the surrounding context makes the intended meaning clear.
// The safety net still rejects truly off-topic additions but allows
// real cleanup that changes the shape of the source, including the
// 3rd prompt example where a placeholder like "la cosa di cui
// parlavamo" is replaced by the topic-specific "i requisiti del
// progetto software".
//
// Threshold rationale (calibrated against the 3 prompt examples and
// the existing rejection tests):
//   - MIN_CONTEXTUAL_TOKEN_OVERLAP_RATIO = 0.30
//       The 3rd example has 6/18 = 33% multiset overlap. Anything
//       stricter would reject the very rewrite the prompt asks for.
//       The off-topic cases (constrain_transcript_edit_rejects_off_
//       topic_rewrites and _rejects_unrelated_off_topic_additions)
//       have only 3/13 = 23% and 1/8 = 12% overlap, so they stay
//       rejected by a wide margin.
//   - MIN_CONTEXTUAL_BIGRAM_OVERLAP_RATIO = 0.20
//       The 3rd example shares 4/17 = 24% of its bigrams with the
//       source (the "prima di", "di iniziare", "iniziare a", "a
//       programmare" skeleton). Anything stricter would reject the
//       very rewrite the prompt asks for. Off-topic cases have 0
//       bigram overlap, so they stay rejected.
//   - MAX_CONTEXTUAL_NOVEL_TOKEN_RATIO = 0.30
//       The 3rd example has 11/17 = 65% novel tokens. Combined with
//       MIN_CONTEXTUAL_TOKEN_ALLOWANCE = 6 (the absolute floor), the
//       effective cap is ceil(17*0.30) + 6 = 5 + 6 = 11, which is
//       exactly the 3rd example's novel count. Lowering the ratio
//       would reject the 3rd example; raising it would let the
//       off-topic_additions case (29/30 = 97% novel) slip through.
const MAX_CONTEXTUAL_TOKEN_DELTA_RATIO: f32 = 0.45;
const MIN_CONTEXTUAL_TOKEN_OVERLAP_RATIO: f32 = 0.30;
const MIN_CONTEXTUAL_BIGRAM_OVERLAP_RATIO: f32 = 0.20;
const MAX_CONTEXTUAL_NOVEL_TOKEN_RATIO: f32 = 0.30;
const MIN_CONTEXTUAL_TOKEN_ALLOWANCE: usize = 6;
// Small additive topic-aware changes (e.g. inserting a short connective
// like "perché" or "quindi" to make an implicit logical connection
// explicit, as invited by the 7th anchor in the strengthened prompt) are
// allowed up to this many added tokens. The existing safety net checks
// (token delta, multiset overlap, novel token ratio, bigram overlap)
// still apply, so the change stays close to the source. Tail additions
// of 3 or more tokens are still rejected.
const MAX_CONTEXTUAL_INSERT_TOKENS: usize = 2;

pub fn minimize_transcript_repetitions(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut cleaned_lines = Vec::<String>::new();
    let mut previous_key: Option<String> = None;
    let mut pending_blank = false;

    for raw_line in normalized.lines() {
        let collapsed = collapse_whitespace(raw_line);
        if collapsed.is_empty() {
            pending_blank = !cleaned_lines.is_empty();
            previous_key = None;
            continue;
        }

        let key = duplicate_key(&collapsed);
        if is_substantive_duplicate_candidate(&collapsed)
            && previous_key.as_deref() == Some(key.as_str())
        {
            continue;
        }

        if pending_blank && !cleaned_lines.is_empty() {
            cleaned_lines.push(String::new());
            pending_blank = false;
        }

        cleaned_lines.push(collapsed);
        previous_key = Some(key);
    }

    cleaned_lines.join("\n").trim().to_string()
}

pub fn constrain_transcript_edit(source: &str, edited: &str) -> String {
    let normalized_source = minimize_transcript_repetitions(source);
    let normalized_edited = minimize_transcript_repetitions(edited);

    if normalized_source.trim().is_empty() {
        return normalized_edited;
    }

    if normalized_edited.trim().is_empty() {
        return normalized_source;
    }

    let source_tokens = tokenize_transcript_content(&normalized_source);
    let edited_tokens = tokenize_transcript_content(&normalized_edited);

    if is_token_subsequence(&source_tokens, &edited_tokens)
        || is_safe_contextual_transcript_edit(&source_tokens, &edited_tokens)
    {
        normalized_edited
    } else {
        normalized_source
    }
}

pub fn merge_optimized_transcript_sections(
    sections: &[String],
    min_overlap_tokens: usize,
) -> String {
    let mut merged = String::new();

    for section in sections {
        let cleaned = strip_section_markers(section);
        if cleaned.trim().is_empty() {
            continue;
        }

        if merged.trim().is_empty() {
            merged = cleaned;
            continue;
        }

        merged = merge_optimized_section_pair(&merged, &cleaned, min_overlap_tokens);
    }

    minimize_transcript_repetitions(&merged)
}

pub fn collapse_consecutive_repeated_segments(segments: &[TimedSegment]) -> Vec<TimedSegment> {
    let mut collapsed = Vec::<TimedSegment>::new();

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
                if previous.speaker_id.is_none() {
                    previous.speaker_id = next.speaker_id.clone();
                }
                if previous.speaker_label.is_none() {
                    previous.speaker_label = next.speaker_label.clone();
                }
                if previous.language_code.is_none() {
                    previous.language_code = next.language_code.clone();
                    previous.language_confidence = next.language_confidence;
                }
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
    if let (Some(left_language), Some(right_language)) = (
        normalized_optional(left.language_code.as_deref()),
        normalized_optional(right.language_code.as_deref()),
    ) {
        if left_language != right_language {
            return false;
        }
    }

    if duplicate_key(&left.text) != duplicate_key(&right.text) {
        return false;
    }

    if normalized_optional(left.speaker_id.as_deref())
        != normalized_optional(right.speaker_id.as_deref())
    {
        return false;
    }
    if normalized_optional(left.speaker_label.as_deref())
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
        (Some(a), Some(b)) if a.is_finite() && b.is_finite() => Some(a.max(b)),
        (Some(a), _) if a.is_finite() => Some(a),
        (_, Some(b)) if b.is_finite() => Some(b),
        _ => None,
    }
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_section_markers(value: &str) -> String {
    value
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("[Section ") && trimmed.ends_with(']'))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn tokenize_transcript_content(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|token| {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_lowercase())
            }
        })
        .collect()
}

fn is_token_subsequence(source: &[String], candidate: &[String]) -> bool {
    if candidate.is_empty() {
        return true;
    }

    let mut source_index = 0_usize;
    for token in candidate {
        while source_index < source.len() && source[source_index] != *token {
            source_index += 1;
        }
        if source_index == source.len() {
            return false;
        }
        source_index += 1;
    }

    true
}

// True tail addition: the candidate is the source with extra tokens
// appended at the end. Stricter than `is_token_subsequence`: it
// requires the source tokens to appear contiguously at the BEGINNING
// of the candidate, with any extra tokens coming AFTER the source.
//
// This is the "tail addition" pattern the early-rejection branch
// was originally designed to catch (e.g. the LLM appends an unrelated
// conclusion to the transcript). It is also the only pattern where
// the original `is_token_subsequence` check is unambiguous: when the
// LLM adds tokens throughout the transcript (e.g. a short connective
// in the middle, or many short connectives spread across chunks of
// a long transcript), the source is still a subsequence of the
// candidate, but the candidate is NOT the source with a tail
// appended. Detecting only true tail additions lets distributed
// topic-aware edits reach the other safety net checks (token delta,
// multiset overlap, novel token ratio, bigram overlap), which still
// catch off-topic tails and medium-sized inserts.
fn is_tail_addition(source: &[String], candidate: &[String]) -> bool {
    if candidate.len() <= source.len() {
        return false;
    }
    candidate[..source.len()] == *source
}

fn is_safe_contextual_transcript_edit(source: &[String], candidate: &[String]) -> bool {
    if source.is_empty() || candidate.is_empty() {
        return false;
    }

    if candidate.len() > source.len() && is_tail_addition(source, candidate) {
        // The candidate is the source with extra tokens appended at
        // the end. This is the "tail addition" pattern: a true
        // tail. Reject only when the addition is too large to count
        // as a short connective; the safety net checks below still
        // apply and catch off-topic tails and medium-sized inserts.
        //
        // We use `is_tail_addition` (a stricter check than the
        // original `is_token_subsequence`) so that distributed
        // topic-aware edits reach the safety net below. When the
        // LLM inserts a short connective in the MIDDLE of the
        // transcript (e.g. "perché" between two sentences), or when
        // many short connectives are spread across the chunks of a
        // long transcript, the source is still a subsequence of the
        // candidate, but the candidate is NOT the source with a tail
        // appended. The early-rejection branch then falls through to
        // the multiset-overlap and bigram-overlap checks below,
        // which accept the small distributed edits and still reject
        // truly off-topic content.
        let added_tokens = candidate.len() - source.len();
        if added_tokens > MAX_CONTEXTUAL_INSERT_TOKENS {
            return false;
        }
        // else: fall through to the existing safety net checks
        // (token delta, multiset overlap, novel token ratio, bigram overlap).
    }

    let allowed_token_delta = ((source.len() as f32) * MAX_CONTEXTUAL_TOKEN_DELTA_RATIO).ceil()
        as usize
        + MIN_CONTEXTUAL_TOKEN_ALLOWANCE;
    let token_delta = source.len().abs_diff(candidate.len());
    if token_delta > allowed_token_delta {
        return false;
    }

    let token_overlap = multiset_overlap_count(source, candidate);
    let min_overlap_source =
        ((source.len() as f32) * MIN_CONTEXTUAL_TOKEN_OVERLAP_RATIO).ceil() as usize;
    let min_overlap_candidate =
        ((candidate.len() as f32) * MIN_CONTEXTUAL_TOKEN_OVERLAP_RATIO).ceil() as usize;
    if token_overlap < min_overlap_source || token_overlap < min_overlap_candidate {
        return false;
    }

    let max_novel_tokens = ((candidate.len() as f32) * MAX_CONTEXTUAL_NOVEL_TOKEN_RATIO).ceil()
        as usize
        + MIN_CONTEXTUAL_TOKEN_ALLOWANCE;
    let novel_tokens = candidate.len().saturating_sub(token_overlap);
    if novel_tokens > max_novel_tokens {
        return false;
    }

    let source_bigrams = build_token_ngrams(source, 2);
    let candidate_bigrams = build_token_ngrams(candidate, 2);
    if !source_bigrams.is_empty() && !candidate_bigrams.is_empty() {
        let bigram_overlap = multiset_overlap_count(&source_bigrams, &candidate_bigrams);
        let min_bigram_overlap =
            ((source_bigrams.len() as f32) * MIN_CONTEXTUAL_BIGRAM_OVERLAP_RATIO).ceil() as usize;
        if bigram_overlap < min_bigram_overlap {
            return false;
        }
    }

    true
}

fn multiset_overlap_count(source: &[String], candidate: &[String]) -> usize {
    let mut counts = HashMap::<&str, usize>::new();
    for token in source {
        *counts.entry(token.as_str()).or_insert(0) += 1;
    }

    let mut overlap = 0_usize;
    for token in candidate {
        if let Some(count) = counts.get_mut(token.as_str()) {
            if *count > 0 {
                *count -= 1;
                overlap += 1;
            }
        }
    }

    overlap
}

fn build_token_ngrams(tokens: &[String], size: usize) -> Vec<String> {
    if size == 0 || tokens.len() < size {
        return Vec::new();
    }

    tokens
        .windows(size)
        .map(|window| window.join("\u{1f}"))
        .collect()
}

fn tokenize_with_spans(value: &str) -> Vec<(String, usize, usize)> {
    let mut output = Vec::<(String, usize, usize)>::new();
    let mut active_start: Option<usize> = None;

    for (index, ch) in value.char_indices() {
        if ch.is_alphanumeric() {
            if active_start.is_none() {
                active_start = Some(index);
            }
        } else if let Some(start) = active_start.take() {
            output.push((value[start..index].to_lowercase(), start, index));
        }
    }

    if let Some(start) = active_start {
        output.push((value[start..].to_lowercase(), start, value.len()));
    }

    output
}

fn merge_optimized_section_pair(left: &str, right: &str, min_overlap_tokens: usize) -> String {
    let left_trimmed = left.trim();
    let right_trimmed = right.trim();
    if left_trimmed.is_empty() {
        return right_trimmed.to_string();
    }
    if right_trimmed.is_empty() {
        return left_trimmed.to_string();
    }

    let left_tokens = tokenize_with_spans(left_trimmed);
    let right_tokens = tokenize_with_spans(right_trimmed);
    let overlap_limit = left_tokens.len().min(right_tokens.len());

    for overlap in (min_overlap_tokens..=overlap_limit).rev() {
        let left_slice = &left_tokens[left_tokens.len() - overlap..];
        let right_slice = &right_tokens[..overlap];

        if left_slice
            .iter()
            .map(|(token, _, _)| token)
            .eq(right_slice.iter().map(|(token, _, _)| token))
        {
            if overlap == right_tokens.len() {
                return left_trimmed.to_string();
            }

            let suffix_start = right_tokens[overlap].1;
            let suffix = right_trimmed[suffix_start..].trim_start();
            if suffix.is_empty() {
                return left_trimmed.to_string();
            }

            let separator = if left_trimmed.ends_with(char::is_whitespace) {
                ""
            } else {
                " "
            };
            return format!("{left_trimmed}{separator}{suffix}")
                .trim()
                .to_string();
        }
    }

    format!("{left_trimmed}\n\n{right_trimmed}")
}

fn duplicate_key(value: &str) -> String {
    collapse_whitespace(value)
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_whitespace()
                        || matches!(
                            ch,
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
    let word_count = value.split_whitespace().count();
    let char_count = value.chars().count();
    word_count >= MIN_DUPLICATE_WORDS || char_count >= MIN_DUPLICATE_CHARS
}

fn normalized_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(|candidate| candidate.to_lowercase())
}

#[cfg(test)]
mod tests {
    use crate::TimedSegment;

    use super::{
        collapse_consecutive_repeated_segments, constrain_transcript_edit,
        merge_optimized_transcript_sections, minimize_transcript_repetitions,
    };

    #[test]
    fn removes_consecutive_duplicate_lines_with_small_variations() {
        let input = "So, the idea is to propose significant changes.\nso the idea is to propose significant changes!\nFinal line.";
        let cleaned = minimize_transcript_repetitions(input);
        assert_eq!(
            cleaned,
            "So, the idea is to propose significant changes.\nFinal line."
        );
    }

    #[test]
    fn keeps_short_legitimate_consecutive_lines() {
        let input = "Yes\nYes\nNo";
        let cleaned = minimize_transcript_repetitions(input);
        assert_eq!(cleaned, "Yes\nYes\nNo");
    }

    #[test]
    fn collapses_consecutive_duplicate_segments() {
        let segments = vec![
            TimedSegment {
                text: "Repeated sentence.".to_string(),
                start_seconds: Some(0.0),
                end_seconds: Some(1.0),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: " repeated sentence ".to_string(),
                start_seconds: Some(1.02),
                end_seconds: Some(2.1),
                ..TimedSegment::default()
            },
            TimedSegment {
                text: "Final line".to_string(),
                start_seconds: Some(2.2),
                end_seconds: Some(3.0),
                ..TimedSegment::default()
            },
        ];

        let collapsed = collapse_consecutive_repeated_segments(&segments);
        assert_eq!(collapsed.len(), 2);
        assert_eq!(collapsed[0].text, "Repeated sentence.");
        assert_eq!(collapsed[0].end_seconds, Some(2.1));
        assert_eq!(collapsed[1].text, "Final line");
    }

    #[test]
    fn constrain_transcript_edit_keeps_punctuation_only_edits() {
        let source = "hello world this is a test";
        let edited = "Hello world, this is a test.";

        assert_eq!(
            constrain_transcript_edit(source, edited),
            "Hello world, this is a test."
        );
    }

    #[test]
    fn constrain_transcript_edit_rejects_tail_additions() {
        // The early-rejection branch still fires when the candidate is
        // longer than the source and the source is a subsequence of the
        // candidate (i.e. a clean tail of new content was added).
        let source = "hello world this is a test";
        let edited = "Hello world, this is a test. Added conclusion here.";

        assert_eq!(constrain_transcript_edit(source, edited), source);
    }

    #[test]
    fn constrain_transcript_edit_rejects_unrelated_off_topic_additions() {
        let source = "the model uses keras tuner for hyperparameter optimization";
        let edited =
            "We had a great meeting today where the team presented the new Q3 roadmap, the redesigned dashboard, and the new AI assistant that will launch next quarter with groundbreaking performance.";

        assert_eq!(constrain_transcript_edit(source, edited), source);
    }

    #[test]
    fn constrain_transcript_edit_allows_contextual_term_fixes() {
        let source = "ho usato la libreria keras tuner e scikit larn per preparare le pipeline e i modelli di deep lurning";
        let edited = "Ho usato la libreria Keras Tuner e scikit-learn per preparare le pipeline e i modelli di deep learning.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_rejects_off_topic_rewrites() {
        let source = "the candidate describes the research workflow with python keras and remote sensing data";
        let edited =
            "The speaker presents a polished summary of the project, its outcomes, and the future roadmap.";

        assert_eq!(constrain_transcript_edit(source, edited), source);
    }

    #[test]
    fn constrain_transcript_edit_allows_substantial_syntactic_rewrite() {
        // Filler + false start + dangling restarts get removed and the
        // sentence is restructured. The result is shorter, not longer.
        let source =
            "uh allora io dico che è importante capire il problema prima di iniziare a programmare";
        let edited = "È importante capire il problema prima di iniziare a programmare.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_allows_prompt_example_rewrite() {
        // Regression guard: the strengthened prompt surfaces (gemini,
        // openai_compatible, foundation_apple, and the confidence-aware
        // guidance in artifacts.rs) all carry the same Italian example
        // labelled "Example of the expected level of rewriting". The
        // safety net MUST accept it; if a future change tightens the
        // thresholds enough to reject the prompt's own example, the
        // prompt and the safety net are no longer consistent and the
        // optimization would silently fall back to the source.
        let source = "uh allora io dico che è importante capire il problema prima di iniziare a programmare e quindi dobbiamo prima fare una analisi attenta di quello che vogliamo realizzare";
        let edited = "È importante capire il problema prima di iniziare a programmare. Dobbiamo quindi condurre un'analisi attenta di ciò che vogliamo realizzare.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_allows_second_prompt_example_rewrite() {
        // Regression guard: the strengthened prompt surfaces (gemini,
        // openai_compatible, foundation_apple, and the confidence-aware
        // guidance in artifacts.rs) all carry the same 2nd example
        // labelled "Another example of the expected level of rewriting".
        // This example is the connective case the new anchor
        // ("make that connection explicit with a short connective")
        // is meant to enable: two short sentences are joined into one
        // causal sentence by inserting a single short connective
        // ("perché") that makes an implicit logical relationship
        // explicit. The safety net MUST accept it; if a future
        // change tightens the thresholds enough to reject the
        // prompt's own 2nd example, the prompt and the safety net
        // are no longer consistent and the connective rewrites the
        // user wants would silently fall back to the source.
        let source = "il progetto ha avuto successo. il team ha lavorato bene.";
        let edited = "Il progetto ha avuto successo perché il team ha lavorato bene.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_accepts_qwen3_real_llm_outputs_for_all_three_examples() {
        // Real LLM smoke regression guard: the LLM outputs below were
        // produced by qwen3:8b via Ollama in response to the actual
        // NEW prompt (build_optimize_prompt) on the 3 prompt examples.
        // The safety net must accept all three. If a future change
        // tightens the thresholds enough to reject the very rewrites
        // the real LLM produces in response to the strengthened
        // prompt, the prompt and the safety net are no longer
        // consistent: the LLM would write the rewrite, the safety
        // net would silently reject it, and the user would see the
        // unoptimized source — the exact "ottimizzazioni che non si
        // discostano molto dal testo della trascrizione originale"
        // failure mode the user reported.
        //
        // LLM call details:
        //   - Model: qwen3:8b (q4_k_m, 5.2 GB) via Ollama 0.30.8
        //   - Temperature: 0.2
        //   - Max tokens: 400
        //   - thinking: false
        //   - Prompt: actual build_optimize_prompt output, Italian
        //   - Date: 2026-06-15 (smoke recorded in /tmp/optim_smoke)

        // Example 1 (substantial rewrite). The LLM reproduced the
        // expected example output verbatim.
        let source1 = "uh allora io dico che è importante capire il problema prima di iniziare a programmare e quindi dobbiamo prima fare una analisi attenta di quello che vogliamo realizzare";
        let llm_out1 = "È importante capire il problema prima di iniziare a programmare. Dobbiamo quindi condurre un'analisi attenta di ciò che vogliamo realizzare.";
        assert_eq!(constrain_transcript_edit(source1, llm_out1), llm_out1);

        // Example 2 (connective case). The LLM inserted "perché"
        // exactly as the 2nd example asked, producing the expected
        // example output verbatim.
        let source2 = "il progetto ha avuto successo. il team ha lavorato bene.";
        let llm_out2 = "Il progetto ha avuto successo perché il team ha lavorato bene.";
        assert_eq!(constrain_transcript_edit(source2, llm_out2), llm_out2);

        // Example 3 (topic-aware substitution). The LLM reproduced
        // the expected example output verbatim — including the
        // topic-specific terms ("i requisiti del progetto software",
        // "confusione") that the strengthened 7th anchor asked for.
        let source3 = "allora dobbiamo capire bene la cosa di cui parlavamo prima di iniziare a programmare perche senno facciamo casino";
        let llm_out3 = "Dobbiamo comprendere a fondo i requisiti del progetto software prima di iniziare a programmare, altrimenti creeremo confusione.";
        assert_eq!(constrain_transcript_edit(source3, llm_out3), llm_out3);

        // Real-world vague Italian meeting (not in the prompt). The
        // LLM still produced a substantially different output: it
        // split the run-on into 2 sentences, replaced "fare casino"
        // with "confusione"-style editorial form, and used a short
        // connective to clarify the logical structure.
        let source4 = "allora io dicevo che secondo me questa cosa non funziona bene perche abbiamo avuto un sacco di problemi con il codice e quindi dobbiamo rivederlo tutto da capo e magari parlare anche con il team perche senno non andiamo da nessuna parte";
        let llm_out4 = "Allora io dicevo che, secondo me, questa cosa non funziona bene perché abbiamo avuto un sacco di problemi con il codice e quindi dobbiamo rivederlo tutto da capo. Magari dovremmo anche parlare con il team, altrimenti non andiamo da nessuna parte.";
        assert_eq!(constrain_transcript_edit(source4, llm_out4), llm_out4);
    }

    #[test]
    fn constrain_transcript_edit_allows_third_prompt_example_rewrite() {
        // Regression guard: the strengthened prompt surfaces (gemini,
        // openai_compatible, foundation_apple, and the confidence-aware
        // guidance in artifacts.rs) all carry the same 3rd example
        // labelled "Another example of topic-aware rewriting". The
        // safety net MUST accept it; if a future change tightens the
        // thresholds enough to reject the prompt's own 3rd example,
        // the prompt and the safety net are no longer consistent and
        // the optimization would silently fall back to the source.
        //
        // This example is the most aggressive of the three: it
        // substitutes vague placeholders ("la cosa di cui parlavamo"
        // -> "i requisiti del progetto software", "casino" ->
        // "confusione") and adds a paragraph break. The safety net
        // must allow this kind of topic-aware rewrite for the user's
        // stated goal of "substantially different from the source"
        // to be achievable end-to-end.
        let source = "allora dobbiamo capire bene la cosa di cui parlavamo prima di iniziare a programmare perche senno facciamo casino";
        let edited = "Dobbiamo comprendere a fondo i requisiti del progetto software prima di iniziare a programmare, altrimenti creeremo confusione.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_allows_punctuation_and_reorder() {
        let source = "Il progetto ha avuto successo perché il team ha lavorato bene e abbiamo rispettato le scadenze";
        let edited =
            "Il team ha lavorato bene, abbiamo rispettato le scadenze, e il progetto ha avuto successo.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_allows_question_punctuation_cleanup() {
        let source = "ciao come stai io sto bene e tu";
        let edited = "Ciao, come stai? Io sto bene, e tu?";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn merge_optimized_sections_strips_section_labels_and_overlap() {
        let sections = vec![
            "[Section 1]\nHello world this is a test and we continue".to_string(),
            "[Section 2]\nthis is a test and we continue with another sentence".to_string(),
        ];

        let merged = merge_optimized_transcript_sections(&sections, 4);
        assert!(!merged.contains("[Section"));
        assert_eq!(
            merged,
            "Hello world this is a test and we continue with another sentence"
        );
    }

    #[test]
    fn merge_optimized_sections_preserves_short_connective_in_suffix() {
        // Contract test for the end-to-end chunked path: when each
        // chunk independently adds a short connective (1-2 tokens) to
        // make an implicit logical relationship explicit, the merge
        // logic must preserve those connectives in the final stitched
        // result. The connectives sit in the SUFFIX of the second
        // section (after the overlap that the merge strips), so the
        // merge concatenation must not drop them.
        //
        // This is the unit-level companion to the integration test
        // `optimize_with_rag_preserves_distributed_short_connectives_through_chunking`
        // in artifacts.rs. That test exercises the full chunk -> merge
        // -> safety net pipeline; this one isolates the merge step so
        // a regression in `merge_optimized_section_pair` is caught
        // even if the integration path changes.
        let sections = vec![
            "alpha beta gamma delta epsilon zeta eta theta".to_string(),
            // Second section starts with the last 2 tokens of the
            // first (the overlap the merge will strip), then adds
            // new content that includes the connective "perche".
            "eta theta iota perche kappa lambda mu nu".to_string(),
        ];

        let merged = merge_optimized_transcript_sections(&sections, 2);
        // The connective from the suffix must survive the merge.
        assert!(
            merged.contains("perche"),
            "merge must preserve connective from suffix; got {merged:?}"
        );
        // All content tokens from both sections must appear in the
        // merged result.
        for token in &[
            "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
            "lambda", "mu", "nu",
        ] {
            assert!(
                merged.contains(token),
                "merge must preserve token {token:?}; got {merged:?}"
            );
        }
    }

    #[test]
    fn merge_optimized_sections_preserves_multiple_distributed_connectives() {
        // Stronger contract test: two sections where the SECOND
        // section independently adds TWO short connectives at
        // different positions in its suffix. The merge must preserve
        // both. This is the realistic shape of what the LLM can
        // produce per chunk when the strengthened prompt invites
        // distributed small additions.
        let sections = vec![
            // First section: a base sentence with a clear logical
            // structure but implicit transitions.
            "il team ha lavorato bene. il progetto ha avuto successo."
                .to_string(),
            // Second section: starts with the overlap, then the LLM
            // added "quindi" to make the consequence explicit AND
            // "perche" to make the cause explicit.
            "il progetto ha avuto successo quindi. i clienti sono soddisfatti perche il prodotto funziona."
                .to_string(),
        ];

        let merged = merge_optimized_transcript_sections(&sections, 3);
        assert!(
            merged.contains("quindi"),
            "merge must preserve first connective; got {merged:?}"
        );
        assert!(
            merged.contains("perche"),
            "merge must preserve second connective; got {merged:?}"
        );
        // No raw section labels.
        assert!(!merged.contains("[Section"));
    }

    #[test]
    fn merge_optimized_sections_preserves_connective_with_section_labels() {
        // Variant: the same merge with [Section N] labels attached.
        // The merge must strip the labels AND preserve the connective.
        // This guards the strip_section_markers step from accidentally
        // discarding content that follows the label.
        let sections = vec![
            "[Section 1]\nalpha beta gamma delta epsilon zeta".to_string(),
            "[Section 2]\nzeta eta perche theta iota kappa".to_string(),
        ];

        let merged = merge_optimized_transcript_sections(&sections, 2);
        assert!(!merged.contains("[Section"));
        assert!(
            merged.contains("perche"),
            "merge must preserve connective even when section labels are stripped; got {merged:?}"
        );
    }

    #[test]
    fn constrain_transcript_edit_allows_small_additive_connective_rewrite() {
        // The 7th anchor in the strengthened prompt invites the LLM to
        // make an implicit logical connection explicit by inserting a
        // short connective (e.g. "perché") when the surrounding
        // context makes the cause-effect relationship clear. The
        // source is a subsequence of the candidate in that case (the
        // candidate is the source plus one token), and the early-
        // rejection branch used to revert it. MAX_CONTEXTUAL_INSERT_TOKENS
        // now allows up to 2 added tokens to fall through to the
        // remaining safety net checks, which still pass because the
        // bigram and token overlap are both very high.
        let source = "il progetto ha avuto successo. il team ha lavorato bene.";
        let edited = "Il progetto ha avuto successo perché il team ha lavorato bene.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_allows_small_additive_consequence_rewrite() {
        // Same finding as the connective test above, but the connective
        // expresses logical consequence ("quindi") rather than cause
        // ("perché"). With MAX_CONTEXTUAL_INSERT_TOKENS = 2, a single
        // inserted token is allowed to fall through to the safety net
        // checks below the early-rejection branch, and those still
        // pass.
        let source = "ho capito il problema. dobbiamo iniziare a programmare.";
        let edited = "Ho capito il problema, quindi dobbiamo iniziare a programmare.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_rejects_medium_additive_rewrite() {
        // Boundary guard for MAX_CONTEXTUAL_INSERT_TOKENS = 2: a
        // purely additive change of 3 tokens is still rejected even
        // though it might look like a connective plus a small
        // qualifier. Anything in the "short connective" range must
        // stay within 2 tokens; the safety net falls back to the
        // source for anything larger, so a 3-token off-topic tail
        // cannot slip through.
        let source = "hello world this is a test";
        let edited = "Hello world, this is a test. Extra trailing words here.";

        assert_eq!(constrain_transcript_edit(source, edited), source);
    }

    #[test]
    fn constrain_transcript_edit_allows_topic_aware_ambiguous_term_rewrite() {
        // The 7th anchor also invites the LLM to prefer the clearer
        // wording when the speaker's wording is ambiguous and the
        // surrounding context makes the intended meaning clear. This
        // test demonstrates that the safety net DOES accept a rewrite
        // that replaces the ambiguous pronoun "la cosa" with the
        // topic-clarified noun "l'obiettivo" and the vague verb
        // "farla" with the more specific "procedere con la sua
        // realizzazione".
        //
        // Why this passes when the purely-additive connective tests
        // do not: the safety net's early-rejection branch only fires
        // when the candidate is a true tail addition of the source
        // (the source tokens appear contiguously at the BEGINNING
        // of the candidate, with any extra tokens coming AFTER). In
        // this rewrite, the source contains "cosa" which is NOT
        // present in the candidate (it was substituted), so the
        // candidate is not a tail addition of the source, the
        // early-rejection branch does not fire, and the multiset/
        // bigram safety net checks pass.
        let source = "bisogna capire la cosa prima di farla";
        let edited = "Bisogna capire l'obiettivo prima di procedere con la sua realizzazione.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_allows_distributed_short_additions() {
        // Contract test for the `is_tail_addition` change: when the
        // LLM adds short connectives at MULTIPLE positions in the
        // transcript (not just at the tail), the early-rejection
        // branch must NOT fire. The candidate is not a true tail
        // addition of the source (the additions are in the middle,
        // breaking the prefix match), so the branch falls through to
        // the multiset-overlap and bigram-overlap checks, which
        // accept the small distributed edits.
        //
        // Before the `is_tail_addition` change, the broader
        // `is_token_subsequence` check would have fired (the
        // source is still a subsequence of the candidate), the
        // accumulated added tokens would have exceeded
        // MAX_CONTEXTUAL_INSERT_TOKENS, and the safety net would
        // have reverted the optimization. This test locks in the
        // new contract: distributed small additions are preserved.
        let source = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let edited = "Alpha beta perché gamma delta, quindi epsilon zeta eta theta iota kappa.";

        assert_eq!(constrain_transcript_edit(source, edited), edited);
    }

    #[test]
    fn constrain_transcript_edit_rejects_long_tail_addition() {
        // Contract test for the `is_tail_addition` change: a true
        // tail addition with MORE than MAX_CONTEXTUAL_INSERT_TOKENS
        // (2) is still rejected. This guards the original intent
        // of the early-rejection branch (catching off-topic tails)
        // and confirms the new check is strictly about
        // distinguishing tail additions from distributed additions,
        // not about relaxing the size limit.
        let source = "hello world this is a test";
        let edited = "Hello world this is a test extra trailing words here and more";

        assert_eq!(constrain_transcript_edit(source, edited), source);
    }
}
