import type { TranscriptionEngine } from "../types";

export function isComputeDeviceOptionDisabled(
  engine: TranscriptionEngine,
  target: "file" | "live",
  device: "auto" | "gpu" | "cpu",
): boolean {
  return engine === "parakeet_cpp" && target === "live" && device === "cpu";
}
