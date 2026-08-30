import { describe, expect, it } from "vitest";

import {
  parseSegmentRepairReport,
  parseSpeakerQualityReport,
  parseTimelineManualEditsReport,
  suspiciousSpeakerSegmentIndexes,
} from "./qualityReports";

describe("quality report metadata", () => {
  it("parses versioned repair and manual-edit reports", () => {
    expect(parseSegmentRepairReport(JSON.stringify({
      version: "segment_repair_v1",
      status: "completed",
      input_segment_count: 4,
      output_segment_count: 3,
      collapsed_repeated_segment_count: 1,
      timestamp_repair_count: 0,
      changed: true,
    }))).toMatchObject({ changed: true, output_segment_count: 3 });
    expect(parseTimelineManualEditsReport(JSON.stringify({
      version: "timeline_manual_edits_v1",
      manual_edit_count: 2,
      last_edited_at: "2026-08-25T00:00:00Z",
    }))).toMatchObject({ manual_edit_count: 2 });
  });

  it("keeps only valid warning evidence and exposes navigable indexes", () => {
    const report = parseSpeakerQualityReport(JSON.stringify({
      version: "speaker_quality_v1",
      status: "completed",
      warning_count: 2,
      warnings: [
        { kind: "short_flip", segment_indexes: [0, 1, 2], speaker_ids: ["a", "b", "a"] },
        { kind: "unknown", segment_indexes: [4], speaker_ids: ["c"] },
      ],
    }));
    expect(report?.warning_count).toBe(1);
    expect([...suspiciousSpeakerSegmentIndexes(report)]).toEqual([0, 1, 2]);
  });

  it("rejects malformed or future metadata", () => {
    expect(parseSegmentRepairReport("{}")).toBeNull();
    expect(parseSpeakerQualityReport("not-json")).toBeNull();
    expect(parseTimelineManualEditsReport(JSON.stringify({ version: "timeline_manual_edits_v2" }))).toBeNull();
  });
});
