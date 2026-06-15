import { describe, expect, it } from "vitest";

import type { ProvisioningModelCatalogEntry, RuntimeHealth } from "../types";
import {
  canWarmStartFromSetupReport,
  getInitialSetupMissingModels,
  isInitialSetupComplete,
  shouldBlockMainUiDuringStartup,
  shouldRepairPyannoteRuntime,
} from "./initialSetup";

function createRuntimeHealthFixture(): RuntimeHealth {
  return {
    app_version: "0.1.16",
    host_os: "macos",
    host_arch: "aarch64",
    is_apple_silicon: true,
    preferred_engine: "whisper_cpp",
    configured_engine: "whisper_cpp",
    runtime_source: "managed_release_asset",
    managed_runtime_required: true,
    managed_runtime: {
      source: "managed_release_asset",
      ready: true,
      ffmpeg: {
        resolved_path: "/tmp/ffmpeg",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
      whisper_cli: {
        resolved_path: "/tmp/whisper-cli",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
      whisper_stream: {
        resolved_path: "/tmp/whisper-stream",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
      parakeet_cli: {
        resolved_path: "/tmp/parakeet-cli",
        available: false,
        failure_reason: "",
        failure_message: "",
      },
    },
    ffmpeg_path: "ffmpeg",
    ffmpeg_resolved: "/tmp/ffmpeg",
    ffmpeg_available: true,
    whisper_cli_path: "whisper-cli",
    whisper_cli_resolved: "/tmp/whisper-cli",
    whisper_cli_available: true,
    whisper_stream_path: "whisper-stream",
    whisper_stream_resolved: "/tmp/whisper-stream",
    whisper_stream_available: true,
    parakeet_cli_path: "parakeet-cli",
    parakeet_cli_resolved: "/tmp/parakeet-cli",
    parakeet_cli_available: false,
    models_dir_configured: "/tmp/models",
    models_dir_resolved: "/tmp/models",
    parakeet_models_dir_configured: "parakeet-models",
    parakeet_models_dir_resolved: "/tmp/parakeet-models",
    model_filename: "ggml-base.bin",
    model_present: true,
    parakeet_model_filename: "tdt-0.6b-v3-q4_k.gguf",
    parakeet_model_present: false,
    missing_parakeet_models: ["tdt-0.6b-v3-q4_k.gguf"],
    coreml_encoder_present: true,
    missing_models: [],
    missing_encoders: [],
    pyannote: {
      enabled: false,
      ready: true,
      runtime_installed: true,
      model_installed: true,
      runtime_dir: "/tmp/runtime/pyannote",
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
      engine: "whisper_cpp",
      experimental: false,
    },
    {
      key: "large_turbo",
      label: "Large Turbo",
      model_file: "ggml-large-v3-turbo-q8_0.bin",
      installed: true,
      coreml_installed: true,
      engine: "whisper_cpp",
      experimental: false,
    },
    {
      key: "tdt06b_v3_q4",
      label: "Parakeet TDT 0.6B v3 Q4_K",
      model_file: "tdt-0.6b-v3-q4_k.gguf",
      installed: true,
      coreml_installed: true,
      engine: "parakeet_cpp",
      experimental: true,
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
        runtime_dir: "/tmp/runtime/pyannote",
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
        runtime_dir: "/tmp/runtime/pyannote",
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
        runtime_dir: "/tmp/runtime/pyannote",
        arch: "aarch64-apple-darwin",
        device: "cpu",
        source: "release_asset",
        reason_code: "pyannote_runtime_missing",
        message: "missing",
      }),
    ).toBe(false);
  });

  it("requires whisper and Parakeet default models during first-launch setup", () => {
    const catalog = createModelCatalogFixture();
    catalog[1] = {
      ...catalog[1],
      coreml_installed: false,
    };
    catalog[2] = {
      ...catalog[2],
      installed: false,
    };

    expect(getInitialSetupMissingModels(catalog, true)).toEqual([
      "large_turbo",
      "tdt06b_v3_q4",
    ]);
  });

  it("requires privacy, runtime, and models, but allows deferred pyannote setup", () => {
    const runtimeHealth = createRuntimeHealthFixture();
    const catalog = createModelCatalogFixture();

    expect(isInitialSetupComplete(true, runtimeHealth, catalog)).toBe(true);
    expect(isInitialSetupComplete(false, runtimeHealth, catalog)).toBe(false);

    runtimeHealth.pyannote.ready = false;
    expect(isInitialSetupComplete(true, runtimeHealth, catalog)).toBe(true);
  });

  it("allows warm start only for trusted completed setup reports", () => {
    const runtimeHealth = createRuntimeHealthFixture();

    expect(
      canWarmStartFromSetupReport(true, {
        build_version: "0.1.16",
        privacy_accepted: true,
        setup_complete: true,
        final_reason_code: "setup_complete",
        final_error: null,
        runtime_health: runtimeHealth,
        steps: [],
        updated_at: new Date().toISOString(),
        trusted_for_fast_start: true,
      }),
    ).toBe(true);

    expect(
      canWarmStartFromSetupReport(true, {
        build_version: "0.1.16",
        privacy_accepted: true,
        setup_complete: true,
        final_reason_code: "setup_complete",
        final_error: "stale",
        runtime_health: runtimeHealth,
        steps: [],
        updated_at: new Date().toISOString(),
        trusted_for_fast_start: true,
      }),
    ).toBe(false);
  });

  it("does not block the main UI for trusted warm starts while diagnostics load in background", () => {
    expect(
      shouldBlockMainUiDuringStartup({
        hasSettings: true,
        privacyAccepted: true,
        warmStartEligible: true,
        startupRequirementsLoaded: false,
        initialSetupReady: false,
      }),
    ).toBe(false);

    expect(
      shouldBlockMainUiDuringStartup({
        hasSettings: true,
        privacyAccepted: true,
        warmStartEligible: false,
        startupRequirementsLoaded: false,
        initialSetupReady: false,
      }),
    ).toBe(true);
  });
});
