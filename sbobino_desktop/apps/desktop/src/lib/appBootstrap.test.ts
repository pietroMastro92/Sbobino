import { describe, expect, it, vi } from "vitest";
import type {
  AppSettings,
  ProvisioningModelCatalogEntry,
  ProvisioningStatus,
  RuntimeHealth,
  TranscriptArtifact,
} from "../types";
import { loadInitialAppBootstrapData, type AppBootstrapLoaders } from "./appBootstrap";

function createSettingsFixture(): AppSettings {
  return {
    transcription_engine: "whisper_cpp",
    model: "base",
    language: "it",
    ai_post_processing: false,
    gemini_model: "gemini-2.5-flash",
    gemini_api_key: null,
    gemini_api_key_present: false,
    whisper_cli_path: "",
    whisperkit_cli_path: "",
    ffmpeg_path: "",
    models_dir: "",
    auto_update_enabled: true,
    auto_update_repo: "owner/repo",
    general: {
      auto_update_enabled: true,
      auto_update_repo: "owner/repo",
      privacy_policy_version_accepted: null,
      privacy_policy_accepted_at: null,
      appearance_mode: "system",
      app_language: "it",
    },
    transcription: {
      engine: "whisper_cpp",
      model: "base",
      language: "it",
      whisper_cli_path: "",
      whisperkit_cli_path: "",
      ffmpeg_path: "",
      models_dir: "",
      enable_ai_post_processing: false,
      speaker_diarization: {
        enabled: false,
        device: "cpu",
        speaker_colors: {},
      },
      whisper_options: {
        translate_to_english: false,
        no_context: false,
        split_on_word: false,
        tinydiarize: false,
        diarize: false,
        temperature: 0,
        temperature_increment_on_fallback: 0.2,
        temperature_fallback_count: 0,
        entropy_threshold: 2.4,
        logprob_threshold: -1,
        first_token_logprob_threshold: -1,
        no_speech_threshold: 0.6,
        word_threshold: 0.01,
        best_of: 5,
        beam_size: 5,
        threads: 4,
        processors: 1,
        use_prefill_prompt: false,
        use_prefill_cache: false,
        without_timestamps: false,
        word_timestamps: true,
        prompt: null,
        concurrent_worker_count: 1,
        chunking_strategy: "vad",
        audio_encoder_compute_units: "all",
        text_decoder_compute_units: "all",
      },
    },
    ai: {
      active_provider: "none",
      active_remote_service_id: null,
      providers: {
        foundation_apple: {
          enabled: false,
        },
        gemini: {
          api_key: null,
          has_api_key: false,
          model: "gemini-2.5-flash",
        },
      },
      remote_services: [],
    },
    prompts: {
      templates: [],
      bindings: {
        optimize_prompt_id: "",
        summary_prompt_id: "",
        faq_prompt_id: "",
        emotion_prompt_id: "",
      },
    },
  };
}

function createProvisioningFixture(): ProvisioningStatus {
  return {
    ready: true,
    models_dir: "/tmp/models",
    missing_models: [],
    missing_encoders: [],
    pyannote: {
      enabled: false,
      ready: false,
      runtime_installed: false,
      model_installed: false,
      arch: "arm64",
      device: "cpu",
      source: "managed",
      reason_code: "",
      message: "",
    },
  };
}

function createRuntimeHealthFixture(): RuntimeHealth {
  return {
    app_version: "0.1.14",
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
    },
    ffmpeg_path: "",
    ffmpeg_resolved: "/tmp/ffmpeg",
    ffmpeg_available: true,
    whisper_cli_path: "",
    whisper_cli_resolved: "/tmp/whisper-cli",
    whisper_cli_available: true,
    whisper_stream_path: "",
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
      enabled: true,
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

describe("loadInitialAppBootstrapData", () => {
  it("keeps settings available in the standalone settings window when optional loads fail", async () => {
    const fetchSettingsSnapshot = vi.fn().mockResolvedValue(createSettingsFixture());
    const listRecentArtifacts = vi.fn<() => Promise<TranscriptArtifact[]>>().mockRejectedValue(
      new Error("recent artifacts unavailable"),
    );
    const listDeletedArtifacts = vi.fn().mockRejectedValue(new Error("deleted unavailable"));
    const provisioningStatus = vi.fn().mockRejectedValue(new Error("provisioning unavailable"));
    const provisioningModels = vi.fn().mockRejectedValue(new Error("models unavailable"));
    const fetchRuntimeHealth = vi.fn().mockRejectedValue(new Error("runtime unavailable"));

    const result = await loadInitialAppBootstrapData(
      {
        fetchSettingsSnapshot,
        listRecentArtifacts,
        listDeletedArtifacts,
        provisioningStatus,
        provisioningModels,
        fetchRuntimeHealth,
      },
      {
        standaloneSettingsWindow: true,
        includeDeletedArtifacts: true,
        includeProvisioning: true,
        includeModelCatalog: true,
        includeRuntimeHealth: true,
      },
    );

    expect(result.settings.general.app_language).toBe("it");
    expect(result.activeArtifacts).toBeNull();
    expect(result.deletedArtifacts).toBeNull();
    expect(result.provisioning).toBeNull();
    expect(result.modelCatalog).toBeNull();
    expect(result.runtimeHealth).toBeNull();
    expect(listRecentArtifacts).not.toHaveBeenCalled();
  });

  it("returns successful optional data even if one non-settings load fails", async () => {
    const settings = createSettingsFixture();
    const activeArtifacts: TranscriptArtifact[] = [];
    const deletedArtifacts: TranscriptArtifact[] = [];
    const modelCatalog: ProvisioningModelCatalogEntry[] = [];
    const runtimeHealth = createRuntimeHealthFixture();
    const loaders: AppBootstrapLoaders = {
      fetchSettingsSnapshot: vi.fn().mockResolvedValue(settings),
      listRecentArtifacts: vi.fn().mockResolvedValue(activeArtifacts),
      listDeletedArtifacts: vi.fn().mockResolvedValue(deletedArtifacts),
      provisioningStatus: vi.fn().mockRejectedValue(new Error("provisioning unavailable")),
      provisioningModels: vi.fn().mockResolvedValue(modelCatalog),
      fetchRuntimeHealth: vi.fn().mockResolvedValue(runtimeHealth),
    };

    const result = await loadInitialAppBootstrapData(loaders, {
      standaloneSettingsWindow: false,
      includeActiveArtifacts: true,
      includeDeletedArtifacts: true,
      includeProvisioning: true,
      includeModelCatalog: true,
      includeRuntimeHealth: true,
    });

    expect(result.settings).toBe(settings);
    expect(result.activeArtifacts).toBe(activeArtifacts);
    expect(result.deletedArtifacts).toBe(deletedArtifacts);
    expect(result.provisioning).toBeNull();
    expect(result.modelCatalog).toBe(modelCatalog);
    expect(result.runtimeHealth).toBe(runtimeHealth);
  });

  it("still fails when the settings snapshot itself cannot be loaded", async () => {
    const loaders: AppBootstrapLoaders = {
      fetchSettingsSnapshot: vi.fn().mockRejectedValue(new Error("settings unavailable")),
      listRecentArtifacts: vi.fn().mockResolvedValue([]),
      listDeletedArtifacts: vi.fn().mockResolvedValue([]),
      provisioningStatus: vi.fn().mockResolvedValue(createProvisioningFixture()),
      provisioningModels: vi.fn().mockResolvedValue([]),
      fetchRuntimeHealth: vi.fn().mockResolvedValue(createRuntimeHealthFixture()),
    };

    await expect(loadInitialAppBootstrapData(loaders, {
      standaloneSettingsWindow: true,
    })).rejects.toThrow(
      "settings unavailable",
    );
  });

  it("skips non-essential startup loaders by default", async () => {
    const loaders: AppBootstrapLoaders = {
      fetchSettingsSnapshot: vi.fn().mockResolvedValue(createSettingsFixture()),
      listRecentArtifacts: vi.fn().mockResolvedValue([]),
      listDeletedArtifacts: vi.fn().mockResolvedValue([]),
      provisioningStatus: vi.fn().mockResolvedValue(createProvisioningFixture()),
      provisioningModels: vi.fn().mockResolvedValue([]),
      fetchRuntimeHealth: vi.fn().mockResolvedValue(createRuntimeHealthFixture()),
    };

    const result = await loadInitialAppBootstrapData(loaders, {
      standaloneSettingsWindow: false,
    });

    expect(result.activeArtifacts).toEqual([]);
    expect(result.deletedArtifacts).toBeNull();
    expect(result.provisioning).toBeNull();
    expect(result.modelCatalog).toBeNull();
    expect(result.runtimeHealth).toBeNull();
    expect(loaders.listRecentArtifacts).toHaveBeenCalledTimes(1);
    expect(loaders.listDeletedArtifacts).not.toHaveBeenCalled();
    expect(loaders.provisioningStatus).not.toHaveBeenCalled();
    expect(loaders.provisioningModels).not.toHaveBeenCalled();
  });
});
