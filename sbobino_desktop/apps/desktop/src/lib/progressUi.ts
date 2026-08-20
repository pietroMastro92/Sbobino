export function clampPercentage(value: number): number {
  if (!Number.isFinite(value)) return 0;
  if (value < 0) return 0;
  if (value > 100) return 100;
  return value;
}

export interface StructuredProgressValue {
  percentage: number;
  overall_percentage?: number;
  current_seconds?: number | null;
  total_seconds?: number | null;
}

export function percentageFromProgress(progress: StructuredProgressValue): number {
  if (progress.overall_percentage !== undefined) {
    return clampPercentage(progress.overall_percentage);
  }
  const currentSeconds = progress.current_seconds ?? null;
  const totalSeconds = progress.total_seconds ?? null;
  if (currentSeconds !== null && totalSeconds !== null && totalSeconds > 0) {
    return clampPercentage((currentSeconds / totalSeconds) * 100);
  }
  return clampPercentage(progress.percentage);
}

export function makeProgressVisible(value: number): number {
  const clamped = clampPercentage(value);
  if (clamped > 0 && clamped < 1) {
    return 1;
  }
  return clamped;
}

export function formatProgressPercentageLabel(value: number): string {
  const rounded = Math.round(makeProgressVisible(value));
  if (rounded === 100) {
    return "100%";
  }
  return `${String(rounded).padStart(2, "0")}%`;
}
