import React from "react";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "../App";
import { changeLanguage } from "../i18n";
import {
  fetchRuntimeHealth,
  fetchRuntimeStatus,
  fetchSettingsSnapshot,
  provisioningDownloadModel,
  provisioningInstallPyannote,
  provisioningInstallRuntime,
  provisioningModels,
  provisioningStart,
  saveSettings,
} from "../lib/tauri";
import { useAppStore } from "../state/useAppStore";
import type { InitialSetupReport } from "../lib/initialSetup";
import type {
  AppSettings,
  ProvisioningModelCatalogEntry,
  RuntimeHealth,
} from "../types";

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
    fetchRuntimeHealth: vi.fn(async (..._args: unknown[]) => undefined),
    fetchRuntimeStatus: vi.fn(async (..._args: unknown[]) => undefined),
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
    provisioningDownloadModel: vi.fn(async (..._args: unknown[]) => undefined),
    provisioningInstallPyannote: vi.fn(async (..._args: unknown[]) => undefined),
    provisioningInstallRuntime: vi.fn(async (..._args: unknown[]) => undefined),
    provisioningModels: vi.fn(async (..._args: unknown[]) => undefined),
    provisioningStart: vi.fn(async (..._args: unknown[]) => undefined),
    provisioningStatus: asyncNoop,
    readArtifactAudio: asyncNoop,
    readAudioFile: asyncNoop,
    renameArtifact: asyncNoop,
    retryAutomaticImportQuarantineItem: asyncNoop,
    resetPromptTemplates: asyncNoop,
    resumeRealtime: asyncNoop,
    restoreArtifacts: asyncNoop,
    runArtifactSpeakerDiarization: asyncNoop,
    saveSettings: vi.fn(async (settings: AppSettings) => settings),
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

vi.mock("@tauri-apps/api/core", () => {
  const invoke = vi.fn(async (command: string): Promise<null | undefined> => {
    if (command === "companion_status") return null;
    return undefined;
  });
  return {
    convertFileSrc: vi.fn((path: string) => `asset://${path}`),
    invoke,
  };
});

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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

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

function createRuntimeHealthFixture(
  overrides: Partial<RuntimeHealth> = {},
): RuntimeHealth {
  return {
    app_version: "2.0.36",
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
        resolved_path: "/managed/r3/ffmpeg",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
      whisper_cli: {
        resolved_path: "/managed/r3/whisper-cli",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
      whisper_stream: {
        resolved_path: "/managed/r3/whisper-stream",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
      parakeet_cli: {
        resolved_path: "/managed/r3/parakeet-cli",
        available: true,
        failure_reason: "",
        failure_message: "",
      },
    },
    ffmpeg_path: "",
    ffmpeg_resolved: "/managed/r3/ffmpeg",
    ffmpeg_available: true,
    whisper_cli_path: "",
    whisper_cli_resolved: "/managed/r3/whisper-cli",
    whisper_cli_available: true,
    whisper_stream_path: "",
    whisper_stream_resolved: "/managed/r3/whisper-stream",
    whisper_stream_available: true,
    parakeet_cli_path: "parakeet-cli",
    parakeet_cli_resolved: "/managed/r3/parakeet-cli",
    parakeet_cli_available: true,
    models_dir_configured: "/models",
    models_dir_resolved: "/models",
    parakeet_models_dir_configured: "parakeet-models",
    parakeet_models_dir_resolved: "/models/parakeet",
    model_filename: "ggml-large-v3-turbo-q8_0.bin",
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
      runtime_dir: "/managed/r3/pyannote",
      arch: "aarch64-apple-darwin",
      device: "cpu",
      source: "release_asset",
      reason_code: "pyannote_ready_disabled",
      message: "Pyannote ready but disabled",
    },
    setup_complete: true,
    ...overrides,
  };
}

const staleHealth = createRuntimeHealthFixture({
  runtime_source: "bundle12nov",
  managed_runtime: {
    ...createRuntimeHealthFixture().managed_runtime,
    source: "bundle12nov",
    ready: false,
    ffmpeg: {
      resolved_path: "/managed/bundle12nov/ffmpeg",
      available: false,
      failure_reason: "missing",
      failure_message: "bundle12nov ffmpeg missing",
    },
  },
  ffmpeg_resolved: "/managed/bundle12nov/ffmpeg",
  ffmpeg_available: false,
  whisper_cli_resolved: "/managed/bundle12nov/whisper-cli",
  whisper_cli_available: false,
  model_filename: "ggml-base.bin",
  model_present: true,
  pyannote: {
    enabled: false,
    ready: false,
    runtime_installed: false,
    model_installed: false,
    runtime_dir: "/managed/bundle12nov/pyannote",
    arch: "aarch64-apple-darwin",
    device: "cpu",
    source: "bundle12nov",
    reason_code: "pyannote_runtime_missing",
    message: "Pyannote runtime missing in bundle12nov",
  },
  setup_complete: false,
});

const freshHealth = createRuntimeHealthFixture();

function createSetupReport(runtimeHealth: RuntimeHealth): InitialSetupReport {
  return {
    build_version: "2.0.36",
    privacy_accepted: true,
    setup_complete: false,
    final_reason_code: null,
    final_error: null,
    runtime_health: runtimeHealth,
    steps: [],
    updated_at: "2026-11-12T10:00:00.000Z",
  };
}

function createCatalogFixture(): ProvisioningModelCatalogEntry[] {
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
  ];
}

function renderApp(options: { strict?: boolean } = {}) {
  const app = (
    <App
      standaloneSettingsWindow
      initialBootstrap={{
        runtimeHealth: null,
        provisioning: null,
        modelCatalog: null,
        startupRequirementsLoaded: false,
        setupReport: createSetupReport(staleHealth),
      }}
    />
  );
  return render(options.strict ? <React.StrictMode>{app}</React.StrictMode> : app);
}

describe("Local Models runtime diagnostics freshness", () => {
  let localStorageDescriptor: PropertyDescriptor | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(fetchRuntimeHealth).mockReset().mockResolvedValue(undefined as never);
    vi.mocked(fetchRuntimeStatus).mockReset().mockResolvedValue(undefined as never);
    vi.mocked(provisioningModels).mockReset().mockResolvedValue([]);
    vi.mocked(provisioningInstallPyannote).mockReset().mockResolvedValue({ started: false });
    vi.mocked(provisioningInstallRuntime).mockReset().mockResolvedValue({ started: false });
    vi.mocked(provisioningStart).mockReset().mockResolvedValue({ started: false });
    vi.mocked(provisioningDownloadModel).mockReset().mockResolvedValue({ started: false });
    vi.mocked(saveSettings).mockClear();
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
    localStorageDescriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
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

  it("replaces the stored snapshot with a verified read and keeps actions disabled while checking", async () => {
    const healthRead = deferred<RuntimeHealth>();
    vi.mocked(fetchRuntimeHealth).mockReturnValue(healthRead.promise);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());
    // The old path resolved status quickly. It must not be presented as current.
    vi.mocked(fetchRuntimeStatus).mockResolvedValue(staleHealth);

    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));

    expect(screen.getByText("Checking...")).toBeVisible();
    expect(screen.queryByText("bundle12nov")).toBeNull();
    expect(screen.queryByText("/managed/bundle12nov/pyannote")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Install Pyannote" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Install Local Runtime" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Repair Runtime" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Download Missing Models" }),
    ).toBeDisabled();

    await act(async () => {
      healthRead.resolve(freshHealth);
    });

    await waitFor(() =>
      expect(
        screen.getByText("Pyannote ready on aarch64-apple-darwin."),
      ).toBeVisible(),
    );
    expect(screen.queryByText("bundle12nov")).toBeNull();
    expect(screen.queryByText("/managed/bundle12nov/pyannote")).toBeNull();
    expect(screen.getByText("/managed/r3/pyannote")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: "Repair" })).toBeEnabled();

    expect(provisioningInstallPyannote).not.toHaveBeenCalled();
    expect(provisioningInstallRuntime).not.toHaveBeenCalled();
    expect(provisioningStart).not.toHaveBeenCalled();
    expect(provisioningDownloadModel).not.toHaveBeenCalled();
    expect(saveSettings).not.toHaveBeenCalled();
  });

  it("does not let a slower status read overwrite the verified health", async () => {
    const statusRead = deferred<RuntimeHealth>();
    vi.mocked(fetchRuntimeStatus).mockReturnValue(statusRead.promise);
    vi.mocked(fetchRuntimeHealth).mockResolvedValue(freshHealth);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());

    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /^Transcription/ }));
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));

    await waitFor(() =>
      expect(
        screen.getByText("Pyannote ready on aarch64-apple-darwin."),
      ).toBeVisible(),
    );

    await act(async () => {
      statusRead.resolve(staleHealth);
    });

    expect(screen.queryByText("bundle12nov")).toBeNull();
    expect(
      screen.getByText("Pyannote ready on aarch64-apple-darwin."),
    ).toBeVisible();
    expect(screen.getByText("/managed/r3/pyannote")).toBeVisible();
  });

  it("does not let a slower status failure replace the verified health", async () => {
    const statusRead = deferred<RuntimeHealth>();
    vi.mocked(fetchRuntimeStatus).mockReturnValue(statusRead.promise);
    vi.mocked(fetchRuntimeHealth).mockResolvedValue(freshHealth);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());

    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /^Transcription/ }));
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));

    await waitFor(() =>
      expect(
        screen.getByText("Pyannote ready on aarch64-apple-darwin."),
      ).toBeVisible(),
    );

    await act(async () => {
      statusRead.reject(new Error("status unavailable"));
    });

    expect(
      screen.queryByText(/Could not read transcription runtime health/),
    ).toBeNull();
    expect(
      screen.getByText("Pyannote ready on aarch64-apple-darwin."),
    ).toBeVisible();
  });

  it("reports an unverifiable runtime explicitly and verifies again from Refresh", async () => {
    vi.mocked(fetchRuntimeHealth)
      .mockRejectedValueOnce(new Error("ipc unavailable"))
      .mockResolvedValue(freshHealth);
    vi.mocked(fetchRuntimeStatus).mockResolvedValue(staleHealth);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());

    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not read transcription runtime health",
      ),
    );
    expect(screen.queryByText("bundle12nov")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Install Pyannote" }),
    ).toBeDisabled();

    const refreshButtons = screen.getAllByRole("button", { name: "Refresh" });
    expect(refreshButtons[0]).toBeEnabled();
    fireEvent.click(refreshButtons[0]);

    await waitFor(() =>
      expect(
        screen.getByText("Pyannote ready on aarch64-apple-darwin."),
      ).toBeVisible(),
    );
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByText("/managed/r3/pyannote")).toBeVisible();
  });

  it("ignores a probe from an earlier Local Models visit and verifies again on re-entry", async () => {
    const firstRead = deferred<RuntimeHealth>();
    vi.mocked(fetchRuntimeHealth)
      .mockReturnValueOnce(firstRead.promise)
      .mockResolvedValueOnce(freshHealth);
    vi.mocked(fetchRuntimeStatus).mockResolvedValue(staleHealth);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());

    renderApp();
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));
    expect(screen.getByText("Checking...")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /^General/ }));

    await act(async () => {
      firstRead.resolve(staleHealth);
    });
    expect(screen.queryByText("bundle12nov")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));
    expect(screen.getByText("Checking...")).toBeVisible();
    await waitFor(() =>
      expect(
        screen.getByText("Pyannote ready on aarch64-apple-darwin."),
      ).toBeVisible(),
    );
    expect(screen.queryByText("bundle12nov")).toBeNull();
  });

  it("settles after the StrictMode double mount without a stuck loading state", async () => {
    vi.mocked(fetchRuntimeHealth).mockResolvedValue(freshHealth);
    vi.mocked(fetchRuntimeStatus).mockResolvedValue(staleHealth);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());

    renderApp({ strict: true });
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));

    await waitFor(() =>
      expect(
        screen.getByText("Pyannote ready on aarch64-apple-darwin."),
      ).toBeVisible(),
    );
    expect(screen.queryByText("Checking...")).toBeNull();
    expect(screen.queryByText("bundle12nov")).toBeNull();
  });

  it("does not revive the UI when a pending read resolves after unmount", async () => {
    const healthRead = deferred<RuntimeHealth>();
    vi.mocked(fetchRuntimeHealth).mockReturnValue(healthRead.promise);
    vi.mocked(fetchRuntimeStatus).mockResolvedValue(staleHealth);
    vi.mocked(provisioningModels).mockResolvedValue(createCatalogFixture());

    const { unmount } = renderApp();
    fireEvent.click(screen.getByRole("button", { name: /Local Models/ }));
    expect(screen.getByText("Checking...")).toBeVisible();
    unmount();

    await act(async () => {
      healthRead.resolve(staleHealth);
    });
    expect(screen.queryByText("bundle12nov")).toBeNull();
  });
});
