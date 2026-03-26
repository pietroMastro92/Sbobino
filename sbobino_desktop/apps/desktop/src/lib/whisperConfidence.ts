import type { TimelineV2 } from "../types";
import { t } from "../i18n";

export const WHISPER_CONFIDENCE_COLORS = [
  "rgb(220, 5, 12)",
  "rgb(232, 96, 28)",
  "rgb(241, 147, 45)",
  "rgb(246, 193, 65)",
  "rgb(247, 240, 86)",
  "rgb(144, 201, 135)",
  "rgb(78, 178, 101)",
] as const;

export type ConfidenceTranscriptFragment = {
  text: string;
  confidence: number | null;
  color: string | null;
  colorIndex: number | null;
  tooltip: string | null;
};

export type ConfidenceTranscriptDocument = {
  fragments: ConfidenceTranscriptFragment[];
  confidenceWordCount: number;
};

type TimelineWordLike = {
  text?: unknown;
  confidence?: unknown;
};

type TimelineSegmentLike = {
  text?: unknown;
  words?: unknown;
};

function parseNonEmptyText(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function normalizeConfidence(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return null;
  }
  if (value <= 0) {
    return 0;
  }
  if (value >= 1) {
    return 1;
  }
  return value;
}

function isWhisperControlToken(tokenText: string): boolean {
  return tokenText.startsWith("[_") && tokenText.endsWith("]");
}

function appendFragment(
  fragments: ConfidenceTranscriptFragment[],
  fragment: ConfidenceTranscriptFragment,
): void {
  if (!fragment.text) {
    return;
  }

  const previous = fragments.length > 0 ? fragments[fragments.length - 1] : undefined;
  if (
    previous
    && previous.confidence === fragment.confidence
    && previous.color === fragment.color
    && previous.colorIndex === fragment.colorIndex
    && previous.tooltip === fragment.tooltip
  ) {
    previous.text += fragment.text;
    return;
  }

  fragments.push(fragment);
}

function plainFragment(text: string): ConfidenceTranscriptFragment {
  return {
    text,
    confidence: null,
    color: null,
    colorIndex: null,
    tooltip: null,
  };
}

function parseTimeline(timelineV2Json: string | null | undefined): TimelineV2 | null {
  const raw = timelineV2Json?.trim();
  if (!raw) {
    return null;
  }

  try {
    const parsed = JSON.parse(raw) as TimelineV2 | null;
    return Array.isArray(parsed?.segments) ? parsed : null;
  } catch {
    return null;
  }
}

function formatConfidenceTooltip(confidence: number): string {
  return t("confidence.tooltip", "{percent}% confidence", {
    percent: Math.round(confidence * 100),
  });
}

function buildAnnotatedSegmentFragments(
  segmentText: string,
  words: unknown,
): ConfidenceTranscriptDocument {
  const fragments: ConfidenceTranscriptFragment[] = [];
  const segmentLower = segmentText.toLocaleLowerCase();
  let cursor = 0;
  let confidenceWordCount = 0;

  const tokenEntries = Array.isArray(words)
    ? words
      .map((word) => (word && typeof word === "object" ? word as TimelineWordLike : null))
      .filter((word): word is TimelineWordLike => Boolean(word))
    : [];

  if (tokenEntries.length === 0) {
    return {
      fragments: [plainFragment(segmentText)],
      confidenceWordCount: 0,
    };
  }

  for (const token of tokenEntries) {
    const tokenText = parseNonEmptyText(token.text);
    if (!tokenText) {
      continue;
    }
    if (isWhisperControlToken(tokenText)) {
      continue;
    }

    const matchIndex = segmentLower.indexOf(tokenText.toLocaleLowerCase(), cursor);
    if (matchIndex === -1) {
      return {
        fragments: [plainFragment(segmentText)],
        confidenceWordCount: 0,
      };
    }

    appendFragment(fragments, plainFragment(segmentText.slice(cursor, matchIndex)));

    const matchedText = segmentText.slice(matchIndex, matchIndex + tokenText.length);
    const confidence = normalizeConfidence(token.confidence);
    if (confidence === null) {
      appendFragment(fragments, plainFragment(matchedText));
    } else {
      const colorIndex = whisperConfidenceColorIndex(confidence);
      appendFragment(fragments, {
        text: matchedText,
        confidence,
        color: WHISPER_CONFIDENCE_COLORS[colorIndex],
        colorIndex,
        tooltip: formatConfidenceTooltip(confidence),
      });
      confidenceWordCount += 1;
    }

    cursor = matchIndex + tokenText.length;
  }

  appendFragment(fragments, plainFragment(segmentText.slice(cursor)));

  return {
    fragments,
    confidenceWordCount,
  };
}

function buildFromSegments(segments: TimelineSegmentLike[]): ConfidenceTranscriptDocument | null {
  const fragments: ConfidenceTranscriptFragment[] = [];
  let emittedSegments = 0;
  let confidenceWordCount = 0;

  for (const segment of segments) {
    const text = parseNonEmptyText(segment.text);
    if (!text) {
      continue;
    }

    if (emittedSegments > 0) {
      appendFragment(fragments, plainFragment("\n"));
    }

    const annotated = buildAnnotatedSegmentFragments(text, segment.words);
    annotated.fragments.forEach((fragment) => appendFragment(fragments, fragment));
    confidenceWordCount += annotated.confidenceWordCount;
    emittedSegments += 1;
  }

  if (confidenceWordCount === 0 || fragments.length === 0) {
    return null;
  }

  return { fragments, confidenceWordCount };
}

export function whisperConfidenceColorIndex(confidence: number): number {
  const paletteSize = WHISPER_CONFIDENCE_COLORS.length;
  const clamped = normalizeConfidence(confidence) ?? 0;
  const rawIndex = Math.floor((clamped ** 3) * paletteSize);
  return Math.min(paletteSize - 1, Math.max(0, rawIndex));
}

export function buildConfidenceTranscript(
  rawTranscript: string,
  timelineV2Json: string | null | undefined,
): ConfidenceTranscriptDocument | null {
  const parsedTimeline = parseTimeline(timelineV2Json);
  if (!parsedTimeline) {
    return null;
  }

  const normalizedSegments = parsedTimeline.segments
    .map((segment) => (segment && typeof segment === "object" ? segment as TimelineSegmentLike : null))
    .filter((segment): segment is TimelineSegmentLike => Boolean(segment));

  if (normalizedSegments.length === 0) {
    return null;
  }

  if (!rawTranscript) {
    return buildFromSegments(normalizedSegments);
  }

  const transcriptLower = rawTranscript.toLocaleLowerCase();
  const fragments: ConfidenceTranscriptFragment[] = [];
  let transcriptCursor = 0;
  let confidenceWordCount = 0;

  for (const segment of normalizedSegments) {
    const segmentText = parseNonEmptyText(segment.text);
    if (!segmentText) {
      continue;
    }

    const segmentIndex = transcriptLower.indexOf(
      segmentText.toLocaleLowerCase(),
      transcriptCursor,
    );
    if (segmentIndex === -1) {
      return buildFromSegments(normalizedSegments);
    }

    appendFragment(
      fragments,
      plainFragment(rawTranscript.slice(transcriptCursor, segmentIndex)),
    );

    const annotated = buildAnnotatedSegmentFragments(segmentText, segment.words);
    annotated.fragments.forEach((fragment) => appendFragment(fragments, fragment));
    confidenceWordCount += annotated.confidenceWordCount;
    transcriptCursor = segmentIndex + segmentText.length;
  }

  appendFragment(fragments, plainFragment(rawTranscript.slice(transcriptCursor)));

  if (confidenceWordCount === 0 || fragments.length === 0) {
    return null;
  }

  return { fragments, confidenceWordCount };
}
