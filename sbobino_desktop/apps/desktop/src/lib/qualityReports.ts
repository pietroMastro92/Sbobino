export type QualityReportStatus = "completed" | "unchanged" | "unavailable";

export type SegmentRepairReport = {
  version: "segment_repair_v1";
  status: QualityReportStatus;
  input_segment_count: number;
  output_segment_count: number;
  collapsed_repeated_segment_count: number;
  timestamp_repair_count: number;
  changed: boolean;
};

export type SpeakerQualityWarning = {
  kind: "short_flip" | "rapid_turn";
  segment_indexes: number[];
  speaker_ids: string[];
  duration_seconds?: number;
};

export type SpeakerQualityReport = {
  version: "speaker_quality_v1";
  status: QualityReportStatus;
  warning_count: number;
  warnings: SpeakerQualityWarning[];
};

export type TimelineManualEditsReport = {
  version: "timeline_manual_edits_v1";
  manual_edit_count: number;
  last_edited_at: string;
};

function parseObject(value: string | null | undefined): Record<string, unknown> | null {
  if (!value?.trim()) return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function nonNegativeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 0
    ? value
    : null;
}

function qualityStatus(value: unknown): QualityReportStatus | null {
  return value === "completed" || value === "unchanged" || value === "unavailable"
    ? value
    : null;
}

export function parseSegmentRepairReport(
  value: string | null | undefined,
): SegmentRepairReport | null {
  const parsed = parseObject(value);
  if (!parsed || parsed.version !== "segment_repair_v1") return null;
  const status = qualityStatus(parsed.status);
  const input = nonNegativeInteger(parsed.input_segment_count);
  const output = nonNegativeInteger(parsed.output_segment_count);
  const collapsed = nonNegativeInteger(parsed.collapsed_repeated_segment_count);
  const timestamps = nonNegativeInteger(parsed.timestamp_repair_count);
  if (
    !status ||
    input === null ||
    output === null ||
    collapsed === null ||
    timestamps === null ||
    typeof parsed.changed !== "boolean"
  ) {
    return null;
  }
  return {
    version: "segment_repair_v1",
    status,
    input_segment_count: input,
    output_segment_count: output,
    collapsed_repeated_segment_count: collapsed,
    timestamp_repair_count: timestamps,
    changed: parsed.changed,
  };
}

export function parseSpeakerQualityReport(
  value: string | null | undefined,
): SpeakerQualityReport | null {
  const parsed = parseObject(value);
  if (!parsed || parsed.version !== "speaker_quality_v1") return null;
  const status = qualityStatus(parsed.status);
  if (!status || !Array.isArray(parsed.warnings)) return null;
  const warnings = parsed.warnings.flatMap((candidate): SpeakerQualityWarning[] => {
    if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) return [];
    const warning = candidate as Record<string, unknown>;
    if (warning.kind !== "short_flip" && warning.kind !== "rapid_turn") return [];
    if (!Array.isArray(warning.segment_indexes) || !Array.isArray(warning.speaker_ids)) return [];
    const segmentIndexes = warning.segment_indexes.filter(
      (index): index is number => nonNegativeInteger(index) !== null,
    );
    const speakerIds = warning.speaker_ids.filter(
      (speaker): speaker is string => typeof speaker === "string" && speaker.trim().length > 0,
    );
    if (segmentIndexes.length === 0 || speakerIds.length === 0) return [];
    return [{
      kind: warning.kind,
      segment_indexes: segmentIndexes,
      speaker_ids: speakerIds,
      ...(typeof warning.duration_seconds === "number" && Number.isFinite(warning.duration_seconds)
        ? { duration_seconds: warning.duration_seconds }
        : {}),
    }];
  });
  return {
    version: "speaker_quality_v1",
    status,
    warning_count: warnings.length,
    warnings,
  };
}

export function parseTimelineManualEditsReport(
  value: string | null | undefined,
): TimelineManualEditsReport | null {
  const parsed = parseObject(value);
  const count = parsed ? nonNegativeInteger(parsed.manual_edit_count) : null;
  if (
    !parsed ||
    parsed.version !== "timeline_manual_edits_v1" ||
    count === null ||
    typeof parsed.last_edited_at !== "string"
  ) {
    return null;
  }
  return {
    version: "timeline_manual_edits_v1",
    manual_edit_count: count,
    last_edited_at: parsed.last_edited_at,
  };
}

export function suspiciousSpeakerSegmentIndexes(
  report: SpeakerQualityReport | null,
): Set<number> {
  return new Set(report?.warnings.flatMap((warning) => warning.segment_indexes) ?? []);
}
