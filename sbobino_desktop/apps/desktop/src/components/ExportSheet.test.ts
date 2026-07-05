import { describe, expect, it } from "vitest";

import { buildPreviewContent, type ExportFormat, type ExportStyle } from "./ExportSheet";

describe("buildPreviewContent", () => {
  it("includes speaker names in segments preview when requested", () => {
    const preview = buildPreviewContent({
      transcriptText: "Fallback transcript",
      segments: [
        { time: "00:12", line: "Alice opens the meeting.", speakerLabel: "Alice" },
        { time: "00:24", line: "Bob confirms the next step.", speakerLabel: "Bob" },
      ],
      style: "segments",
      format: "txt",
      includeTimestamps: true,
      includeSpeakerNames: true,
      language: "en",
      title: "Meeting",
    });

    expect(preview).toContain("[00:12] Alice: Alice opens the meeting.");
    expect(preview).toContain("[00:24] Bob: Bob confirms the next step.");
  });

  it("adds the speaker column to segments csv preview when requested", () => {
    const preview = buildPreviewContent({
      transcriptText: "Fallback transcript",
      segments: [
        { time: "00:12", line: "Alice opens the meeting.", speakerLabel: "Alice" },
      ],
      style: "segments",
      format: "csv",
      includeTimestamps: true,
      includeSpeakerNames: true,
      language: "en",
      title: "Meeting",
    });

    expect(preview).toContain("Start Timestamp;End Timestamp;Transcript;Speaker");
    expect(preview).toContain("00:12;00:23;\"Alice opens the meeting.\";\"Alice\"");
  });

  it("builds every exposed export mode without dropping speaker or timeline content", () => {
    const cases: Array<{ style: ExportStyle; formats: ExportFormat[] }> = [
      { style: "transcript", formats: ["txt", "docx", "html", "pdf", "md"] },
      { style: "subtitles", formats: ["srt", "vtt"] },
      { style: "segments", formats: ["txt", "csv", "docx", "html", "pdf", "md", "json"] },
    ];

    for (const { style, formats } of cases) {
      for (const format of formats) {
        const preview = buildPreviewContent({
          transcriptText: "Fallback transcript",
          segments: [
            { time: "00:12", line: "Alice opens the meeting.", speakerLabel: "Alice" },
            { time: "00:24", line: "Bob confirms the next step.", speakerLabel: "Bob" },
          ],
          style,
          format,
          includeTimestamps: true,
          includeSpeakerNames: true,
          language: "en",
          title: "Meeting",
          summary: "Short summary",
          faqs: "Q: Next?\nA: Follow up.",
        });

        expect(preview.trim(), `${style}/${format}`).not.toBe("");
        if (style === "subtitles" && format === "vtt") {
          expect(preview).toContain("WEBVTT");
        }
        if (style === "subtitles" && format === "srt") {
          expect(preview).toContain("00:00:12,000 --> 00:00:23,000");
        }
        if (style !== "transcript" || format !== "csv") {
          expect(preview).toContain("Alice");
        }
      }
    }
  });
});
