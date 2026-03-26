import { describe, expect, it } from "vitest";

import {
  WHISPER_CONFIDENCE_COLORS,
  buildConfidenceTranscript,
  whisperConfidenceColorIndex,
} from "./whisperConfidence";

describe("whisper confidence helpers", () => {
  it("matches the whisper.cpp cubic band mapping", () => {
    expect(WHISPER_CONFIDENCE_COLORS).toHaveLength(7);
    expect(whisperConfidenceColorIndex(0)).toBe(0);
    expect(whisperConfidenceColorIndex(0.8)).toBe(3);
    expect(whisperConfidenceColorIndex(0.91)).toBe(5);
    expect(whisperConfidenceColorIndex(1)).toBe(6);
    expect(whisperConfidenceColorIndex(4)).toBe(6);
  });

  it("builds transcript fragments from raw transcript and timeline confidence words", () => {
    const rawTranscript = "hello world\nsecond line";
    const timeline = JSON.stringify({
      version: 2,
      segments: [
        {
          text: "hello world",
          words: [
            { text: "hello", confidence: 0.91 },
            { text: "world", confidence: 0.42 },
          ],
        },
        {
          text: "second line",
          words: [
            { text: "second", confidence: 0.2 },
            { text: "line" },
          ],
        },
      ],
    });

    const document = buildConfidenceTranscript(rawTranscript, timeline);

    expect(document).not.toBeNull();
    expect(document?.confidenceWordCount).toBe(3);
    expect(document?.fragments.map((fragment) => fragment.text).join("")).toBe(rawTranscript);
    expect(document?.fragments.find((fragment) => fragment.text === "hello")?.confidence).toBe(0.91);
    expect(document?.fragments.find((fragment) => fragment.text === "world")?.confidence).toBe(0.42);
    expect(document?.fragments.find((fragment) => fragment.text.includes("line"))?.confidence).toBeNull();
  });

  it("ignores whisper control tokens when deciding whether confidence colors are available", () => {
    const rawTranscript = "strutture";
    const timeline = JSON.stringify({
      version: 2,
      segments: [
        {
          text: "strutture",
          words: [
            { text: "[_BEG_]", confidence: 1.0 },
            { text: "str", confidence: 0.99 },
            { text: "utt", confidence: 0.95 },
            { text: "ure", confidence: 0.94 },
            { text: "[_TT_1000]", confidence: 1.0 },
          ],
        },
      ],
    });

    const document = buildConfidenceTranscript(rawTranscript, timeline);

    expect(document).not.toBeNull();
    expect(document?.confidenceWordCount).toBe(3);
    expect(document?.fragments.map((fragment) => fragment.text).join("")).toBe(rawTranscript);
  });
});
