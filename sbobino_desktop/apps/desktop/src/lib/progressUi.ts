import type { JobProgress } from "../types";

export function clampPercentage(value: number): number {
  if (!Number.isFinite(value)) return 0;
  if (value < 0) return 0;
  if (value > 100) return 100;
  return value;
}

export function makeProgressVisible(value: number): number {
  const clamped = clampPercentage(value);
  if (clamped > 0 && clamped < 1) {
    return 1;
  }
  return clamped;
}

export function formatProgressPercentageLabel(value: number, estimated = false): string {
  const rounded = Math.round(makeProgressVisible(value));
  if (rounded === 100) {
    return "100%";
  }
  return `${estimated ? "~" : ""}${String(rounded).padStart(2, "0")}%`;
}

export function percentageFromJobProgress(
  progress: JobProgress | null | undefined,
): number {
  if (!progress) return 0;
  if (Number.isFinite(progress.percentage)) {
    return clampPercentage(progress.percentage);
  }
  const currentSeconds = progress.current_seconds ?? null;
  const totalSeconds = progress.total_seconds ?? null;
  if (currentSeconds !== null && totalSeconds !== null && totalSeconds > 0) {
    return clampPercentage((currentSeconds / totalSeconds) * 100);
  }
  return 0;
}
