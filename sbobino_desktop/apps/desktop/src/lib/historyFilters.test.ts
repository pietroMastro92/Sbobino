import { describe, expect, it } from "vitest";

import type { TranscriptArtifact } from "../types";
import {
  filterHistoryArtifacts,
  historyGeneratedOutputs,
  historyQuality,
  historySourceId,
  historySourceKind,
  historyStatus,
  historyTags,
  normalizeHistoryTags,
} from "./historyFilters";

function artifact(
  overrides: Partial<TranscriptArtifact> = {},
): TranscriptArtifact {
  return {
    id: "artifact",
    job_id: "job",
    title: "Example",
    kind: "file",
    source_label: "Example",
    source_origin: "imported",
    audio_available: true,
    audio_backfill_status: "imported",
    revision: 1,
    raw_transcript: "raw transcript",
    optimized_transcript: "",
    summary: "",
    faqs: "",
    metadata: {},
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

describe("history filters", () => {
  it("normalizes and deduplicates tags with compatibility for absent metadata", () => {
    expect(normalizeHistoryTags([" Course ", "course", "", "Exam"])).toEqual([
      "Course",
      "Exam",
    ]);
    expect(historyTags(artifact())).toEqual([]);
    expect(
      historyTags(
        artifact({
          metadata: {
            auto_import_tags_v1: '[" Course ","course","exam"]',
          },
        }),
      ),
    ).toEqual(["Course", "exam"]);
    expect(
      historyTags(
        artifact({
          metadata: { auto_import_tags_v1: '{"version":1,"tags":["x"]}' },
        }),
      ),
    ).toEqual(["x"]);
  });

  it("classifies automatic, realtime, manual, and unassigned records", () => {
    expect(
      historySourceKind(
        artifact({ metadata: { auto_import_source_id: "lectures" } }),
      ),
    ).toBe("automatic");
    expect(
      historySourceId(artifact({ metadata: { auto_import_source_id: " lectures " } })),
    ).toBe("lectures");
    expect(historySourceKind(artifact({ kind: "realtime", source_origin: "realtime" }))).toBe(
      "realtime",
    );
    expect(historySourceKind(artifact())).toBe("manual");
    expect(
      historySourceKind(
        artifact({ source_origin: "legacy_external", raw_transcript: "" }),
      ),
    ).toBe("unassigned");
  });

  it("derives durable status and quality from metadata with legacy fallback", () => {
    expect(historyStatus(artifact())).toBe("completed");
    expect(
      historyStatus(artifact({ metadata: { processing_status: "running" } })),
    ).toBe("processing");
    expect(historyStatus(artifact({ metadata: { status: "failed" } }))).toBe("failed");
    expect(historyStatus(artifact({ raw_transcript: "", optimized_transcript: "" }))).toBe(
      "unknown",
    );
    expect(
      historyQuality(
        artifact({ metadata: { auto_post_summary_status: "disabled" } }),
      ),
    ).toBe("warning");
    expect(
      historyQuality(artifact({ metadata: { speaker_diarization_status: "failed" } })),
    ).toBe("failed");
  });

  it("detects generated study, meeting, summary, and faq outputs", () => {
    expect(
      historyGeneratedOutputs(
        artifact({
          summary: "summary",
          faqs: "faq",
          metadata: {
            study_pack_v1: '{"body_markdown":"study"}',
            meeting_intelligence_v1: '{"body_markdown":"meeting"}',
          },
        }),
      ),
    ).toEqual(["study", "meeting", "summary", "faqs"]);
  });

  it("filters by source, status, generated output, and tags without mutation", () => {
    const records = [
      artifact({
        id: "automatic-study",
        metadata: {
          auto_import_source_id: "course",
          auto_import_tags_v1: '["Course"]',
          study_pack_v1: '{"body_markdown":"study"}',
        },
      }),
      artifact({
        id: "manual-failed",
        metadata: { status: "failed" },
        raw_transcript: "",
      }),
      artifact({ id: "realtime", kind: "realtime", source_origin: "realtime" }),
    ];
    const result = filterHistoryArtifacts(records, {
      source: "automatic",
      output: "study",
      tag: "course",
    });
    expect(result.map((record) => record.id)).toEqual(["automatic-study"]);
    expect(records).toHaveLength(3);
    expect(
      filterHistoryArtifacts(records, { status: "failed" }).map((record) => record.id),
    ).toEqual(["manual-failed"]);
    expect(
      filterHistoryArtifacts(records, { source: "realtime" }).map((record) => record.id),
    ).toEqual(["realtime"]);
    expect(
      filterHistoryArtifacts(records, { source: "source:course" }).map((record) => record.id),
    ).toEqual(["automatic-study"]);
    expect(filterHistoryArtifacts(records, { output: "none" })).toEqual([
      records[1],
      records[2],
    ]);
  });
});
