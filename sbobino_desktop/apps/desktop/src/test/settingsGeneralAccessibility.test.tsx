import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { changeLanguage } from "../i18n";
import { fetchSettingsSnapshot } from "../lib/tauri";
import { useAppStore } from "../state/useAppStore";
import type { AppSettings } from "../types";

vi.mock("../lib/tauri", () => {
  const asyncNoop = vi.fn(async (..._args: unknown[]) => undefined);
  const subscriptionNoop = vi.fn(async (..._args: unknown[]) => () => undefined);
  return {
    analyzeArtifactEmotions: asyncNoop,
    cancelArtifactSpeakerDiarization: asyncNoop,
    cancelTranscription: asyncNoop,
    chatArtifact: asyncNoop,
    checkUpdates: asyncNoop,
    clearAutomaticImportQuarantineItem: asyncNoop,
    deleteArtifacts: asyncNoop,
    emptyDeletedArtifacts: asyncNoop,
    ensureTranscriptionRuntime: asyncNoop,
    exportAppBackup: asyncNoop,
    exportArtifact: asyncNoop,
    fetchAiCapabilityStatus: asyncNoop,
    fetchRealtimeStartReadiness: asyncNoop,
    fetchTranscriptionStartPreflight: asyncNoop,
    fetchRuntimeHealth: asyncNoop,
    fetchRuntimeStatus: asyncNoop,
    fetchSettingsSnapshot: vi.fn(),
    listTranscriptionLanguages: asyncNoop,
    generateArtifactPack: asyncNoop,
    getArtifact: asyncNoop,
    hardDeleteArtifacts: asyncNoop,
    importAppBackup: asyncNoop,
    listArtifactChat: asyncNoop,
    listAiServiceModels: asyncNoop,
    listDeletedArtifacts: asyncNoop,
    listGeminiModels: asyncNoop,
    listRecentArtifacts: asyncNoop,
    openSettingsWindow: asyncNoop,
    pauseRealtime: asyncNoop,
    planPyannoteBackgroundAction: asyncNoop,
    previewArtifactExport: asyncNoop,
    provisioningCancel: asyncNoop,
    provisioningDownloadModel: asyncNoop,
    provisioningInstallPyannote: asyncNoop,
    provisioningInstallRuntime: asyncNoop,
    provisioningModels: asyncNoop,
    provisioningStart: asyncNoop,
    provisioningStatus: asyncNoop,
    readArtifactAudio: asyncNoop,
    readAudioFile: asyncNoop,
    renameArtifact: asyncNoop,
    retryAutomaticImportQuarantineItem: asyncNoop,
    resetPromptTemplates: asyncNoop,
    resumeRealtime: asyncNoop,
    restoreArtifacts: asyncNoop,
    runArtifactSpeakerDiarization: asyncNoop,
    saveSettings: asyncNoop,
    saveSettingsPartial: asyncNoop,
    scanAutomaticImport: asyncNoop,
    startRealtime: asyncNoop,
    startTranscription: asyncNoop,
    stopRealtime: asyncNoop,
    subscribeArtifactSpeakerDiarizationProgress: subscriptionNoop,
    subscribeArtifactUpdated: subscriptionNoop,
    subscribeJobCompleted: subscriptionNoop,
    subscribeJobFailed: subscriptionNoop,
    subscribeJobProgress: subscriptionNoop,
    subscribeProvisioningProgress: subscriptionNoop,
    subscribeProvisioningStatus: subscriptionNoop,
    subscribeRealtimeDelta: subscriptionNoop,
    subscribeRealtimeInputLevel: subscriptionNoop,
    subscribeMenuCheckUpdates: subscriptionNoop,
    subscribeRealtimeSaved: subscriptionNoop,
    subscribeRealtimeStatus: subscriptionNoop,
    subscribeSettingsNavigate: subscriptionNoop,
    subscribeSettingsUpdated: subscriptionNoop,
    subscribeTranscriptionDelta: subscriptionNoop,
    summarizeArtifact: asyncNoop,
    optimizeArtifact: asyncNoop,
    testPromptTemplate: asyncNoop,
    testAiService: asyncNoop,
    updateArtifact: asyncNoop,
    updateArtifactTimeline: asyncNoop,
    writeTrimmedAudio: asyncNoop,
    writeSetupReport: asyncNoop,
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: vi.fn(() => ({
    listen: vi.fn(async () => () => undefined),
  })),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    hide: vi.fn(async () => undefined),
    isFullscreen: vi.fn(async () => false),
    listen: vi.fn(async () => () => undefined),
    onResized: vi.fn(async () => () => undefined),
    setTheme: vi.fn(async () => undefined),
  })),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(async () => false),
  message: vi.fn(async () => undefined),
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  exit: vi.fn(async () => undefined),
  relaunch: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(async () => null),
}));

function createSettingsFixture(): AppSettings {
  return {
    transcription_engine: "whisper_cpp",
    model: "base",
    language: "en",
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
      app_language: "en",
    },
    transcription: {
      engine: "whisper_cpp",
      compute_device: "auto",
      live_compute_device: "auto",
      model: "base",
      parakeet_model: "tdt06b_v3_q4",
      language: "en",
      whisper_cli_path: "",
      whisperkit_cli_path: "",
      parakeet_cli_path: "parakeet-cli",
      ffmpeg_path: "",
      models_dir: "",
      parakeet_models_dir: "parakeet-models",
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
    automation: {
      enabled: false,
      run_scan_on_app_start: true,
      scan_interval_minutes: 15,
      allowed_extensions: ["wav"],
      watched_sources: [],
      excluded_folders: [],
      source_statuses: [],
      recent_activity: [],
      quarantined_items: [],
    },
    organization: { workspaces: [] },
    ai: {
      active_provider: "none",
      active_remote_service_id: null,
      providers: {
        foundation_apple: { enabled: false },
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

describe("General settings accessibility", () => {
  let localStorageDescriptor: PropertyDescriptor | undefined;

  beforeEach(() => {
    changeLanguage("en");
    const settings = createSettingsFixture();
    useAppStore.setState({
      settings,
      selectedFile: null,
      activeJobId: null,
      progress: null,
      error: null,
      artifacts: [],
    });
    vi.mocked(fetchSettingsSnapshot).mockResolvedValue(settings);
    localStorageDescriptor = Object.getOwnPropertyDescriptor(
      window,
      "localStorage",
    );
    const values = new Map<string, string>();
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        clear: () => values.clear(),
        getItem: (key: string) => values.get(key) ?? null,
        key: (index: number) => Array.from(values.keys())[index] ?? null,
        removeItem: (key: string) => values.delete(key),
        setItem: (key: string, value: string) => values.set(key, value),
        get length() {
          return values.size;
        },
      },
    });
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    if (localStorageDescriptor) {
      Object.defineProperty(window, "localStorage", localStorageDescriptor);
    } else {
      Reflect.deleteProperty(window, "localStorage");
    }
  });

  it("mounts named controls for update checks, appearance, and app language", () => {
    render(<App standaloneSettingsWindow />);

    expect(
      screen.getByRole("checkbox", { name: "Enable auto update checks" }),
    ).toBeChecked();
    expect(
      screen.getByRole("combobox", { name: "Appearance" }),
    ).toHaveValue("system");
    expect(
      screen.getByRole("combobox", { name: "App Language" }),
    ).toHaveValue("en");
  });
});
