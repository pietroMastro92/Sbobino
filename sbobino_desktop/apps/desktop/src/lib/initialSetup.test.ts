import { describe, expect, it } from "vitest";

import type { ProvisioningModelCatalogEntry, RuntimeHealth } from "../types";
import {
  getInitialSetupMissingModels,
  isInitialSetupComplete,
  shouldRepairPyannoteRuntime,
} from "./initialSetup";

function createRuntimeHealthFixture(): RuntimeHealth {
  return {
    host_os: "macos",
    host_arch: "aarch64",
    is_apple_silicon: true,
    preferred_engine: "whisper_cpp",
    configured_engine: "whisper_cpp",
    ffmpeg_path: "ffmpeg",
    ffmpeg_resolved: "/tmp/ffmpeg",
    ffmpeg_available: true,
    whisper_cli_path: "whisper-cli",
    whisper_cli_resolved: "/tmp/whisper-cli",
    whisper_cli_available: true,
    whisper_stream_path: "whisper-stream",
    whisper_stream_resolved: "/tmp/whisper-stream",
    whisper_stream_available: true,
    models_dir_configured: "/tmp/models",
    models_dir_resolved: "/tmp/models",
    model_filename: "ggml-base.bin",
    model_present: true,
    coreml_encoder_present: true,
    missing_models: [],
    missing_encoders: [],
    pyannote: {
      enabled: false,
      ready: true,
      runtime_installed: true,
      model_installed: true,
      arch: "aarch64-apple-darwin",
      device: "cpu",
      source: "release_asset",
      reason_code: "ok",
      message: "ready",
    },
    setup_complete: true,
  };
}

function createModelCatalogFixture(): ProvisioningModelCatalogEntry[] {
  return [
    {
      key: "base",
      label: "Base",
      model_file: "ggml-base.bin",
      installed: true,
      coreml_installed: true,
    },
    {
      key: "large_turbo",
      label: "Large Turbo",
      model_file: "ggml-large-v3-turbo-q8_0.bin",
      installed: true,
      coreml_installed: true,
    },
  ];
}

describe("initialSetup helpers", () => {
  it("marks version and repair errors as auto-repairable", () => {
    expect(
      shouldRepairPyannoteRuntime({
        enabled: true,
        ready: false,
        runtime_installed: true,
        model_installed: true,
        arch: "aarch64-apple-darwin",
        device: "cpu",
        source: "release_asset",
        reason_code: "pyannote_version_mismatch",
        message: "stale runtime",
      }),
    ).toBe(true);

    expect(
      shouldRepairPyannoteRuntime({
        enabled: true,
        ready: false,
        runtime_installed: true,
        model_installed: true,
        arch: "aarch64-apple-darwin",
        device: "cpu",
        source: "release_asset",
        reason_code: "pyannote_repair_required",
        message: "repair required",
      }),
    ).toBe(true);

    expect(
      shouldRepairPyannoteRuntime({
        enabled: true,
        ready: false,
        runtime_installed: false,
        model_installed: false,
        arch: "aarch64-apple-darwin",
        device: "cpu",
        source: "release_asset",
        reason_code: "pyannote_runtime_missing",
        message: "missing",
      }),
    ).toBe(false);
  });

  it("requires base and large turbo during first-launch setup", () => {
    const catalog = createModelCatalogFixture();
    catalog[1] = {
      ...catalog[1],
      coreml_installed: false,
    };

    expect(getInitialSetupMissingModels(catalog, true)).toEqual(["large_turbo"]);
  });

  it("keeps the app blocked until privacy, runtime, pyannote, and models are all ready", () => {
    const runtimeHealth = createRuntimeHealthFixture();
    const catalog = createModelCatalogFixture();

    expect(isInitialSetupComplete(true, runtimeHealth, catalog)).toBe(true);
    expect(isInitialSetupComplete(false, runtimeHealth, catalog)).toBe(false);

    runtimeHealth.pyannote.ready = false;
    expect(isInitialSetupComplete(true, runtimeHealth, catalog)).toBe(false);
  });
});
