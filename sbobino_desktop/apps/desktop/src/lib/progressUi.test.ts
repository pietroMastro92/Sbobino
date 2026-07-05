import { describe, expect, it } from "vitest";
import {
  clampPercentage,
  formatProgressPercentageLabel,
  makeProgressVisible,
  percentageFromJobProgress,
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
    expect(formatProgressPercentageLabel(42, true)).toBe("~42%");
    expect(formatProgressPercentageLabel(100, true)).toBe("100%");
  });

  it("trusts backend percentage over phase-local current seconds", () => {
    expect(
      percentageFromJobProgress({
        job_id: "job",
        stage: "preparing_audio",
        message: "Preparing audio",
        percentage: 10,
        current_seconds: 7814,
        total_seconds: 7814,
      }),
    ).toBe(10);
  });
});
