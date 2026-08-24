import type { TimelineV2, TimelineV2Segment, TimelineV2Word } from "../types";

export const LOW_CONFIDENCE_WORD_THRESHOLD = 0.58;
export const LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD = 0.72;

export type TranscriptReviewItem = {
  id: string;
  segmentIndex: number;
  originalText: string;
  contextText: string;
  confidence: number | null;
  startSeconds: number | null;
  endSeconds: number | null;
  kind: "low_confidence" | "confidence_unavailable";
};

export type PersistedTranscriptReviewAction =
  | "confirmed"
  | "corrected"
  | "ignored";

export function parseTranscriptReviewDecisions(
  value: string | null | undefined,
): Record<string, PersistedTranscriptReviewAction> {
  if (!value) return {};
  try {
    const parsed = JSON.parse(value) as {
      decisions?: Record<string, { action?: unknown }>;
    };
    const decisions: Record<string, PersistedTranscriptReviewAction> = {};
    for (const [id, decision] of Object.entries(parsed.decisions ?? {})) {
      if (
        decision.action === "confirmed"
        || decision.action === "corrected"
        || decision.action === "ignored"
      ) {
        decisions[id] = decision.action;
      }
    }
    return decisions;
  } catch {
    return {};
  }
}

function parseTimeline(timelineV2Json: string | null | undefined): TimelineV2 | null {
  const raw = timelineV2Json?.trim();
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as TimelineV2;
    return Array.isArray(parsed?.segments) ? parsed : null;
  } catch {
    return null;
  }
}

function finiteConfidence(word: TimelineV2Word): number | null {
  return typeof word.confidence === "number" && Number.isFinite(word.confidence)
    ? Math.min(1, Math.max(0, word.confidence))
    : null;
}

function normalizedWordText(word: TimelineV2Word): string {
  return typeof word.text === "string" ? word.text.trim() : "";
}

function spanText(words: TimelineV2Word[]): string {
  return words
    .map(normalizedWordText)
    .filter(Boolean)
    .join(" ")
    .replace(/\s+([,.;:!?])/g, "$1")
    .trim();
}

function segmentFallback(segment: TimelineV2Segment, segmentIndex: number): TranscriptReviewItem | null {
  const text = segment.text?.trim();
  if (!text) return null;
  return {
    id: `segment-${segmentIndex}-confidence-unavailable`,
    segmentIndex,
    originalText: text,
    contextText: text,
    confidence: null,
    startSeconds: segment.start_seconds ?? null,
    endSeconds: segment.end_seconds ?? null,
    kind: "confidence_unavailable",
  };
}

function lowConfidenceItems(
  segment: TimelineV2Segment,
  segmentIndex: number,
): TranscriptReviewItem[] {
  const words = Array.isArray(segment.words) ? segment.words : [];
  const items: TranscriptReviewItem[] = [];
  let cursor = 0;

  while (cursor < words.length) {
    const confidence = finiteConfidence(words[cursor]);
    if (confidence === null || confidence > LOW_CONFIDENCE_WORD_THRESHOLD) {
      cursor += 1;
      continue;
    }

    const start = cursor;
    let end = cursor + 1;
    while (end < words.length) {
      const nextConfidence = finiteConfidence(words[end]);
      if (nextConfidence === null || nextConfidence > LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD) {
        break;
      }
      end += 1;
    }

    const suspectWords = words.slice(start, end);
    const originalText = spanText(suspectWords);
    if (originalText) {
      const confidences = suspectWords
        .map(finiteConfidence)
        .filter((value): value is number => value !== null);
      items.push({
        id: `segment-${segmentIndex}-words-${start}-${end - 1}`,
        segmentIndex,
        originalText,
        contextText: segment.text?.trim() || originalText,
        confidence: confidences.length > 0 ? Math.min(...confidences) : null,
        startSeconds: suspectWords[0]?.start_seconds ?? segment.start_seconds ?? null,
        endSeconds:
          suspectWords[suspectWords.length - 1]?.end_seconds ?? segment.end_seconds ?? null,
        kind: "low_confidence",
      });
    }
    cursor = end;
  }

  return items;
}

export function buildTranscriptReviewQueue(
  timelineV2Json: string | null | undefined,
): TranscriptReviewItem[] {
  const timeline = parseTimeline(timelineV2Json);
  if (!timeline) return [];

  return timeline.segments.flatMap((segment, segmentIndex) => {
    const words = Array.isArray(segment.words) ? segment.words : [];
    const hasWordConfidence = words.some((word) => finiteConfidence(word) !== null);
    if (!hasWordConfidence) {
      const fallback = segmentFallback(segment, segmentIndex);
      return fallback ? [fallback] : [];
    }
    return lowConfidenceItems(segment, segmentIndex);
  });
}
