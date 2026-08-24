import { describe, expect, it } from "vitest";

import {
  buildTranscriptReviewQueue,
  LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD,
  LOW_CONFIDENCE_WORD_THRESHOLD,
  parseTranscriptReviewDecisions,
} from "./transcriptReview";

describe("transcript review queue", () => {
  it("groups a weak word with adjacent uncertain words using backend thresholds", () => {
    const queue = buildTranscriptReviewQueue(JSON.stringify({
      version: 2,
      segments: [{
        text: "alpha beta gamma delta",
        words: [
          { text: "alpha", confidence: 0.94 },
          { text: "beta", confidence: LOW_CONFIDENCE_WORD_THRESHOLD },
          { text: "gamma", confidence: LOW_CONFIDENCE_SPAN_CONTINUATION_THRESHOLD },
          { text: "delta", confidence: 0.9 },
        ],
      }],
    }));

    expect(queue).toHaveLength(1);
    expect(queue[0]).toMatchObject({
      id: "segment-0-words-1-2",
      originalText: "beta gamma",
      confidence: LOW_CONFIDENCE_WORD_THRESHOLD,
      kind: "low_confidence",
    });
  });

  it("does not queue words above the weak threshold", () => {
    const queue = buildTranscriptReviewQueue(JSON.stringify({
      version: 2,
      segments: [{ text: "clear speech", words: [
        { text: "clear", confidence: 0.59 },
        { text: "speech", confidence: 0.99 },
      ] }],
    }));
    expect(queue).toEqual([]);
  });

  it("falls back to segment review when word confidence is unavailable", () => {
    const queue = buildTranscriptReviewQueue(JSON.stringify({
      version: 2,
      segments: [{
        text: "engine without word scores",
        start_seconds: 1.5,
        end_seconds: 3.5,
      }],
    }));

    expect(queue).toEqual([expect.objectContaining({
      id: "segment-0-confidence-unavailable",
      originalText: "engine without word scores",
      confidence: null,
      startSeconds: 1.5,
      endSeconds: 3.5,
      kind: "confidence_unavailable",
    })]);
  });

  it("returns an empty queue for invalid metadata", () => {
    expect(buildTranscriptReviewQueue("not-json")).toEqual([]);
    expect(buildTranscriptReviewQueue(null)).toEqual([]);
  });

  it("restores only valid persisted review actions", () => {
    expect(parseTranscriptReviewDecisions(JSON.stringify({
      version: 1,
      decisions: {
        first: { action: "corrected" },
        second: { action: "unknown" },
      },
    }))).toEqual({ first: "corrected" });
    expect(parseTranscriptReviewDecisions("invalid")).toEqual({});
  });
});
