import { describe, expect, it } from "vitest";
import {
  clampPercentage,
  formatProgressPercentageLabel,
  makeProgressVisible,
  percentageFromProgress,
} from "./progressUi";

describe("progressUi", () => {
  it("clamps invalid percentages", () => {
    expect(clampPercentage(Number.NaN)).toBe(0);
    expect(clampPercentage(-5)).toBe(0);
    expect(clampPercentage(150)).toBe(100);
  });

  it("shows minimal visible progress once work has started", () => {
    expect(makeProgressVisible(0)).toBe(0);
    expect(makeProgressVisible(0.2)).toBe(1);
    expect(makeProgressVisible(0.99)).toBe(1);
    expect(makeProgressVisible(1.4)).toBe(1.4);
  });

  it("formats compact progress labels without getting stuck on 00%", () => {
    expect(formatProgressPercentageLabel(0)).toBe("00%");
    expect(formatProgressPercentageLabel(0.2)).toBe("01%");
    expect(formatProgressPercentageLabel(7.2)).toBe("07%");
    expect(formatProgressPercentageLabel(100)).toBe("100%");
  });

  it("prefers monotonic overall progress over the completed audio cursor", () => {
    expect(
      percentageFromProgress({
        percentage: 85,
        overall_percentage: 85,
        current_seconds: 60,
        total_seconds: 60,
      }),
    ).toBe(85);
    expect(
      percentageFromProgress({ percentage: 42, current_seconds: 5, total_seconds: 10 }),
    ).toBe(50);
  });
});
