import { describe, expect, it } from "vitest";

import { isComputeDeviceOptionDisabled } from "./transcriptionCompute";

describe("transcription compute device availability", () => {
  it("keeps Parakeet CPU available for files but disables it for live", () => {
    expect(isComputeDeviceOptionDisabled("parakeet_cpp", "file", "cpu")).toBe(
      false,
    );
    expect(isComputeDeviceOptionDisabled("parakeet_cpp", "live", "cpu")).toBe(
      true,
    );
  });

  it("keeps every Whisper compute option available", () => {
    for (const target of ["file", "live"] as const) {
      for (const device of ["auto", "gpu", "cpu"] as const) {
        expect(
          isComputeDeviceOptionDisabled("whisper_cpp", target, device),
        ).toBe(false);
      }
    }
  });
});
