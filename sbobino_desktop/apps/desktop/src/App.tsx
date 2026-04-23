import React, {
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  startTransition,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  confirm as confirmDialog,
  open,
  save,
} from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  check as checkAppUpdate,
  type Update as TauriUpdate,
} from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  ArrowLeft,
  AudioLines,
  Bot,
  Check,
  Cloud,
  Clock3,
  ChevronDown,
  ChevronRight,
  Cpu,
  Database,
  FileAudio,
  FileText,
  Globe,
  HeartPulse,
  History as HistoryIcon,
  House,
  Info,
  Languages,
  List,
  ListChecks,
  ListFilter,
  MessageSquareText,
  Mic,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  Pause,
  Pencil,
  Play,
  Radio,
  Save,
  Settings2,
  Sparkles,
  Square,
  Plus,
  Search,
  Scissors,
  Trash2,
  Upload,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  analyzeArtifactEmotions,
  cancelTranscription,
  chatArtifact,
  checkUpdates,
  deleteArtifacts,
  emptyDeletedArtifacts,
  ensureTranscriptionRuntime,
  exportAppBackup,
  exportArtifact,
  fetchAiCapabilityStatus,
  fetchRealtimeStartReadiness,
  fetchTranscriptionStartPreflight,
  fetchRuntimeHealth,
  fetchSettingsSnapshot,
  getArtifact,
  hardDeleteArtifacts,
  importAppBackup,
  listDeletedArtifacts,
  listGeminiModels,
  listRecentArtifacts,
  openSettingsWindow,
  pauseRealtime,
  planPyannoteBackgroundAction,
  provisioningCancel,
  provisioningDownloadModel,
  provisioningInstallPyannote,
  provisioningInstallRuntime,
  provisioningModels,
  provisioningStart,
  provisioningStatus,
  renameArtifact,
  resetPromptTemplates,
  resumeRealtime,
  restoreArtifacts,
  saveSettings,
  saveSettingsPartial,
  summarizeArtifact,
  optimizeArtifact,
  startRealtime,
  startTranscription,
  stopRealtime,
  subscribeJobCompleted,
  subscribeJobFailed,
  subscribeJobProgress,
  subscribeTranscriptionDelta,
  subscribeProvisioningProgress,
  subscribeProvisioningStatus,
  subscribeRealtimeDelta,
  subscribeRealtimeInputLevel,
  subscribeMenuCheckUpdates,
  subscribeRealtimeSaved,
  subscribeRealtimeStatus,
  subscribeSettingsNavigate,
  subscribeSettingsUpdated,
  testPromptTemplate,
  updateArtifact,
  updateArtifactTimeline,
  writeSetupReport,
} from "./lib/tauri";
import {
  formatProvisioningAssetLabel,
  shouldOfferLocalModelsCta,
} from "./lib/provisioningUi";
import {
  canWarmStartFromSetupReport,
  type InitialSetupReport,
  type InitialSetupStepId,
  INITIAL_SETUP_REQUIRES_PYANNOTE,
  findProvisioningModelEntry,
  getRuntimeToolchainFailureMessage,
  getInitialSetupMissingModels,
  isInitialSetupComplete,
  isRuntimeToolchainReady,
  isProvisionedModelReady,
  shouldBlockMainUiDuringStartup,
} from "./lib/initialSetup";
import {
  createNativeUpdateTimeoutError,
  isNativeUpdateTimeoutError,
  resolveNativeUpdateInstallTimeoutMs,
} from "./lib/nativeUpdater";
import {
  getArtifactDiarizationUiState,
  normalizeJobFailureMessage,
} from "./lib/diarizationUi";
import { loadInitialAppBootstrapData } from "./lib/appBootstrap";
import {
  matchesPyannoteAutoActionMarker,
  readDismissedUpdateVersion,
  readLastPyannoteAutoActionMarker,
  readLastSeenAppVersion,
  readSharedUpdateSnapshot,
  shouldShowUpdateBanner,
  writeDismissedUpdateVersion,
  writeLastPyannoteAutoActionMarker,
  writeLastSeenAppVersion,
  writeSharedUpdateSnapshot,
} from "./lib/updateState";
import {
  moveSpeakerColorMapEntry,
  normalizeSpeakerColorKey,
  removeSpeakerColorMapEntry,
  resolveSpeakerColor,
  sanitizeSpeakerColorMap,
  setSpeakerColorForKey,
} from "./lib/speakerColors";
import {
  mergeSpeakerInTimeline,
  removeSpeakerFromTimeline,
  renameSpeakerInTimeline,
} from "./lib/speakerTimeline";
import {
  clampPercentage,
  formatProgressPercentageLabel,
  makeProgressVisible,
} from "./lib/progressUi";
import {
  buildQueuedTranscriptionJob,
  buildQueuedTranscriptionJobId,
  isQueuedTranscriptionJobId,
  replaceQueuedTranscriptionJob,
} from "./lib/transcriptionQueue";
import { stripAnsi } from "./lib/ansiText";
import { buildConfidenceTranscript } from "./lib/whisperConfidence";
import { useAppStore } from "./state/useAppStore";
import type {
  AiCapabilityStatus,
  AppearanceMode,
  AppSettings,
  ArtifactKind,
  EmotionAnalysisResult,
  JobProgress,
  LanguageCode,
  PromptTask,
  PromptTemplate,
  ProvisioningProgressEvent,
  ProvisioningModelCatalogEntry,
  ProvisioningStatus,
  PyannoteBackgroundActionResponse,
  PyannoteBackgroundActionTrigger,
  RealtimeDelta,
  RealtimeInputLevelEvent,
  RemoteServiceConfig,
  RemoteServiceKind,
  RuntimeHealth,
  SpeakerDiarizationSettings,
  SpeechModel,
  TimelineV2,
  TranscriptionEngine,
  TranscriptionStartPreflight,
  TranscriptArtifact,
  UpdateCheckResponse,
  WhisperOptions,
} from "./types";
import { AudioPlayer, type TrimRegion } from "./components/AudioPlayer";
import { ChatComposer } from "./components/chat/ChatComposer";
import { ChatConversation } from "./components/chat/ChatConversation";
import type {
  ChatMessageOrigin,
  ChatMessageViewModel,
  ChatPromptSuggestion,
} from "./components/chat/chatTypes";
import { buildChatClipboardText } from "./components/chat/chatUtils";
import { ConfidenceTranscript } from "./components/ConfidenceTranscript";
import { ExportSheet, type ExportRequest } from "./components/ExportSheet";
import { LiveMicrophoneWaveform } from "./components/LiveMicrophoneWaveform";
import { ModelManagerSheet } from "./components/ModelManagerSheet";
import { LoadingAnimation } from "./components/LoadingAnimation";
import { SetupMatrixIndicator } from "./components/SetupMatrixIndicator";
import {
  t,
  useTranslation,
  changeLanguage,
  supportedAppLanguages,
  type AppLanguage,
} from "./i18n";
import {
  PRIVACY_POLICY_SUMMARY,
  PRIVACY_POLICY_VERSION,
} from "./legal/privacyPolicy";
import { shouldStartWindowDrag } from "./lib/windowDrag";
import {
  aiActionsAvailable,
  buildChatArtifactPayload,
  buildEmotionAnalysisPayload,
  buildSummaryArtifactPayload,
  defaultEmotionControls,
  defaultSummaryControls,
  normalizeEmotionNarrative,
  parsePersistedEmotionAnalysis,
  shouldAutostartSummary,
} from "./lib/artifactAi";

function HighlightMatch({ text, search }: { text: string; search: string }) {
  if (!search.trim() || !text) return <>{text}</>;
  const escapedSearch = search.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const parts = text.split(new RegExp(`(${escapedSearch})`, "gi"));
  return (
    <>
      {parts.map((part, index) =>
        part.toLowerCase() === search.toLowerCase() ? (
          <mark key={index} className="highlight-text">
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  );
}

type Section =
  | "home"
  | "queue"
  | "history"
  | "deleted_history"
  | "detail"
  | "realtime";
type DetailMode = "transcript" | "segments" | "summary" | "emotion" | "chat";
type TranscriptViewMode = "optimized" | "original";
type InspectorMode = "details" | "info";
type StartupRequirementsSnapshot = {
  modelCatalog: ProvisioningModelCatalogEntry[];
  runtimeHealth: RuntimeHealth;
};
type SettingsPane =
  | "general"
  | "transcription"
  | "whisper_cpp"
  | "local_models"
  | "ai_services"
  | "prompts"
  | "advanced";
const HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY = "has_optimized_transcript";
const EMOTION_ANALYSIS_METADATA_KEY = "emotion_analysis_v1";
const EMOTION_ANALYSIS_GENERATED_AT_METADATA_KEY =
  "emotion_analysis_generated_at";
const SETTINGS_PANES: SettingsPane[] = [
  "general",
  "transcription",
  "whisper_cpp",
  "local_models",
  "ai_services",
  "prompts",
  "advanced",
];

function hasAcceptedCurrentPrivacyPolicy(
  settings: AppSettings | null | undefined,
): boolean {
  return (
    settings?.general.privacy_policy_version_accepted === PRIVACY_POLICY_VERSION
  );
}

function parseStandaloneSettingsPaneFromLocation(): SettingsPane {
  const pane = new URLSearchParams(window.location.search).get("pane");
  return SETTINGS_PANES.includes(pane as SettingsPane)
    ? (pane as SettingsPane)
    : "general";
}

function shouldPreloadSettingsDiagnostics(pane: SettingsPane): boolean {
  return pane === "transcription" || pane === "local_models";
}

function createInitialSetupReport(): InitialSetupReport {
  return {
    build_version: "",
    privacy_accepted: false,
    setup_complete: false,
    startup_gate_complete: false,
    validated_build_version: null,
    final_reason_code: null,
    final_error: null,
    last_validation_reason_code: null,
    last_validation_error: null,
    runtime_health: null,
    updated_at: new Date().toISOString(),
    steps: [
      {
        id: "privacy",
        label: "Privacy policy",
        status: "pending",
        detail: null,
        started_at: null,
        finished_at: null,
      },
      {
        id: "speech-runtime",
        label: "Speech runtime",
        status: "pending",
        detail: null,
        started_at: null,
        finished_at: null,
      },
      {
        id: "pyannote-runtime",
        label: "Speaker diarization runtime",
        status: "pending",
        detail: null,
        started_at: null,
        finished_at: null,
      },
      {
        id: "whisper-models",
        label: "Whisper models",
        status: "pending",
        detail: null,
        started_at: null,
        finished_at: null,
      },
      {
        id: "final-validation",
        label: "Final validation",
        status: "pending",
        detail: null,
        started_at: null,
        finished_at: null,
      },
    ],
  };
}

function updateInitialSetupReportStep(
  report: InitialSetupReport,
  stepId: InitialSetupStepId,
  status: InitialSetupReport["steps"][number]["status"],
  detail?: string | null,
  overrideLabel?: string,
): InitialSetupReport {
  const timestamp = new Date().toISOString();
  return {
    ...report,
    updated_at: timestamp,
    steps: report.steps.map((step) => {
      if (step.id !== stepId) {
        return step;
      }
      return {
        ...step,
        label: overrideLabel ?? step.label,
        status,
        detail: detail ?? step.detail,
        started_at:
          status === "running" && !step.started_at
            ? timestamp
            : step.started_at,
        finished_at:
          status === "completed" || status === "failed"
            ? timestamp
            : step.finished_at,
      };
    }),
  };
}

function hasPersistedOptimizedTranscript(
  artifact: TranscriptArtifact | null | undefined,
): boolean {
  if (!artifact) {
    return false;
  }

  const optimizedTranscript = artifact.optimized_transcript.trim();
  if (!optimizedTranscript) {
    return false;
  }

  const optimizedFlag =
    artifact.metadata?.[
      HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY
    ]?.trim().toLowerCase();
  if (optimizedFlag === "true") {
    return true;
  }

  if (artifact.summary.trim() || artifact.faqs.trim()) {
    return true;
  }

  return optimizedTranscript !== artifact.raw_transcript.trim();
}

type DetailSegment = {
  sourceIndex: number;
  time: string;
  line: string;
  speakerId: string | null;
  speakerLabel: string | null;
  startSeconds: number | null;
  endSeconds: number | null;
};

type KnownSpeaker = {
  id: string;
  label: string;
  color: string;
};

type PromptTestState = {
  input: string;
  output: string;
  running: boolean;
};

type TrimmedAudioValidationSnapshot = {
  path: string;
  durationSeconds: number;
  fileSizeBytes: number;
};

type TrimmedAudioDraft = TrimmedAudioValidationSnapshot & {
  parentArtifactId: string;
  title: string;
  regions: TrimRegion[];
};

type PreparedImportTrimDraft = TrimmedAudioValidationSnapshot & {
  sourcePath: string;
  title: string;
  regions: TrimRegion[];
};

type ActiveDetailContext = {
  title: string;
  artifactAudioId: string | null;
  inputPath: string | null;
  sourceArtifact: TranscriptArtifact | null;
  trimmedAudioDraft: TrimmedAudioDraft | null;
  restoreArtifactOnFailure: boolean;
};

type PendingTranscriptionContext = {
  inputPath: string;
  parentId?: string;
  title?: string;
  detailContext?: ActiveDetailContext | null;
};

type TranscriptionStartRequest = PendingTranscriptionContext & {
  trimValidationSnapshot?: TrimmedAudioValidationSnapshot | null;
};

type QueuedTranscriptionStart = TranscriptionStartRequest & {
  queueId: string;
};

const languageOptions: Array<{ value: LanguageCode; label: string }> = [
  { value: "auto", label: "Auto Detect" },
  { value: "en", label: "English" },
  { value: "it", label: "Italian" },
  { value: "fr", label: "French" },
  { value: "de", label: "German" },
  { value: "es", label: "Spanish" },
  { value: "pt", label: "Portuguese" },
  { value: "zh", label: "Chinese" },
  { value: "ja", label: "Japanese" },
];

const modelOptions: Array<{ value: SpeechModel; label: string }> = [
  { value: "tiny", label: "Tiny" },
  { value: "base", label: "Base" },
  { value: "small", label: "Small" },
  { value: "medium", label: "Medium" },
  { value: "large_turbo", label: "Large Turbo" },
];

const MIN_RETRANSCRIBE_TRIM_DURATION_SECONDS = 1.5;
const LEFT_SIDEBAR_WIDTH_STORAGE_KEY = "sbobino.layout.leftSidebarWidth";
const RIGHT_SIDEBAR_WIDTH_STORAGE_KEY = "sbobino.layout.rightSidebarWidth";
const LEFT_SIDEBAR_MIN_WIDTH = 160;
const LEFT_SIDEBAR_MAX_WIDTH = 320;
const RIGHT_SIDEBAR_MIN_WIDTH = 220;
const RIGHT_SIDEBAR_MAX_WIDTH = 420;
const AUTO_UPDATE_POLL_INTERVAL_MS = 30 * 60 * 1000;
const NATIVE_UPDATE_INSTALL_TIMEOUT_MS =
  resolveNativeUpdateInstallTimeoutMs(
    import.meta.env.VITE_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
  );
const PYANNOTE_AUTO_ACTION_FAILURE_REASON_CODES = new Set([
  "pyannote_install_incomplete",
  "pyannote_checksum_invalid",
  "pyannote_runtime_missing",
  "pyannote_model_missing",
  "pyannote_repair_required",
  "pyannote_version_mismatch",
  "pyannote_arch_mismatch",
]);

function getPyannoteBackgroundActionStatusMessage(
  action: PyannoteBackgroundActionResponse,
): string {
  switch (action.status) {
    case "install_missing":
      return t(
        "provisioning.installingPyannote",
        "Installing pyannote diarization runtime...",
      );
    case "repair_existing":
    case "migrate_assets":
      return t(
        "provisioning.repairingPyannote",
        "Repairing pyannote diarization runtime...",
      );
    case "migrate_manifest":
      return (
        action.message ||
        t("settings.pyannote.desc")
      );
    case "none":
    default:
      return t("settings.pyannote.desc");
  }
}

function guessAppleSiliconFromUA(): boolean {
  const ua = (navigator.userAgent ?? "").toLowerCase();
  const platform = (navigator.platform ?? "").toLowerCase();
  const isMac = ua.includes("macintosh") || platform.includes("mac");

  const userAgentData = (
    navigator as Navigator & {
      userAgentData?: { architecture?: string; platform?: string };
    }
  ).userAgentData;
  const uaArch = userAgentData?.architecture?.toLowerCase() ?? "";
  const uaPlatform = userAgentData?.platform?.toLowerCase() ?? "";

  if (uaPlatform.includes("mac") && uaArch.includes("arm")) {
    return true;
  }
  if (!isMac) {
    return false;
  }

  // Safari on Apple Silicon often reports "Intel" in UA, so default to true on macOS
  // and let backend runtime-health override with authoritative host architecture.
  return true;
}

const allTranscriptionEngineOptions: Array<{
  value: TranscriptionEngine;
  label: string;
}> = [{ value: "whisper_cpp", label: "Whisper C++" }];

const chunkingOptions: Array<{
  value: WhisperOptions["chunking_strategy"];
  label: string;
}> = [
  { value: "vad", label: "Voice Activity Detection" },
  { value: "none", label: "No chunking" },
];

const computeUnitOptions: Array<{
  value: WhisperOptions["audio_encoder_compute_units"];
  label: string;
}> = [
  { value: "cpu_and_neural_engine", label: "CPU + Neural Engine" },
  { value: "cpu_and_gpu", label: "CPU + GPU" },
  { value: "cpu_only", label: "CPU only" },
  { value: "all", label: "All" },
];

const promptTaskOptions: Array<{ value: PromptTask; label: string }> = [
  { value: "optimize", label: "Optimize transcript" },
  { value: "summary", label: "Summary" },
  { value: "faq", label: "FAQ" },
  { value: "emotion_analysis", label: "Emotion analysis" },
];

const fallbackGeminiModelOptions = [
  "gemini-2.5-flash-lite",
  "gemini-2.5-flash",
  "gemini-2.5-pro",
  "gemini-2.0-flash",
  "gemini-2.0-flash-lite",
  "gemini-1.5-flash",
  "gemini-1.5-pro",
];

type ServiceCatalogItem = {
  kind: RemoteServiceKind;
  label: string;
  icon: LucideIcon;
  tone:
    | "google"
    | "openai"
    | "anthropic"
    | "azure"
    | "lmstudio"
    | "ollama"
    | "openrouter"
    | "xai"
    | "huggingface"
    | "custom";
  defaultModel: string | null;
  defaultBaseUrl: string | null;
};

const serviceCatalog: ServiceCatalogItem[] = [
  {
    kind: "google",
    label: "Google",
    icon: Sparkles,
    tone: "google",
    defaultModel: "gemini-2.5-flash",
    defaultBaseUrl: "https://generativelanguage.googleapis.com/v1beta",
  },
  {
    kind: "open_ai",
    label: "OpenAI",
    icon: Bot,
    tone: "openai",
    defaultModel: "gpt-4.1-mini",
    defaultBaseUrl: "https://api.openai.com/v1",
  },
  {
    kind: "anthropic",
    label: "Anthropic",
    icon: Sparkles,
    tone: "anthropic",
    defaultModel: "claude-3-7-sonnet-latest",
    defaultBaseUrl: "https://api.anthropic.com/v1",
  },
  {
    kind: "azure",
    label: "Azure",
    icon: Cloud,
    tone: "azure",
    defaultModel: null,
    defaultBaseUrl: "https://{resource}.openai.azure.com",
  },
  {
    kind: "lm_studio",
    label: "LMStudio",
    icon: Database,
    tone: "lmstudio",
    defaultModel: null,
    defaultBaseUrl: "http://127.0.0.1:1234/v1",
  },
  {
    kind: "ollama",
    label: "Ollama",
    icon: Cpu,
    tone: "ollama",
    defaultModel: "llama3.1",
    defaultBaseUrl: "http://127.0.0.1:11434/v1",
  },
  {
    kind: "open_router",
    label: "OpenRouter",
    icon: Globe,
    tone: "openrouter",
    defaultModel: "google/gemini-2.5-flash-lite-preview:free",
    defaultBaseUrl: "https://openrouter.ai/api/v1",
  },
  {
    kind: "xai",
    label: "xAI",
    icon: Sparkles,
    tone: "xai",
    defaultModel: "grok-2-latest",
    defaultBaseUrl: "https://api.x.ai/v1",
  },
  {
    kind: "hugging_face",
    label: "Hugging Face",
    icon: Bot,
    tone: "huggingface",
    defaultModel: null,
    defaultBaseUrl: "https://router.huggingface.co/v1",
  },
  {
    kind: "custom",
    label: "Custom",
    icon: Settings2,
    tone: "custom",
    defaultModel: null,
    defaultBaseUrl: null,
  },
];

function getDefaultPromptTestInput(): string {
  return t(
    "settings.prompts.defaultTestInput",
    "This is an example of some transcribed text.",
  );
}
function getDefaultWhisperOptions(
  useAppleSiliconDefaults = guessAppleSiliconFromUA(),
): WhisperOptions {
  // On Intel Macs: use more CPU threads, greedy beam search, and CPU-only compute units
  // (no Neural Engine or CoreML acceleration available)
  const threads = useAppleSiliconDefaults
    ? 4
    : Math.max(
        4,
        Math.min(8, Math.floor((navigator.hardwareConcurrency ?? 8) / 2)),
      );

  return {
    translate_to_english: false,
    no_context: true,
    split_on_word: true,
    tinydiarize: false,
    diarize: false,
    temperature: 0,
    temperature_increment_on_fallback: 0.1,
    temperature_fallback_count: 5,
    entropy_threshold: 2.5,
    logprob_threshold: -1,
    first_token_logprob_threshold: -1.5,
    no_speech_threshold: 0.72,
    word_threshold: 0.01,
    best_of: useAppleSiliconDefaults ? 5 : 1,
    beam_size: useAppleSiliconDefaults ? 1 : 5,
    threads,
    processors: 1,
    use_prefill_prompt: true,
    use_prefill_cache: true,
    without_timestamps: false,
    word_timestamps: false,
    prompt: null,
    concurrent_worker_count: 4,
    chunking_strategy: "vad",
    audio_encoder_compute_units: useAppleSiliconDefaults
      ? "cpu_and_neural_engine"
      : "cpu_only",
    text_decoder_compute_units: useAppleSiliconDefaults
      ? "cpu_and_neural_engine"
      : "cpu_only",
  };
}

function getDefaultSpeakerDiarizationSettings(): SpeakerDiarizationSettings {
  return {
    enabled: false,
    device: "cpu",
    speaker_colors: {},
  };
}

type SettingsPaneDefinition = {
  key: SettingsPane;
  label: string;
  description: string;
  group: "General" | "Transcription" | "AI";
  icon: LucideIcon;
};

function getSettingsPaneDefinitions(): SettingsPaneDefinition[] {
  return [
    {
      key: "general",
      label: t("nav.general"),
      description: t("settings.general.desc"),
      group: "General",
      icon: House,
    },
    {
      key: "transcription",
      label: t("nav.transcription"),
      description: t("settings.transcription.desc"),
      group: "Transcription",
      icon: Mic,
    },
    {
      key: "whisper_cpp",
      label: t("nav.whisperCpp"),
      description: t("settings.whisper.desc"),
      group: "Transcription",
      icon: Settings2,
    },
    {
      key: "local_models",
      label: t("nav.localModels"),
      description: t("settings.localModels.desc"),
      group: "Transcription",
      icon: Upload,
    },
    {
      key: "advanced",
      label: t("nav.advanced"),
      description: t("settings.advanced.desc"),
      group: "Transcription",
      icon: Settings2,
    },
    {
      key: "ai_services",
      label: t("nav.aiServices"),
      description: t("settings.ai.desc"),
      group: "AI",
      icon: Sparkles,
    },
    {
      key: "prompts",
      label: t("nav.prompts"),
      description: t("settings.prompts.desc"),
      group: "AI",
      icon: MessageSquareText,
    },
  ];
}

const AI_SERVICE_NONE = "__none";
const AI_SERVICE_FOUNDATION = "__foundation";

function fileLabel(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] ?? path;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString();
}

async function withTimeout<T>(
  promise: Promise<T>,
  ms: number,
  timeoutMessage: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error(timeoutMessage)), ms);
      }),
    ]);
  } finally {
    if (timer !== null) {
      clearTimeout(timer);
    }
  }
}

function dayGroupLabel(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "recent";
  }
  const now = new Date();
  const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startDate = new Date(
    date.getFullYear(),
    date.getMonth(),
    date.getDate(),
  );
  const dayMs = 24 * 60 * 60 * 1000;
  const diffDays = Math.round(
    (startToday.getTime() - startDate.getTime()) / dayMs,
  );
  if (diffDays === 0) return t("day.today");
  if (diffDays === 1) return t("day.yesterday");
  return date.toLocaleDateString();
}

function previewSnippet(value: string, maxLength = 170): string {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (normalized.length <= maxLength) {
    return normalized;
  }
  return `${normalized.slice(0, maxLength).trimEnd()}...`;
}

function parseFiniteSeconds(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value >= 0 ? value : 0;
  }
  return null;
}

function parseNonEmptyText(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function formatTimelineTimestamp(seconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(seconds));
  const mm = String(Math.floor(totalSeconds / 60)).padStart(2, "0");
  const ss = String(totalSeconds % 60).padStart(2, "0");
  return `${mm}:${ss}`;
}

function formatTrimRangeLabel(region: TrimRegion): string {
  return `${formatTimelineTimestamp(region.startTime)}-${formatTimelineTimestamp(region.endTime)}`;
}

function buildTrimArtifactTitle(
  sourceLabel: string,
  regions: TrimRegion[],
): string {
  const sortedRegions = [...regions].sort(
    (left, right) => left.startTime - right.startTime,
  );
  const ranges = sortedRegions.map(formatTrimRangeLabel).join(", ");
  return ranges ? `${sourceLabel} - Trim ${ranges}` : `${sourceLabel} - Trim`;
}

function buildLiveSessionTitle(timestamp = new Date()): string {
  const twoDigits = (value: number): string => String(value).padStart(2, "0");
  return `live_${twoDigits(timestamp.getDate())}${twoDigits(timestamp.getMonth() + 1)}${timestamp.getFullYear()}_${twoDigits(timestamp.getHours())}${twoDigits(timestamp.getMinutes())}${twoDigits(timestamp.getSeconds())}`;
}

function buildActiveDetailContext(params: {
  artifactAudioId?: string | null;
  inputPath: string | null;
  requestedTitle?: string;
  sourceArtifact?: TranscriptArtifact | null;
  trimmedAudioDraft?: TrimmedAudioDraft | null;
  restoreArtifactOnFailure?: boolean;
}): ActiveDetailContext {
  const {
    artifactAudioId = null,
    inputPath,
    requestedTitle,
    sourceArtifact = null,
    trimmedAudioDraft = null,
    restoreArtifactOnFailure = false,
  } = params;
  const title =
    requestedTitle?.trim() ||
    trimmedAudioDraft?.title ||
    sourceArtifact?.title ||
    sourceArtifact?.source_label ||
    (inputPath
      ? fileLabel(inputPath)
      : t("detail.transcribing", "Transcribing"));

  return {
    title,
    artifactAudioId,
    inputPath,
    sourceArtifact,
    trimmedAudioDraft,
    restoreArtifactOnFailure,
  };
}

function parseTimelineV2Segments(
  timelineV2Json: string | null | undefined,
): DetailSegment[] {
  const raw = timelineV2Json?.trim();
  if (!raw) {
    return [];
  }

  try {
    const parsed = JSON.parse(raw) as TimelineV2 | null;
    const segments = parsed?.segments;
    if (!Array.isArray(segments)) {
      return [];
    }

    return segments.flatMap((segment, sourceIndex) => {
      if (!segment || typeof segment !== "object") {
        return [];
      }

      const text = parseNonEmptyText((segment as { text?: unknown }).text);
      if (!text) {
        return [];
      }

      const startSeconds = parseFiniteSeconds(
        (segment as { start_seconds?: unknown }).start_seconds,
      );
      const endSeconds = parseFiniteSeconds(
        (segment as { end_seconds?: unknown }).end_seconds,
      );
      let firstWordStart: number | null = null;
      let lastWordEnd: number | null = null;
      const words = (segment as { words?: unknown }).words;
      if (Array.isArray(words)) {
        for (const word of words) {
          if (!word || typeof word !== "object") {
            continue;
          }
          const wordStart = parseFiniteSeconds(
            (word as { start_seconds?: unknown }).start_seconds,
          );
          if (firstWordStart === null && wordStart !== null) {
            firstWordStart = wordStart;
          }
          const wordEnd = parseFiniteSeconds(
            (word as { end_seconds?: unknown }).end_seconds,
          );
          if (wordEnd !== null) {
            lastWordEnd = wordEnd;
          }
        }
      }

      const resolvedStartSeconds = startSeconds ?? firstWordStart;
      const resolvedEndSeconds = endSeconds ?? lastWordEnd;
      const anchorSeconds = resolvedStartSeconds ?? resolvedEndSeconds;
      if (anchorSeconds === null) {
        return [];
      }

      const speakerLabel =
        parseNonEmptyText(
          (segment as { speaker_label?: unknown }).speaker_label,
        ) ??
        parseNonEmptyText((segment as { speaker_id?: unknown }).speaker_id);
      const speakerId =
        parseNonEmptyText((segment as { speaker_id?: unknown }).speaker_id) ??
        (speakerLabel ? normalizeSpeakerColorKey(speakerLabel) : null);

      return [
        {
          sourceIndex,
          time: formatTimelineTimestamp(anchorSeconds),
          line: text,
          speakerId,
          speakerLabel,
          startSeconds: resolvedStartSeconds,
          endSeconds: resolvedEndSeconds,
        },
      ];
    });
  } catch {
    return [];
  }
}

function parseTimelineV2Document(
  timelineV2Json: string | null | undefined,
): { version: number; segments: Array<Record<string, unknown>> } | null {
  const raw = timelineV2Json?.trim();
  if (!raw) {
    return null;
  }
  try {
    const parsed = JSON.parse(raw) as { version?: unknown; segments?: unknown };
    if (!Array.isArray(parsed?.segments)) {
      return null;
    }
    return {
      version:
        typeof parsed.version === "number" && Number.isFinite(parsed.version)
          ? parsed.version
          : 2,
      segments: parsed.segments
        .map((segment) =>
          segment && typeof segment === "object"
            ? { ...(segment as Record<string, unknown>) }
            : null,
        )
        .filter((segment): segment is Record<string, unknown> =>
          Boolean(segment),
        ),
    };
  } catch {
    return null;
  }
}

function validateTrimmedAudioDraftForTranscription(
  draft: TrimmedAudioValidationSnapshot | null | undefined,
): string | null {
  if (!draft) {
    return null;
  }

  if (!draft.path.trim()) {
    return t(
      "detail.trimMissingPath",
      "Trimmed audio is missing. Apply the trim again before retranscribing.",
    );
  }

  if (!Number.isFinite(draft.fileSizeBytes) || draft.fileSizeBytes <= 0) {
    return t(
      "detail.trimEmpty",
      "Trimmed audio is empty. Apply the trim again before retranscribing.",
    );
  }

  if (!Number.isFinite(draft.durationSeconds) || draft.durationSeconds <= 0) {
    return t(
      "detail.trimInvalidDuration",
      "Trimmed audio duration is invalid. Apply the trim again before retranscribing.",
    );
  }

  if (draft.durationSeconds < MIN_RETRANSCRIBE_TRIM_DURATION_SECONDS) {
    return t(
      "detail.trimTooShort",
      "Trimmed audio is too short ({seconds}s). Select at least {minimum}s before retranscribing.",
      {
        seconds: draft.durationSeconds.toFixed(2),
        minimum: MIN_RETRANSCRIBE_TRIM_DURATION_SECONDS.toFixed(1),
      },
    );
  }

  return null;
}

function formatSpeakerSummary(labels: string[]): string {
  if (labels.length === 0) {
    return "";
  }

  const visible = labels.slice(0, 3);
  const extraCount = labels.length - visible.length;
  if (extraCount <= 0) {
    return visible.join(", ");
  }

  return `${visible.join(", ")} +${extraCount}`;
}

function normalizeSpeakerId(label: string): string {
  return normalizeSpeakerColorKey(label);
}

function colorWithAlpha(hexColor: string, alpha: number): string {
  const normalized = hexColor.trim();
  if (!/^#[0-9A-Fa-f]{6}$/.test(normalized)) {
    return hexColor;
  }

  const numeric = Number.parseInt(normalized.slice(1), 16);
  const red = (numeric >> 16) & 0xff;
  const green = (numeric >> 8) & 0xff;
  const blue = numeric & 0xff;
  const safeAlpha = Math.max(0, Math.min(1, alpha));
  return `rgba(${red}, ${green}, ${blue}, ${safeAlpha})`;
}

function buildSpeakerAccentStyle(
  color: string | null | undefined,
): CSSProperties | undefined {
  if (!color) {
    return undefined;
  }

  return {
    borderColor: colorWithAlpha(color, 0.36),
    background: colorWithAlpha(color, 0.12),
    color,
  };
}

function readSegmentSpeakerLabel(
  segment: Record<string, unknown>,
): string | null {
  const label = parseNonEmptyText(segment.speaker_label);
  if (label) {
    return label;
  }
  return parseNonEmptyText(segment.speaker_id);
}

function readSegmentStartSeconds(
  segment: Record<string, unknown>,
): number | null {
  const start = parseFiniteSeconds(segment.start_seconds);
  if (start !== null) {
    return start;
  }
  const words = segment.words;
  if (!Array.isArray(words)) {
    return null;
  }
  for (const word of words) {
    if (!word || typeof word !== "object") {
      continue;
    }
    const wordStart = parseFiniteSeconds(
      (word as { start_seconds?: unknown }).start_seconds,
    );
    if (wordStart !== null) {
      return wordStart;
    }
  }
  return null;
}

function readSegmentEndSeconds(
  segment: Record<string, unknown>,
): number | null {
  const end = parseFiniteSeconds(segment.end_seconds);
  if (end !== null) {
    return end;
  }
  const words = segment.words;
  if (!Array.isArray(words)) {
    return null;
  }
  for (let index = words.length - 1; index >= 0; index -= 1) {
    const word = words[index];
    if (!word || typeof word !== "object") {
      continue;
    }
    const wordEnd = parseFiniteSeconds(
      (word as { end_seconds?: unknown }).end_seconds,
    );
    if (wordEnd !== null) {
      return wordEnd;
    }
  }
  return null;
}

function pushOrReplaceQueueItem(
  items: JobProgress[],
  incoming: JobProgress,
): JobProgress[] {
  const existing = items.find((entry) => entry.job_id === incoming.job_id);
  if (!existing) {
    return [incoming, ...items];
  }
  return items.map((entry) =>
    entry.job_id === incoming.job_id ? incoming : entry,
  );
}

function formatShortDuration(seconds: number): string {
  const mm = String(Math.floor(seconds / 60));
  const ss = String(seconds % 60).padStart(2, "0");
  return `${mm}:${ss}`;
}

function parseArtifactDurationSeconds(
  artifact: TranscriptArtifact | null | undefined,
): number {
  if (!artifact) {
    return 0;
  }

  const persisted = artifact.metadata?.duration_seconds?.trim();
  if (!persisted) {
    return 0;
  }

  const seconds = Number.parseFloat(persisted);
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return 0;
  }

  return Math.round(seconds);
}

type InlineInfoHintProps = {
  label: string;
  description: string;
  side?: "top" | "left";
};

function InlineInfoHint({
  label,
  description,
  side = "top",
}: InlineInfoHintProps): JSX.Element {
  return (
    <button
      type="button"
      className={`inline-info-hint inline-info-hint--${side}`}
      aria-label={`${label}: ${description}`}
      data-tooltip={description}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      <Info size={13} />
    </button>
  );
}

function percentageFromJobProgress(
  progress: JobProgress | null | undefined,
): number {
  if (!progress) return 0;
  const currentSeconds = progress.current_seconds ?? null;
  const totalSeconds = progress.total_seconds ?? null;
  if (currentSeconds !== null && totalSeconds !== null && totalSeconds > 0) {
    return clampPercentage((currentSeconds / totalSeconds) * 100);
  }
  return clampPercentage(progress.percentage);
}

function activeJobPercentage(
  activeJobId: string | null,
  activeQueueJob: JobProgress | null,
  progress: JobProgress | null,
): number {
  if (!activeJobId) return 0;
  const queuePercentage =
    activeQueueJob?.job_id === activeJobId
      ? percentageFromJobProgress(activeQueueJob)
      : 0;
  const livePercentage =
    progress?.job_id === activeJobId ? percentageFromJobProgress(progress) : 0;
  return clampPercentage(Math.max(queuePercentage, livePercentage));
}

function TranscriptionPreview({
  text,
  fontSize,
  previewRef,
}: {
  text: string;
  fontSize: number;
  previewRef: React.RefObject<HTMLDivElement>;
}): JSX.Element {
  return (
    <div
      ref={previewRef}
      className="detail-editor transcription-preview"
      style={{ fontSize: `${fontSize}px` }}
    >
      {stripAnsi(text)}
    </div>
  );
}

function mergeTranscriptionPreview(previous: string, incoming: string): string {
  const next = incoming.trim();
  if (!next) return previous;

  const current = previous.trimEnd();
  const currentPlain = stripAnsi(current);
  const nextPlain = stripAnsi(next);

  if (!current) return next;
  if (currentPlain === nextPlain) return previous;
  if (currentPlain.includes(nextPlain)) return previous;
  if (nextPlain.startsWith(currentPlain)) return next;

  if (current === currentPlain && next === nextPlain) {
    const overlapLimit = Math.min(current.length, next.length);
    for (let size = overlapLimit; size > 0; size -= 1) {
      if (current.slice(-size) === next.slice(0, size)) {
        return `${current}${next.slice(size)}`;
      }
    }

    return `${current}\n${next}`;
  }

  return `${current}\n${next}`;
}

function setCancelPillDangerProximity(
  button: HTMLButtonElement,
  clientX: number,
): void {
  const bounds = button.getBoundingClientRect();
  if (bounds.width <= 0) {
    button.style.setProperty("--danger-proximity", "0");
    return;
  }
  const activationStart = bounds.left + bounds.width * 0.46;
  const activationWidth = Math.max(bounds.width * 0.54, 1);
  const proximity = (clientX - activationStart) / activationWidth;
  const clamped = Math.min(1, Math.max(0, proximity));
  button.style.setProperty("--danger-proximity", clamped.toFixed(3));
}

function resetCancelPillDangerProximity(button: HTMLButtonElement): void {
  button.style.setProperty("--danger-proximity", "0");
}

function formatJobStageLabel(stage: string): string {
  switch (stage) {
    case "queued":
      return t("queue.stage.queued", "Queued");
    case "preparing_audio":
      return t("queue.stage.preparingAudio", "Preparing audio");
    case "transcribing":
      return t("queue.stage.transcribing", "Transcribing");
    case "diarizing":
      return t("queue.stage.diarizing", "Assigning speakers");
    case "optimizing":
      return t("queue.stage.optimizing", "Optimizing");
    case "summarizing":
      return t("queue.stage.summarizing", "Summarizing");
    case "persisting":
      return t("queue.stage.persisting", "Saving");
    case "completed":
      return t("queue.stage.completed", "Completed");
    case "failed":
      return t("queue.stage.failed", "Failed");
    case "cancelled":
      return t("queue.stage.cancelled", "Cancelled");
    default:
      return t("queue.inProgress", "Transcription in progress");
  }
}

function formatJobMessage(stage: string): string {
  switch (stage) {
    case "queued":
      return t("queue.queuedJob", "Queued transcription job.");
    case "preparing_audio":
      return t(
        "queue.message.preparingAudio",
        "Preparing audio for transcription...",
      );
    case "transcribing":
      return t("queue.message.transcribing", "Transcribing audio...");
    case "diarizing":
      return t("queue.message.diarizing", "Assigning speakers...");
    case "optimizing":
      return t("queue.message.optimizing", "Improving transcript...");
    case "summarizing":
      return t("queue.message.summarizing", "Generating summary...");
    case "persisting":
      return t("queue.message.persisting", "Saving transcription...");
    case "completed":
      return t("queue.message.completed", "Transcription completed.");
    case "failed":
      return t("queue.message.failed", "Transcription failed.");
    case "cancelled":
      return t("queue.message.cancelled", "Transcription cancelled.");
    default:
      return t("queue.inProgress", "Transcription in progress");
  }
}

function formatProviderLabel(kind: RemoteServiceKind): string {
  const entry = serviceCatalog.find((item) => item.kind === kind);
  if (kind === "custom") {
    return t("settings.ai.customService", "Custom");
  }
  if (entry) return entry.label;
  return kind
    .replace(/_/g, " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function formatRemoteServiceLabel(
  service: Pick<RemoteServiceConfig, "kind" | "label" | "model">,
  settings?: AppSettings | null,
): string {
  if (service.kind === "google") {
    return `Google (${service.model?.trim() || settings?.ai.providers.gemini.model || "Gemini"})`;
  }

  const label = service.label?.trim();
  if (service.kind === "custom" && (!label || label === "Custom")) {
    return t("settings.ai.customService", "Custom");
  }

  return label || formatProviderLabel(service.kind);
}

function formatSpeechModelLabel(model: SpeechModel, fallback?: string): string {
  return t(`speechModel.${model}`, fallback ?? model);
}

function formatArtifactKindLabel(kind: ArtifactKind): string {
  return kind === "realtime"
    ? t("history.live", "Live")
    : t("history.file", "File");
}

function formatAppError(error: unknown): string {
  const pickMessage = (value: unknown, depth = 0): string | null => {
    if (depth > 5 || value == null) return null;
    if (typeof value === "string") {
      const trimmed = value.trim();
      return trimmed.length > 0 ? trimmed : null;
    }
    if (value instanceof Error) {
      const direct = value.message?.trim();
      if (direct) return direct;
      const cause = (value as Error & { cause?: unknown }).cause;
      return pickMessage(cause, depth + 1);
    }
    if (typeof value !== "object") return null;

    const record = value as Record<string, unknown>;
    const direct =
      typeof record.message === "string" ? record.message.trim() : "";
    if (direct.length > 0) return direct;

    return (
      pickMessage(record.error, depth + 1) ??
      pickMessage(record.cause, depth + 1) ??
      pickMessage(record.details, depth + 1) ??
      pickMessage(record.data, depth + 1)
    );
  };

  const message = pickMessage(error);
  if (message) return message;

  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

function formatAppErrorCode(error: unknown): string | null {
  const pickCode = (value: unknown, depth = 0): string | null => {
    if (depth > 5 || value == null || typeof value !== "object") return null;
    const record = value as Record<string, unknown>;
    if (typeof record.code === "string" && record.code.trim().length > 0) {
      return record.code.trim();
    }
    return (
      pickCode(record.error, depth + 1) ??
      pickCode(record.cause, depth + 1) ??
      pickCode(record.details, depth + 1) ??
      pickCode(record.data, depth + 1)
    );
  };

  return pickCode(error);
}

function formatUiError(key: string, fallback: string, error: unknown): string {
  const base = t(key, fallback);
  const detail = formatAppError(error).trim();
  if (!detail || detail === base) {
    return base;
  }
  return `${base}: ${detail}`;
}

function formatPyannoteHealthMessage(
  health: RuntimeHealth["pyannote"] | null | undefined,
): string | null {
  if (!health) {
    return null;
  }
  if (health.ready) {
    return t("settings.pyannote.readyOn", "Pyannote ready on {arch}.", {
      arch: health.arch,
    });
  }

  if (health.reason_code === "pyannote_runtime_missing") {
    return t("settings.pyannote.desc");
  }

  if (health.reason_code === "pyannote_model_missing") {
    return t("settings.pyannote.desc");
  }

  if (health.message) {
    return health.message || t("settings.pyannote.desc");
  }

  return t("settings.pyannote.desc");
}

function formatRealtimeStatusMessage(state: string): string {
  switch (state) {
    case "running":
      return t("realtime.running", "Listening live");
    case "paused":
      return t("realtime.paused", "Live paused");
    default:
      return t("realtime.idle", "Realtime idle");
  }
}

function formatRuntimeNotReadyMessage(
  runtimeHealth?: RuntimeHealth | null,
): string {
  const managedFailure = getRuntimeToolchainFailureMessage(runtimeHealth);
  if (managedFailure) {
    return managedFailure;
  }
  return t(
    "error.runtimeNotReadyDetails",
    "Transcription runtime is not ready. Check FFmpeg, Whisper CLI, and Whisper Stream in Settings > Local Models.",
  );
}

function formatTranscriptionPreflightMessage(
  preflight: TranscriptionStartPreflight,
): string {
  if (preflight.reason_code === "whispercpp_missing") {
    return t(
      "error.whisperCliNotRunnable",
      "Whisper CLI is not runnable at '{path}'. Configure Whisper CLI path in Settings > Local Models.",
      { path: preflight.whisper_cli_resolved },
    );
  }

  if (preflight.reason_code === "model_missing") {
    return t(
      "error.modelMissingAtPath",
      "Model file '{model}' was not found at '{path}'. Download models from Settings > Local Models.",
      { model: preflight.model_filename, path: preflight.model_path },
    );
  }

  if (preflight.reason_code === "ffmpeg_missing") {
    return preflight.message;
  }

  if (preflight.reason_code.startsWith("pyannote_")) {
    return (
      formatPyannoteHealthMessage(preflight.pyannote) ??
      t(
        "error.cannotStartOnMachine",
        "Transcription cannot start on this machine.",
      )
    );
  }

  return t(
    "error.cannotStartOnMachine",
    "Transcription cannot start on this machine.",
  );
}

function ProgressRing({
  percentage,
  size = 18,
}: {
  percentage: number;
  size?: number;
}): JSX.Element {
  const clamped = makeProgressVisible(percentage);
  const ringStyle = {
    width: `${size}px`,
    height: `${size}px`,
    backgroundImage: `conic-gradient(from -90deg, var(--progress-ring-fill, var(--accent)) ${clamped}%, var(--progress-ring-track, var(--line-soft)) ${clamped}% 100%)`,
  } satisfies CSSProperties;

  return (
    <span className="progress-ring" style={ringStyle} aria-hidden>
      <span className="progress-ring-core" />
    </span>
  );
}

function RollingProgressValue({ value }: { value: string }): JSX.Element {
  const [settledValue, setSettledValue] = useState(value);
  const [outgoingValue, setOutgoingValue] = useState<string | null>(null);
  const [isRolling, setIsRolling] = useState(false);

  useEffect(() => {
    if (value === settledValue) {
      return;
    }

    setOutgoingValue(settledValue);
    setSettledValue(value);
    setIsRolling(false);

    const frame = window.requestAnimationFrame(() => {
      setIsRolling(true);
    });
    const timeout = window.setTimeout(() => {
      setOutgoingValue(null);
      setIsRolling(false);
    }, 260);

    return () => {
      window.cancelAnimationFrame(frame);
      window.clearTimeout(timeout);
    };
  }, [settledValue, value]);

  return (
    <span className="transcribing-cancel-pill-value" aria-hidden>
      {outgoingValue ? (
        <>
          <span
            className={`transcribing-cancel-pill-value-slot transcribing-cancel-pill-value-slot-out${isRolling ? " is-rolling" : ""}`}
          >
            {outgoingValue}
          </span>
          <span
            className={`transcribing-cancel-pill-value-slot transcribing-cancel-pill-value-slot-in${isRolling ? " is-rolling" : ""}`}
          >
            {settledValue}
          </span>
        </>
      ) : (
        <span className="transcribing-cancel-pill-value-slot transcribing-cancel-pill-value-slot-stable">
          {settledValue}
        </span>
      )}
    </span>
  );
}

function readStoredFlag(key: string, fallback: boolean): boolean {
  if (typeof window === "undefined") {
    return fallback;
  }
  const raw = window.localStorage.getItem(key);
  if (raw === "true") return true;
  if (raw === "false") return false;
  return fallback;
}

function readStoredNumber(key: string, fallback: number): number {
  if (typeof window === "undefined") {
    return fallback;
  }
  const raw = window.localStorage.getItem(key);
  if (!raw) {
    return fallback;
  }
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function createRemoteServiceId(kind: RemoteServiceKind): string {
  if (
    typeof crypto !== "undefined" &&
    typeof crypto.randomUUID === "function"
  ) {
    return `${kind}_${crypto.randomUUID()}`;
  }
  return `${kind}_${Date.now()}_${Math.random().toString(16).slice(2, 10)}`;
}

function normalizeSettings(settings: AppSettings): AppSettings {
  const normalizedSpeakerDiarization = sanitizeSpeakerDiarizationSettings({
    ...getDefaultSpeakerDiarizationSettings(),
    ...settings.transcription.speaker_diarization,
  });
  const normalizedWhisperOptions = sanitizeWhisperOptions({
    ...getDefaultWhisperOptions(),
    ...settings.transcription.whisper_options,
  });

  const normalized: AppSettings = {
    ...settings,
    general: {
      ...settings.general,
      privacy_policy_version_accepted:
        settings.general?.privacy_policy_version_accepted ?? null,
      privacy_policy_accepted_at:
        settings.general?.privacy_policy_accepted_at ?? null,
      appearance_mode: settings.general?.appearance_mode ?? "system",
    },
    transcription: {
      ...settings.transcription,
      speaker_diarization: normalizedSpeakerDiarization,
      whisper_options: normalizedWhisperOptions,
    },
    ai: {
      ...settings.ai,
      active_remote_service_id: settings.ai.active_remote_service_id ?? null,
      providers: {
        ...settings.ai.providers,
        foundation_apple: {
          ...settings.ai.providers.foundation_apple,
        },
        gemini: {
          ...settings.ai.providers.gemini,
          has_api_key:
            settings.ai.providers.gemini.has_api_key ||
            Boolean(settings.ai.providers.gemini.api_key?.trim()),
        },
      },
      remote_services: (settings.ai.remote_services ?? []).map((service) => ({
        ...service,
        has_api_key: service.has_api_key || Boolean(service.api_key?.trim()),
        label: formatRemoteServiceLabel(service, settings),
      })),
    },
    prompts: {
      ...settings.prompts,
      bindings: {
        ...settings.prompts.bindings,
      },
      templates: settings.prompts.templates.map((template) => ({
        ...template,
      })),
    },
  };

  normalized.model = normalized.transcription.model;
  normalized.language = normalized.transcription.language;
  normalized.transcription_engine = normalized.transcription.engine;
  normalized.ai_post_processing =
    normalized.transcription.enable_ai_post_processing;
  normalized.whisper_cli_path = normalized.transcription.whisper_cli_path;
  normalized.whisperkit_cli_path = normalized.transcription.whisperkit_cli_path;
  normalized.ffmpeg_path = normalized.transcription.ffmpeg_path;
  normalized.models_dir = normalized.transcription.models_dir;

  normalized.auto_update_enabled = normalized.general.auto_update_enabled;
  normalized.auto_update_repo = normalized.general.auto_update_repo;

  normalized.gemini_model = normalized.ai.providers.gemini.model;
  normalized.gemini_api_key = normalized.ai.providers.gemini.api_key;
  normalized.gemini_api_key_present =
    normalized.ai.providers.gemini.has_api_key;

  return normalized;
}

function sanitizeSpeakerDiarizationSettings(
  settings: SpeakerDiarizationSettings,
): SpeakerDiarizationSettings {
  const device =
    settings.device === "auto" || settings.device === "mps"
      ? settings.device
      : "cpu";

  return {
    enabled: Boolean(settings.enabled),
    device,
    speaker_colors: sanitizeSpeakerColorMap(settings.speaker_colors),
  };
}

function sanitizeWhisperOptions(options: WhisperOptions): WhisperOptions {
  const clamp = (value: number, min: number, max: number): number =>
    Math.min(max, Math.max(min, value));

  return {
    translate_to_english: options.translate_to_english,
    no_context: options.no_context,
    split_on_word: options.split_on_word,
    tinydiarize: Boolean(options.tinydiarize),
    diarize: Boolean(options.diarize),
    temperature: clamp(options.temperature, 0, 1),
    temperature_increment_on_fallback: clamp(
      options.temperature_increment_on_fallback,
      0,
      2,
    ),
    temperature_fallback_count: Math.round(
      clamp(options.temperature_fallback_count, 0, 20),
    ),
    entropy_threshold: clamp(options.entropy_threshold, 0, 10),
    logprob_threshold: clamp(options.logprob_threshold, -10, 0),
    first_token_logprob_threshold: clamp(
      options.first_token_logprob_threshold,
      -10,
      0,
    ),
    no_speech_threshold: clamp(options.no_speech_threshold, 0, 1),
    word_threshold: clamp(options.word_threshold, 0, 1),
    best_of: Math.round(clamp(options.best_of, 1, 20)),
    beam_size: Math.round(clamp(options.beam_size, 1, 20)),
    threads: Math.round(clamp(options.threads, 1, 32)),
    processors: Math.round(clamp(options.processors, 1, 16)),
    use_prefill_prompt: options.use_prefill_prompt,
    use_prefill_cache: options.use_prefill_cache,
    without_timestamps: options.without_timestamps,
    word_timestamps: options.word_timestamps,
    prompt: options.prompt?.trim() ? options.prompt : null,
    concurrent_worker_count: Math.round(
      clamp(options.concurrent_worker_count, 1, 16),
    ),
    chunking_strategy: options.chunking_strategy === "none" ? "none" : "vad",
    audio_encoder_compute_units: options.audio_encoder_compute_units,
    text_decoder_compute_units: options.text_decoder_compute_units,
  };
}

type DetailCenterModeControlProps = {
  detailMode: DetailMode;
  summaryDisabled: boolean;
  emotionDisabled: boolean;
  chatDisabled: boolean;
  onSelect: (mode: "transcript" | "summary" | "emotion" | "chat") => void;
};

function DetailCenterModeControl({
  detailMode,
  summaryDisabled,
  emotionDisabled,
  chatDisabled,
  onSelect,
}: DetailCenterModeControlProps): JSX.Element {
  const { t } = useTranslation();
  return (
    <div className="segmented-control detail-mode-slider">
      <button
        className={
          detailMode === "transcript" || detailMode === "segments"
            ? "seg active"
            : "seg"
        }
        onClick={() => onSelect("transcript")}
        title={t("detail.transcript", "Transcription")}
      >
        <FileText size={15} />
      </button>
      <button
        className={detailMode === "summary" ? "seg active" : "seg"}
        onClick={() => onSelect("summary")}
        title={t("detail.summary", "AI Summary")}
        disabled={summaryDisabled}
      >
        <Sparkles size={15} />
      </button>
      <button
        className={detailMode === "emotion" ? "seg active" : "seg"}
        onClick={() => onSelect("emotion")}
        title={t("detail.emotion", "Emotion Analysis")}
        disabled={emotionDisabled}
      >
        <HeartPulse size={15} />
      </button>
      <button
        className={detailMode === "chat" ? "seg active" : "seg"}
        onClick={() => onSelect("chat")}
        title={t("detail.aiChatTitle", "AI Chat")}
        disabled={chatDisabled}
      >
        <MessageSquareText size={15} />
      </button>
    </div>
  );
}

type DetailToolbarProps = {
  leftSidebarOpen: boolean;
  rightSidebarOpen: boolean;
  rightSidebarForcedCollapsed: boolean;
  detailMode: DetailMode;
  title: string;
  hasArtifact: boolean;
  hasActiveJob: boolean;
  transcriptionProgress: number;
  onToggleSidebar: () => void;
  onBack: () => void;
  onRenameTitle?: () => void;
  onSelectMode: (mode: "transcript" | "summary" | "emotion" | "chat") => void;
  onOpenExport: () => void;
  onShowDetailsPanel: () => void;
  onHideDetailsPanel: () => void;
  onCancel: () => void;
  isImprovingText?: boolean;
  onImproveText?: () => void;
  chatDisabled?: boolean;
  optimizeDisabled?: boolean;
  optimizeDisabledTitle?: string;
  showRetranscribe?: boolean;
  isStartingTrimmedAudioRetranscription?: boolean;
  onRetranscribeTrimmedAudio?: () => void;
  realtimeControls?: {
    state: "idle" | "running" | "paused";
    isStopping: boolean;
    onPause: () => void;
    onResume: () => void;
    onStop: () => void;
  } | null;
};

function DetailToolbar({
  leftSidebarOpen,
  rightSidebarOpen,
  rightSidebarForcedCollapsed,
  detailMode,
  title,
  hasArtifact,
  hasActiveJob,
  transcriptionProgress,
  onToggleSidebar,
  onBack,
  onRenameTitle,
  onSelectMode,
  onOpenExport,
  onShowDetailsPanel,
  onHideDetailsPanel,
  onCancel,
  isImprovingText,
  onImproveText,
  chatDisabled,
  optimizeDisabled,
  optimizeDisabledTitle,
  showRetranscribe,
  isStartingTrimmedAudioRetranscription,
  onRetranscribeTrimmedAudio,
  realtimeControls,
}: DetailToolbarProps): JSX.Element {
  const { t } = useTranslation();
  const rightSidebarTitle = rightSidebarForcedCollapsed
    ? t(
        "detail.expandWindowForDetails",
        "Widen the window to show the details panel",
      )
    : rightSidebarOpen
      ? t("detail.hideDetailsPanel", "Hide details panel")
      : t("detail.showDetailsPanel", "Show details panel");
  const roundedTranscriptionProgress = Math.round(
    makeProgressVisible(transcriptionProgress),
  );
  const transcriptionProgressText = formatProgressPercentageLabel(
    transcriptionProgress,
  );
  const cancelTranscriptionTitle = `${t("detail.cancelTranscription", "Cancel transcription")} (${roundedTranscriptionProgress}%)`;
  return (
    <header
      className={`detail-toolbar ${!leftSidebarOpen ? "sidebar-closed" : ""}`}
      data-tauri-drag-region
    >
      <div className="detail-toolbar-edge detail-toolbar-edge-left">
        <button
          className={`icon-button sidebar-toggle-btn sidebar-toggle-left ${leftSidebarOpen ? "is-open" : ""}`}
          onClick={onToggleSidebar}
          title={
            leftSidebarOpen
              ? t("topbar.hideSidebar", "Hide sidebar")
              : t("topbar.showSidebar", "Show sidebar")
          }
        >
          <PanelLeftClose className="icon-close" size={16} />
          <PanelLeftOpen className="icon-open" size={16} />
        </button>
      </div>

      <div className="detail-toolbar-primary" data-tauri-drag-region>
        <button
          className="icon-button"
          onClick={onBack}
          title={t("detail.backToHistory")}
        >
          <ArrowLeft size={16} />
        </button>
        <div className="detail-title-group" data-tauri-drag-region>
          <strong className="detail-title" data-tauri-drag-region>
            {title}
          </strong>
          {hasArtifact && onRenameTitle ? (
            <button
              className="icon-button detail-title-rename-button"
              onClick={onRenameTitle}
              title={t("rename.title", "Rename transcription")}
              aria-label={t("rename.title", "Rename transcription")}
            >
              <Pencil size={14} />
            </button>
          ) : null}
        </div>
      </div>

      <div className="detail-toolbar-controls">
        <div className="detail-toolbar-center">
          <DetailCenterModeControl
            detailMode={detailMode}
            summaryDisabled={!hasArtifact}
            emotionDisabled={!hasArtifact}
            chatDisabled={chatDisabled || !hasArtifact}
            onSelect={onSelectMode}
          />
        </div>

        <div className="detail-toolbar-actions">
          {realtimeControls ? (
            <>
              <button
                className="realtime-toolbar-button realtime-toolbar-button--secondary"
                onClick={
                  realtimeControls.state === "paused"
                    ? realtimeControls.onResume
                    : realtimeControls.onPause
                }
                disabled={
                  realtimeControls.state === "idle" ||
                  realtimeControls.isStopping
                }
              >
                <span className="button-content">
                  {realtimeControls.state === "paused" ? (
                    <Play size={14} />
                  ) : (
                    <Pause size={14} />
                  )}
                  <span className="detail-action-label">
                    {realtimeControls.state === "paused"
                      ? t("realtime.resume", "Resume")
                      : t("realtime.pause", "Pause")}
                  </span>
                </span>
              </button>
              <button
                className="realtime-toolbar-button realtime-toolbar-button--primary"
                onClick={realtimeControls.onStop}
                disabled={
                  realtimeControls.state === "idle" ||
                  realtimeControls.isStopping
                }
              >
                <span className="button-content">
                  <Square size={13} />
                  <span className="detail-action-label">
                    {t("realtime.stopAndSave", "Stop & Save")}
                  </span>
                </span>
              </button>
            </>
          ) : null}
          {detailMode === "transcript" &&
            !showRetranscribe &&
            onImproveText && (
              <button
                className="optimize-hover-button"
                onClick={() => void onImproveText()}
                disabled={optimizeDisabled || isImprovingText || !hasArtifact}
                title={
                  optimizeDisabled
                    ? optimizeDisabledTitle
                    : t("detail.improveText", "Improve Text")
                }
              >
                <div className="button-content">
                  <Sparkles size={14} />
                  <span className="detail-action-label">
                    {t("detail.optimize", "Optimize")}
                  </span>
                </div>
              </button>
            )}
          {detailMode === "transcript" &&
            showRetranscribe &&
            onRetranscribeTrimmedAudio && (
              <button
                className={`retranscribe-hover-button ${isStartingTrimmedAudioRetranscription ? "is-busy" : ""}`}
                onClick={() => void onRetranscribeTrimmedAudio()}
                title={
                  isStartingTrimmedAudioRetranscription
                    ? t(
                        "detail.startingTrimmedRetranscription",
                        "Starting trimmed transcription...",
                      )
                    : t(
                        "detail.retranscribeTrimmed",
                        "Retranscribe Trimmed Audio",
                      )
                }
                disabled={isStartingTrimmedAudioRetranscription}
              >
                <div className="button-content">
                  {isStartingTrimmedAudioRetranscription ? (
                    <Clock3 size={14} />
                  ) : (
                    <Scissors size={14} />
                  )}
                  <span className="detail-action-label">
                    {isStartingTrimmedAudioRetranscription
                      ? t("home.starting", "Starting...")
                      : t("detail.retranscribe", "Retranscribe")}
                  </span>
                </div>
              </button>
            )}
          {hasArtifact ? (
            <button
              className="secondary-button export-toolbar-button"
              onClick={onOpenExport}
            >
              {t("detail.export", "Export")}
              <ChevronDown size={14} />
            </button>
          ) : null}
          {!hasArtifact && hasActiveJob ? (
            <button
              className="transcribing-cancel-pill"
              onClick={onCancel}
              onMouseMove={(event: ReactMouseEvent<HTMLButtonElement>) =>
                setCancelPillDangerProximity(event.currentTarget, event.clientX)
              }
              onMouseLeave={(event: ReactMouseEvent<HTMLButtonElement>) =>
                resetCancelPillDangerProximity(event.currentTarget)
              }
              title={cancelTranscriptionTitle}
              aria-label={cancelTranscriptionTitle}
            >
              <span className="transcribing-cancel-pill-compact" aria-hidden>
                <ProgressRing percentage={transcriptionProgress} size={20} />
              </span>
              <span className="transcribing-cancel-pill-expanded" aria-hidden>
                <RollingProgressValue value={transcriptionProgressText} />
                <span className="transcribing-cancel-pill-cancel">
                  <X size={14} />
                </span>
              </span>
            </button>
          ) : null}
        </div>
      </div>

      <div className="detail-toolbar-edge detail-toolbar-edge-right">
        <button
          className={`icon-button sidebar-toggle-btn sidebar-toggle-right ${rightSidebarOpen ? "is-open" : ""}`}
          onClick={() =>
            rightSidebarOpen ? onHideDetailsPanel() : onShowDetailsPanel()
          }
          title={rightSidebarTitle}
          disabled={rightSidebarForcedCollapsed}
        >
          <PanelRightClose className="icon-close" size={16} />
          <PanelRightOpen className="icon-open" size={16} />
        </button>
      </div>
    </header>
  );
}

type DetailInspectorHeaderProps = {
  inspectorMode: InspectorMode;
  onInspectorModeChange: (mode: InspectorMode) => void;
  onHideDetailsPanel: () => void;
};

function DetailInspectorHeader({
  inspectorMode,
  onInspectorModeChange,
  onHideDetailsPanel,
}: DetailInspectorHeaderProps): JSX.Element {
  const { t } = useTranslation();
  return (
    <header className="inspector-header">
      <div className="segmented-control inspector-view-toggle">
        <button
          className={inspectorMode === "details" ? "seg active" : "seg"}
          onClick={() => onInspectorModeChange("details")}
          title={t("detail.detailsControls")}
          aria-label={t("detail.detailsControls")}
        >
          <List size={15} />
        </button>
        <button
          className={inspectorMode === "info" ? "seg active" : "seg"}
          onClick={() => onInspectorModeChange("info")}
          title={t("detail.transcriptInfo", "Transcript information")}
          aria-label={t("detail.transcriptInfo", "Transcript information")}
        >
          <Info size={15} />
        </button>
      </div>
    </header>
  );
}

type TranscriptSegmentsTileSwitchProps = {
  detailMode: DetailMode;
  onSelectMode: (mode: "transcript" | "segments") => void;
};

function TranscriptSegmentsTileSwitch({
  detailMode,
  onSelectMode,
}: TranscriptSegmentsTileSwitchProps): JSX.Element {
  const { t } = useTranslation();
  return (
    <div className="inspector-mode-grid">
      <button
        className={
          detailMode === "transcript" ? "mode-tile active" : "mode-tile"
        }
        onClick={() => onSelectMode("transcript")}
        title={t("detail.transcript", "Transcript")}
      >
        <span className="mode-tile-icon">
          <FileText size={18} />
        </span>
        <span>{t("detail.transcript", "Transcript")}</span>
      </button>
      <button
        className={detailMode === "segments" ? "mode-tile active" : "mode-tile"}
        onClick={() => onSelectMode("segments")}
        title={t("detail.segments", "Segments")}
      >
        <span className="mode-tile-icon">
          <List size={18} />
        </span>
        <span>{t("detail.segments", "Segments")}</span>
      </button>
    </div>
  );
}

type AppProps = {
  standaloneSettingsWindow?: boolean;
  initialBootstrap?: {
    runtimeHealth: RuntimeHealth | null;
    provisioning: ProvisioningStatus | null;
    modelCatalog: ProvisioningModelCatalogEntry[] | null;
    startupRequirementsLoaded: boolean;
    setupReport: InitialSetupReport | null;
  };
};

export type GroupedArtifact = TranscriptArtifact & {
  children?: GroupedArtifact[];
};

function createProvisioningUiState(
  status: ProvisioningStatus | null | undefined,
): {
  ready: boolean;
  modelsDir: string;
  missing: string[];
  pyannote: RuntimeHealth["pyannote"] | null;
  running: boolean;
  progress: ProvisioningProgressEvent | null;
  statusMessage: string;
} {
  if (!status) {
    return {
      ready: true,
      modelsDir: "",
      missing: [],
      pyannote: null,
      running: false,
      progress: null,
      statusMessage: "",
    };
  }

  return {
    ready: status.ready,
    modelsDir: status.models_dir,
    missing: [...status.missing_models, ...status.missing_encoders],
    pyannote: status.pyannote,
    running: false,
    progress: null,
    statusMessage: "",
  };
}

export function App({
  standaloneSettingsWindow = false,
  initialBootstrap,
}: AppProps) {
  const { t, language } = useTranslation();
  const initialStandaloneSettingsPane = standaloneSettingsWindow
    ? parseStandaloneSettingsPaneFromLocation()
    : "general";
  const {
    settings,
    selectedFile,
    activeJobId,
    progress,
    error,
    artifacts,
    setSettings,
    setSelectedFile,
    setJobStarted,
    clearActiveJob,
    setProgress,
    setError,
    setArtifacts,
    prependArtifact,
    upsertArtifact,
    removeArtifacts,
  } = useAppStore();

  const preferredAppearanceMode =
    settings?.general?.appearance_mode ?? "system";

  // Apply user-selected appearance mode (system/light/dark) consistently for all windows.
  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      const nextTheme =
        preferredAppearanceMode === "system"
          ? mediaQuery.matches
            ? "dark"
            : "light"
          : preferredAppearanceMode;
      document.documentElement.dataset.theme = nextTheme;

      // Keep native macOS titlebar appearance aligned with the app theme.
      // This avoids light titlebar bleed when the app is forced to dark mode (and vice-versa).
      void (async () => {
        const hasTauriRuntime = Boolean(
          (window as unknown as { __TAURI_INTERNALS__?: unknown })
            .__TAURI_INTERNALS__,
        );
        if (!hasTauriRuntime) return;
        try {
          const { getCurrentWindow } = await import("@tauri-apps/api/window");
          await getCurrentWindow().setTheme(
            preferredAppearanceMode === "system" ? null : nextTheme,
          );
        } catch {
          // Ignore when not running in Tauri desktop runtime.
        }
      })();
    };

    applyTheme();

    if (preferredAppearanceMode !== "system") {
      return;
    }

    const onThemeChange = () => applyTheme();
    mediaQuery.addEventListener("change", onThemeChange);
    return () => mediaQuery.removeEventListener("change", onThemeChange);
  }, [preferredAppearanceMode]);

  const [section, setSection] = useState<Section>("home");
  const [settingsPane, setSettingsPane] = useState<SettingsPane>(
    initialStandaloneSettingsPane,
  );
  const [settingsQuery, setSettingsQuery] = useState("");
  const [showModelManager, setShowModelManager] = useState(false);
  const [detailMode, setDetailMode] = useState<DetailMode>("transcript");
  const [inspectorMode, setInspectorMode] = useState<InspectorMode>("details");
  const [leftSidebarOpen, setLeftSidebarOpen] = useState<boolean>(() =>
    readStoredFlag("sbobino.layout.leftSidebarOpen", true),
  );
  const [rightSidebarOpen, setRightSidebarOpen] = useState<boolean>(() =>
    readStoredFlag("sbobino.layout.rightSidebarOpen", true),
  );
  const [leftSidebarWidth, setLeftSidebarWidth] = useState<number>(() =>
    readStoredNumber(LEFT_SIDEBAR_WIDTH_STORAGE_KEY, 216),
  );
  const [rightSidebarWidth, setRightSidebarWidth] = useState<number>(() =>
    readStoredNumber(RIGHT_SIDEBAR_WIDTH_STORAGE_KEY, 280),
  );
  const [activeSidebarResize, setActiveSidebarResize] = useState<
    "left" | "right" | null
  >(null);
  const [windowWidth, setWindowWidth] = useState<number>(
    () => window.innerWidth,
  );
  const [search, setSearch] = useState("");
  const [deletedSearch, setDeletedSearch] = useState("");
  const [historyKind, setHistoryKind] = useState<"all" | ArtifactKind>("all");
  const [deletedArtifacts, setDeletedArtifacts] = useState<
    TranscriptArtifact[]
  >([]);
  const [selectedArtifactIds, setSelectedArtifactIds] = useState<string[]>([]);
  const [expandedArtifactIds, setExpandedArtifactIds] = useState<Set<string>>(
    new Set(),
  );

  useEffect(() => {
    const updateWindowWidth = () => setWindowWidth(window.innerWidth);
    window.addEventListener("resize", updateWindowWidth);
    return () => window.removeEventListener("resize", updateWindowWidth);
  }, []);

  const toggleArtifactExpansion = useCallback(
    (id: string, e: React.MouseEvent) => {
      e.stopPropagation();
      setExpandedArtifactIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) {
          next.delete(id);
        } else {
          next.add(id);
        }
        return next;
      });
    },
    [],
  );

  const [isStarting, setIsStarting] = useState(false);
  const [isSavingArtifact, setIsSavingArtifact] = useState(false);
  const [isRunningBackupAction, setIsRunningBackupAction] = useState(false);

  const [openArtifacts, setOpenArtifacts] = useState<TranscriptArtifact[]>([]);
  const [activeArtifactId, setActiveArtifactId] = useState<string | null>(null);

  const activeArtifact = useMemo(
    () => openArtifacts.find((a) => a.id === activeArtifactId) || null,
    [openArtifacts, activeArtifactId],
  );
  const rightSidebarForcedCollapsed =
    section === "detail" && windowWidth <= 900;
  const effectiveRightSidebarOpen =
    rightSidebarOpen && !rightSidebarForcedCollapsed;

  const clampLeftSidebarWidth = useCallback(
    (width: number, containerWidth: number) => {
      const maxWidth = Math.min(
        LEFT_SIDEBAR_MAX_WIDTH,
        Math.max(LEFT_SIDEBAR_MIN_WIDTH, Math.round(containerWidth * 0.32)),
      );
      return Math.min(
        maxWidth,
        Math.max(LEFT_SIDEBAR_MIN_WIDTH, Math.round(width)),
      );
    },
    [],
  );

  const clampRightSidebarWidth = useCallback(
    (width: number, containerWidth: number) => {
      const maxWidth = Math.min(
        RIGHT_SIDEBAR_MAX_WIDTH,
        Math.max(RIGHT_SIDEBAR_MIN_WIDTH, Math.round(containerWidth * 0.42)),
      );
      return Math.min(
        maxWidth,
        Math.max(RIGHT_SIDEBAR_MIN_WIDTH, Math.round(width)),
      );
    },
    [],
  );

  const setActiveArtifact = (artifact: TranscriptArtifact | null) => {
    if (!artifact) {
      // Clear all
      setOpenArtifacts([]);
      setActiveArtifactId(null);
      setDraftEmotionAnalysis(null);
      return;
    }
    setOpenArtifacts((prev) => {
      const exists = prev.some((a) => a.id === artifact.id);
      if (exists) {
        return prev.map((a) => (a.id === artifact.id ? artifact : a));
      }
      return [...prev, artifact];
    });
    setActiveArtifactId(artifact.id);
  };

  const draftTitleActiveInstance = activeArtifact?.title || "";
  const [draftTitle, setDraftTitle] = useState("");
  const [draftTranscript, setDraftTranscript] = useState("");
  const [optimizedTranscriptAvailable, setOptimizedTranscriptAvailable] =
    useState(false);
  const [transcriptViewMode, setTranscriptViewMode] =
    useState<TranscriptViewMode>("optimized");
  const [showConfidenceColors, setShowConfidenceColors] = useState(false);
  const [draftSummary, setDraftSummary] = useState("");
  const [draftFaqs, setDraftFaqs] = useState("");
  const [draftEmotionAnalysis, setDraftEmotionAnalysis] =
    useState<EmotionAnalysisResult | null>(null);
  const [emotionTimelineExpanded, setEmotionTimelineExpanded] = useState(false);
  const [showExportSheet, setShowExportSheet] = useState(false);
  const [renameTarget, setRenameTarget] = useState<TranscriptArtifact | null>(
    null,
  );
  const [renameDraft, setRenameDraft] = useState("");
  const [isRenamingArtifact, setIsRenamingArtifact] = useState(false);

  const [chatInput, setChatInput] = useState("");
  const [chatHistory, setChatHistory] = useState<ChatMessageViewModel[]>([]);
  const [copiedChatMessageId, setCopiedChatMessageId] = useState<string | null>(
    null,
  );
  const [isAskingChat, setIsAskingChat] = useState(false);
  const [isImprovingText, setIsImprovingText] = useState(false);
  const [activeJobPreviewText, setActiveJobPreviewText] = useState("");
  const [activeJobTitle, setActiveJobTitle] = useState("");
  const [focusedJobId, setFocusedJobId] = useState<string | null>(null);
  const [selectedSegmentSourceIndex, setSelectedSegmentSourceIndex] = useState<
    number | null
  >(null);
  const [speakerDraft, setSpeakerDraft] = useState("");
  const [mergeSpeakerSourceId, setMergeSpeakerSourceId] = useState("");
  const [mergeSpeakerTargetId, setMergeSpeakerTargetId] = useState("");
  const [isAssigningSpeaker, setIsAssigningSpeaker] = useState(false);
  const [propagateSpeakerAssignment, setPropagateSpeakerAssignment] =
    useState(false);
  const [segmentContextMenu, setSegmentContextMenu] = useState<{
    x: number;
    y: number;
    sourceIndex: number;
  } | null>(null);

  const [queueItems, setQueueItems] = useState<JobProgress[]>([]);
  const [queuedTranscriptionStarts, setQueuedTranscriptionStarts] = useState<
    QueuedTranscriptionStart[]
  >([]);
  const [modelCatalog, setModelCatalog] = useState<
    ProvisioningModelCatalogEntry[]
  >(() => initialBootstrap?.modelCatalog ?? []);
  const [runtimeHealth, setRuntimeHealth] = useState<RuntimeHealth | null>(
    () =>
      initialBootstrap?.runtimeHealth ??
      initialBootstrap?.setupReport?.runtime_health ??
      null,
  );
  const currentBuildVersion =
    runtimeHealth?.app_version ?? initialBootstrap?.setupReport?.build_version ?? null;
  const platformIsAppleSilicon =
    runtimeHealth?.is_apple_silicon ?? guessAppleSiliconFromUA();
  const transcriptionEngineOptions = useMemo(
    () => allTranscriptionEngineOptions,
    [],
  );
  const settingsPaneDefinitions = useMemo(
    () => getSettingsPaneDefinitions(),
    [language],
  );

  const [realtimeState, setRealtimeState] = useState<
    "idle" | "running" | "paused"
  >("idle");
  const [realtimeMessage, setRealtimeMessage] = useState(
    t("realtime.idle", "Realtime idle"),
  );
  const [realtimeFinalLines, setRealtimeFinalLines] = useState<string[]>([]);
  const [realtimePreview, setRealtimePreview] = useState("");
  const [realtimeInputLevels, setRealtimeInputLevels] = useState<number[]>([]);
  const [realtimePreviewState, setRealtimePreviewState] = useState<
    "idle" | "connecting" | "running" | "paused" | "blocked" | "unavailable"
  >("idle");
  const [realtimeSessionOpen, setRealtimeSessionOpen] = useState(false);
  const [realtimeStartedAtMs, setRealtimeStartedAtMs] = useState<number | null>(
    null,
  );
  const [realtimeElapsedSeconds, setRealtimeElapsedSeconds] = useState(0);
  const [isStoppingRealtime, setIsStoppingRealtime] = useState(false);

  const [provisioning, setProvisioning] = useState<{
    ready: boolean;
    modelsDir: string;
    missing: string[];
    pyannote: RuntimeHealth["pyannote"] | null;
    running: boolean;
    progress: ProvisioningProgressEvent | null;
    statusMessage: string;
  }>(() => createProvisioningUiState(initialBootstrap?.provisioning));
  const [startupRequirementsLoaded, setStartupRequirementsLoaded] = useState(
    standaloneSettingsWindow ||
      initialBootstrap?.startupRequirementsLoaded ||
      false,
  );
  const [startupRequirementsError, setStartupRequirementsError] = useState<
    string | null
  >(null);
  const [requiresPostUpdateValidation, setRequiresPostUpdateValidation] =
    useState(() => Boolean(initialBootstrap?.setupReport?.requires_post_update_validation));
  const [postUpdateValidationRunning, setPostUpdateValidationRunning] =
    useState(false);
  const [postUpdateValidationVisible, setPostUpdateValidationVisible] =
    useState(false);
  const [postUpdateValidationError, setPostUpdateValidationError] = useState<
    string | null
  >(() => initialBootstrap?.setupReport?.last_validation_error ?? null);
  const [postUpdateValidationAttempt, setPostUpdateValidationAttempt] =
    useState(0);
  const [initialSetupRunning, setInitialSetupRunning] = useState(false);
  const [initialSetupError, setInitialSetupError] = useState<string | null>(
    null,
  );
  const [initialSetupStepLabel, setInitialSetupStepLabel] = useState<
    string | null
  >(null);
  const [initialSetupStepDetail, setInitialSetupStepDetail] = useState<
    string | null
  >(null);
  const [acceptingPrivacyPolicy, setAcceptingPrivacyPolicy] = useState(false);

  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResponse | null>(
    () => readSharedUpdateSnapshot()?.updateInfo ?? null,
  );
  const [nativeUpdate, setNativeUpdate] = useState<TauriUpdate | null>(null);
  const [updateSource, setUpdateSource] = useState<"native" | "github" | null>(
    () => readSharedUpdateSnapshot()?.updateSource ?? null,
  );
  const [checkingUpdates, setCheckingUpdates] = useState(
    () => readSharedUpdateSnapshot()?.checking ?? false,
  );
  const [installingUpdate, setInstallingUpdate] = useState(
    () => readSharedUpdateSnapshot()?.installing ?? false,
  );
  const [updateDownloadPercent, setUpdateDownloadPercent] = useState<
    number | null
  >(() => readSharedUpdateSnapshot()?.downloadPercent ?? null);
  const [updateStatusMessage, setUpdateStatusMessage] = useState<string | null>(
    () => readSharedUpdateSnapshot()?.statusMessage ?? null,
  );
  const [dismissedUpdateVersion, setDismissedUpdateVersion] = useState<
    string | null
  >(() => readDismissedUpdateVersion());
  const [aiCapabilityStatus, setAiCapabilityStatus] =
    useState<AiCapabilityStatus | null>(null);
  const [aiServicesAcknowledged, setAiServicesAcknowledged] = useState(false);
  const [aiServiceConfigOpen, setAiServiceConfigOpen] = useState<string | null>(
    null,
  );
  const [geminiModelChoices, setGeminiModelChoices] = useState<string[]>(
    fallbackGeminiModelOptions,
  );
  const [loadingGeminiModels, setLoadingGeminiModels] = useState(false);
  const [geminiModelFetchNonce, setGeminiModelFetchNonce] = useState(0);
  const [geminiApiKeyDraft, setGeminiApiKeyDraft] = useState("");
  const [remoteServiceApiKeyDrafts, setRemoteServiceApiKeyDrafts] = useState<
    Record<string, string>
  >({});

  const [activePromptId, setActivePromptId] = useState("");
  const [promptDraft, setPromptDraft] = useState<PromptTemplate | null>(null);
  const [promptBindingTask, setPromptBindingTask] =
    useState<PromptTask>("optimize");
  const [promptTest, setPromptTest] = useState<PromptTestState>({
    input: getDefaultPromptTestInput(),
    output: "",
    running: false,
  });
  const [fontSize, setFontSize] = useState(18);
  const copiedChatResetTimerRef = useRef<number | null>(null);
  const chatMessageSerialRef = useRef(0);
  const promptTestDefaultInputRef = useRef(getDefaultPromptTestInput());
  const previousAutoUpdateEnabledRef = useRef<boolean | null>(null);
  const [summaryIncludeTimestamps, setSummaryIncludeTimestamps] = useState(
    defaultSummaryControls.includeTimestamps,
  );
  const [summaryIncludeSpeakers, setSummaryIncludeSpeakers] = useState(
    defaultSummaryControls.includeSpeakers,
  );
  const [summaryAutostart, setSummaryAutostart] = useState(false);
  const [summarySections, setSummarySections] = useState(
    defaultSummaryControls.sections,
  );
  const [summaryBulletPoints, setSummaryBulletPoints] = useState(
    defaultSummaryControls.bulletPoints,
  );
  const [summaryActionItems, setSummaryActionItems] = useState(
    defaultSummaryControls.actionItems,
  );
  const [summaryKeyPointsOnly, setSummaryKeyPointsOnly] = useState(
    defaultSummaryControls.keyPointsOnly,
  );
  const [summaryLanguage, setSummaryLanguage] = useState<LanguageCode>(
    defaultSummaryControls.language,
  );
  const [summaryCustomPrompt, setSummaryCustomPrompt] = useState("");
  const [emotionIncludeTimestamps, setEmotionIncludeTimestamps] = useState(
    defaultEmotionControls.includeTimestamps,
  );
  const [emotionIncludeSpeakers, setEmotionIncludeSpeakers] = useState(
    defaultEmotionControls.includeSpeakers,
  );
  const [emotionSpeakerDynamics, setEmotionSpeakerDynamics] = useState(
    defaultEmotionControls.speakerDynamics,
  );
  const [emotionLanguage, setEmotionLanguage] = useState<LanguageCode>(
    defaultEmotionControls.language,
  );
  const [chatIncludeTimestamps, setChatIncludeTimestamps] = useState(true);
  const [chatIncludeSpeakers, setChatIncludeSpeakers] = useState(false);
  const [isGeneratingSummary, setIsGeneratingSummary] = useState(false);
  const [isGeneratingEmotionAnalysis, setIsGeneratingEmotionAnalysis] =
    useState(false);
  const [audioDurationSeconds, setAudioDurationSeconds] = useState(0);
  const [preparedImportTrimDraft, setPreparedImportTrimDraft] =
    useState<PreparedImportTrimDraft | null>(null);
  const [trimRegions, setTrimRegions] = useState<TrimRegion[]>([]);
  const [trimmedAudioDraft, setTrimmedAudioDraft] =
    useState<TrimmedAudioDraft | null>(null);
  const [trimRetranscriptionError, setTrimRetranscriptionError] = useState<
    string | null
  >(null);
  const [activeDetailContext, setActiveDetailContext] =
    useState<ActiveDetailContext | null>(null);

  const activeJobIdRef = useRef<string | null>(activeJobId);
  const segmentElementMapRef = useRef<Map<number, HTMLElement>>(new Map());
  const windowFrameRef = useRef<HTMLElement | null>(null);
  const detailLayoutRef = useRef<HTMLDivElement | null>(null);
  const autoInitialSetupAttemptedRef = useRef(false);
  const provisioningProgressKindRef = useRef<
    ProvisioningProgressEvent["asset_kind"] | null
  >(null);
  const pyannoteProvisioningActiveRef = useRef(false);
  const postUpdateValidationTimerRef = useRef<number | null>(null);
  const initialSetupReportRef = useRef<InitialSetupReport>(
    initialBootstrap?.setupReport ?? createInitialSetupReport(),
  );
  const initialSetupStepIdRef = useRef<InitialSetupStepId | null>(null);

  const privacyPolicyAccepted = hasAcceptedCurrentPrivacyPolicy(settings);
  const warmStartEligible = canWarmStartFromSetupReport(
    privacyPolicyAccepted,
    initialBootstrap?.setupReport ?? null,
  );
  const runtimeToolchainReady = isRuntimeToolchainReady(runtimeHealth);
  const initialSetupReady = isInitialSetupComplete(
    privacyPolicyAccepted,
    runtimeHealth,
    modelCatalog,
  );
  const clearPostUpdateValidationTimer = useCallback(() => {
    if (postUpdateValidationTimerRef.current !== null) {
      window.clearTimeout(postUpdateValidationTimerRef.current);
      postUpdateValidationTimerRef.current = null;
    }
  }, []);

  const describeEmotionValence = useCallback(
    (score: number): string => {
      if (score >= 0.35) {
        return t("emotion.valencePositive", "positive");
      }
      if (score <= -0.35) {
        return t("emotion.valenceNegative", "negative");
      }
      return t("emotion.valenceNeutral", "neutral");
    },
    [t],
  );

  const emotionToneClass = useCallback(
    (score: number): "positive" | "negative" | "neutral" => {
      if (score >= 0.35) {
        return "positive";
      }
      if (score <= -0.35) {
        return "negative";
      }
      return "neutral";
    },
    [],
  );

  const describeEmotionIntensity = useCallback(
    (score: number): string => {
      if (score >= 1.5) {
        return t("emotion.intensityHigh", "high");
      }
      if (score >= 0.9) {
        return t("emotion.intensityMedium", "medium");
      }
      if (score > 0.05) {
        return t("emotion.intensityLow", "low");
      }
      return t("emotion.intensityFlat", "flat");
    },
    [t],
  );

  useEffect(() => {
    setEmotionTimelineExpanded(false);
  }, [activeArtifactId, draftEmotionAnalysis]);

  const renderEmotionTimelineCard = useCallback(
    (entry: EmotionAnalysisResult["timeline"][number]) => (
      <article
        key={`emotion-timeline-${entry.segment_index}`}
        className={`emotion-card emotion-card--${emotionToneClass(entry.valence_score)}`}
      >
        <div className="emotion-card-meta">
          <strong>
            {entry.time_label ??
              `${t("emotion.segment", "Segment")} ${entry.segment_index + 1}`}
          </strong>
          {entry.speaker_label ? (
            <span className="kind-chip">{entry.speaker_label}</span>
          ) : null}
        </div>
        <p>{entry.evidence_text}</p>
        <div className="emotion-chip-row">
          {entry.dominant_emotions.map((emotion) => (
            <span
              key={`${entry.segment_index}-${emotion}`}
              className="kind-chip"
            >
              {emotion}
            </span>
          ))}
        </div>
        <div className="emotion-metric-grid">
          <div className="emotion-metric">
            <span className="emotion-metric-label">
              {t("emotion.valenceLabel", "Tone")}
              <InlineInfoHint
                label={t("emotion.valenceLabel", "Tone")}
                description={t(
                  "emotion.valenceHelp",
                  "Tone shows the emotional direction of the segment: negative means more concern or friction, neutral means emotionally flat or balanced, positive means more confidence, relief, or enthusiasm.",
                )}
              />
            </span>
            <strong>{describeEmotionValence(entry.valence_score)}</strong>
            <small>{entry.valence_score.toFixed(1)}</small>
          </div>
          <div className="emotion-metric">
            <span className="emotion-metric-label">
              {t("emotion.intensity", "Intensity")}
              <InlineInfoHint
                label={t("emotion.intensity", "Intensity")}
                description={t(
                  "emotion.intensityHelp",
                  "Intensity shows how emotionally charged the segment is. Low means flat or procedural, high means the language carries stronger stress, urgency, relief, or excitement.",
                )}
              />
            </span>
            <strong>{describeEmotionIntensity(entry.intensity_score)}</strong>
            <small>{entry.intensity_score.toFixed(1)}</small>
          </div>
        </div>
        {entry.shift_label ? <small>{entry.shift_label}</small> : null}
        <div className="emotion-card-actions">
          <button
            className="secondary-button"
            onClick={() =>
              onJumpToEmotionSegment({
                segmentIndex: entry.segment_index,
                evidenceText: entry.evidence_text,
                timeLabel: entry.time_label,
                startSeconds: entry.start_seconds,
              })
            }
          >
            {t("emotion.jump", "Jump to segment")}
          </button>
          <button
            className="secondary-button"
            onClick={() =>
              onAskEmotionQuestion(
                `Reflect on the emotional shift around segment ${entry.segment_index + 1}. What appears to trigger ${entry.dominant_emotions.join(", ") || "the tone change"}?`,
              )
            }
          >
            {t("emotion.ask", "Ask in chat")}
          </button>
        </div>
      </article>
    ),
    [
      describeEmotionIntensity,
      describeEmotionValence,
      emotionToneClass,
      onAskEmotionQuestion,
      onJumpToEmotionSegment,
      t,
    ],
  );
  const focusedJobIdRef = useRef<string | null>(focusedJobId);
  const activeJobDeltaSequenceRef = useRef<number>(-1);
  const activeJobPreviewTextareaRef = useRef<HTMLDivElement>(null);
  const detailMainRef = useRef<HTMLElement | null>(null);
  const mainAreaRef = useRef<HTMLElement | null>(null);
  const leftSidebarRef = useRef<HTMLElement | null>(null);
  const peopleSpeakerInputRef = useRef<HTMLInputElement | null>(null);
  const failedJobMessagesRef = useRef<Map<string, string>>(new Map());
  const pendingTranscriptionContextRef = useRef<
    Map<string, PendingTranscriptionContext>
  >(new Map());
  const queuedTranscriptionSequenceRef = useRef(0);
  const startupWatchdogRef = useRef<number | null>(null);
  const settingsSaveSequenceRef = useRef(0);
  const summaryAutostartedArtifactIdsRef = useRef<Set<string>>(new Set());
  const clearStartupWatchdog = useCallback(() => {
    if (startupWatchdogRef.current !== null) {
      window.clearTimeout(startupWatchdogRef.current);
      startupWatchdogRef.current = null;
    }
  }, []);

  useEffect(() => {
    activeJobIdRef.current = activeJobId;
  }, [activeJobId]);

  useEffect(() => {
    focusedJobIdRef.current = focusedJobId;
  }, [focusedJobId]);

  useEffect(() => {
    if (!settings) {
      return;
    }

    if (
      settings.ai.providers.gemini.has_api_key &&
      !settings.ai.providers.gemini.api_key
    ) {
      setGeminiApiKeyDraft("");
    }

    setRemoteServiceApiKeyDrafts((previous) => {
      let changed = false;
      const next: Record<string, string> = {};

      for (const service of settings.ai.remote_services ?? []) {
        const existing = previous[service.id] ?? "";
        if (service.has_api_key && !service.api_key) {
          if (existing !== "") {
            changed = true;
          }
          next[service.id] = "";
        } else {
          next[service.id] = existing;
        }
      }

      if (
        !changed &&
        Object.keys(previous).length === Object.keys(next).length
      ) {
        const sameKeys = Object.keys(next).every(
          (key) => previous[key] === next[key],
        );
        if (sameKeys) {
          return previous;
        }
      }

      return next;
    });
  }, [settings]);

  useEffect(() => {
    const surfaces = [mainAreaRef.current, leftSidebarRef.current].filter(
      (surface): surface is HTMLElement => Boolean(surface),
    );
    if (surfaces.length === 0) {
      return;
    }

    const hasTauriRuntime = Boolean(
      (window as unknown as { __TAURI_INTERNALS__?: unknown })
        .__TAURI_INTERNALS__,
    );
    if (!hasTauriRuntime) {
      return;
    }

    let disposed = false;
    let startDragging: (() => Promise<void>) | null = null;

    void import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => {
        if (!disposed) {
          const currentWindow = getCurrentWindow();
          startDragging = () => currentWindow.startDragging();
        }
      })
      .catch(() => {
        startDragging = null;
      });

    const handleMouseDown = (event: MouseEvent) => {
      if (event.button !== 0 || event.defaultPrevented) {
        return;
      }
      if (!shouldStartWindowDrag(event.target, { requireExplicitArea: true })) {
        return;
      }
      if (window.getSelection()?.type === "Range") {
        return;
      }
      void startDragging?.();
    };

    surfaces.forEach((surface) =>
      surface.addEventListener("mousedown", handleMouseDown),
    );

    return () => {
      disposed = true;
      surfaces.forEach((surface) =>
        surface.removeEventListener("mousedown", handleMouseDown),
      );
    };
  }, []);

  useEffect(() => {
    if (!trimmedAudioDraft || !activeArtifact) {
      return;
    }
    if (activeArtifact.id !== trimmedAudioDraft.parentArtifactId) {
      setTrimmedAudioDraft(null);
      setTrimRegions([]);
      setTrimRetranscriptionError(null);
    }
  }, [activeArtifact, trimmedAudioDraft]);

  useEffect(() => {
    if (!focusedJobId) {
      return;
    }
    const previewContainer = activeJobPreviewTextareaRef.current;
    if (!previewContainer) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      previewContainer.scrollTop = previewContainer.scrollHeight;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [focusedJobId, activeJobPreviewText]);

  useEffect(() => {
    setSelectedSegmentSourceIndex(null);
    setSpeakerDraft("");
    setShowConfidenceColors(false);
  }, [activeArtifactId]);

  useEffect(() => {
    window.localStorage.setItem(
      "sbobino.layout.leftSidebarOpen",
      String(leftSidebarOpen),
    );
  }, [leftSidebarOpen]);

  useEffect(() => {
    window.localStorage.setItem(
      "sbobino.layout.rightSidebarOpen",
      String(rightSidebarOpen),
    );
  }, [rightSidebarOpen]);

  useEffect(() => {
    const containerWidth =
      windowFrameRef.current?.getBoundingClientRect().width ??
      window.innerWidth;
    setLeftSidebarWidth((current) =>
      clampLeftSidebarWidth(current, containerWidth),
    );
  }, [clampLeftSidebarWidth, windowWidth]);

  useEffect(() => {
    const containerWidth =
      detailLayoutRef.current?.getBoundingClientRect().width ??
      window.innerWidth;
    setRightSidebarWidth((current) =>
      clampRightSidebarWidth(current, containerWidth),
    );
  }, [clampRightSidebarWidth, windowWidth]);

  useEffect(() => {
    window.localStorage.setItem(
      LEFT_SIDEBAR_WIDTH_STORAGE_KEY,
      String(leftSidebarWidth),
    );
  }, [leftSidebarWidth]);

  useEffect(() => {
    window.localStorage.setItem(
      RIGHT_SIDEBAR_WIDTH_STORAGE_KEY,
      String(rightSidebarWidth),
    );
  }, [rightSidebarWidth]);

  useEffect(() => {
    if (!activeSidebarResize) {
      document.body.classList.remove("sidebar-resizing");
      return;
    }

    document.body.classList.add("sidebar-resizing");

    const handleMouseMove = (event: MouseEvent) => {
      if (activeSidebarResize === "left") {
        const frameRect = windowFrameRef.current?.getBoundingClientRect();
        if (!frameRect) {
          return;
        }
        setLeftSidebarWidth(
          clampLeftSidebarWidth(
            event.clientX - frameRect.left,
            frameRect.width,
          ),
        );
        return;
      }

      const detailRect = detailLayoutRef.current?.getBoundingClientRect();
      if (!detailRect) {
        return;
      }
      setRightSidebarWidth(
        clampRightSidebarWidth(
          detailRect.right - event.clientX,
          detailRect.width,
        ),
      );
    };

    const stopResize = () => {
      setActiveSidebarResize(null);
      document.body.classList.remove("sidebar-resizing");
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", stopResize);

    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", stopResize);
      document.body.classList.remove("sidebar-resizing");
    };
  }, [activeSidebarResize, clampLeftSidebarWidth, clampRightSidebarWidth]);

  useEffect(() => {
    if (!standaloneSettingsWindow) {
      return;
    }

    if (settingsPane === "local_models") {
      void refreshProvisioningModels();
      return;
    }

    if (settingsPane === "transcription") {
      void refreshRuntimeHealth();
    }
  }, [settingsPane, standaloneSettingsWindow]);

  useEffect(() => {
    if (section !== "deleted_history") {
      return;
    }

    void (async () => {
      try {
        const deletedArtifactsSnapshot = await listDeletedArtifacts({
          limit: 200,
        });
        setDeletedArtifacts(deletedArtifactsSnapshot);
      } catch (deletedError) {
        setError(
          formatUiError(
            "error.loadDeleted",
            "Could not load Recently Deleted",
            deletedError,
          ),
        );
      }
    })();
  }, [section, setError]);

  useEffect(() => {
    let disposed = false;
    let deferredUpdatesTimer: number | null = null;

    void (async () => {
      try {
        const shouldPrimeSettingsDiagnostics = standaloneSettingsWindow
          ? shouldPreloadSettingsDiagnostics(initialStandaloneSettingsPane)
          : !warmStartEligible;
        const shouldPrimeModelCatalog = standaloneSettingsWindow
          ? initialStandaloneSettingsPane === "local_models"
          : !warmStartEligible;
        const previousSeenVersion = readLastSeenAppVersion();
        const initialSettingsPromise = fetchSettingsSnapshot();
        const initialRuntimeHealthPromise = shouldPrimeSettingsDiagnostics
          ? fetchRuntimeHealth()
          : Promise.resolve(
              initialBootstrap?.setupReport?.runtime_health ?? null,
            );
        const initialModelCatalogPromise = shouldPrimeModelCatalog
          ? provisioningModels()
          : Promise.resolve(null);

        const initialSettings = await initialSettingsPromise;
        if (disposed) return;

        const normalized = normalizeSettings(initialSettings);
        setSettings(normalized);
        if (normalized.general.app_language) {
          changeLanguage(normalized.general.app_language);
        }
        if (warmStartEligible) {
          setStartupRequirementsLoaded(true);
          setStartupRequirementsError(null);
        }

        const [initialRuntimeHealthResult, initialModelCatalogResult] =
          await Promise.allSettled([
          initialRuntimeHealthPromise,
          initialModelCatalogPromise,
        ]);
        if (disposed) return;

        if (
          initialRuntimeHealthResult.status === "fulfilled" &&
          initialRuntimeHealthResult.value
        ) {
          setRuntimeHealth(initialRuntimeHealthResult.value);
          writeLastSeenAppVersion(initialRuntimeHealthResult.value.app_version);
          syncProvisioningFromRuntimeHealth(initialRuntimeHealthResult.value);
        }
        if (
          initialModelCatalogResult.status === "fulfilled" &&
          initialModelCatalogResult.value
        ) {
          setModelCatalog(initialModelCatalogResult.value);
        }
        if (warmStartEligible) {
          setStartupRequirementsLoaded(true);
          setStartupRequirementsError(null);
        } else if (
          shouldPrimeSettingsDiagnostics &&
          shouldPrimeModelCatalog &&
          initialRuntimeHealthResult.status === "fulfilled" &&
          initialModelCatalogResult.status === "fulfilled"
        ) {
          setStartupRequirementsLoaded(true);
          setStartupRequirementsError(null);
        }
        const resolvedBuildVersion =
          initialRuntimeHealthResult.status === "fulfilled" &&
          initialRuntimeHealthResult.value
            ? initialRuntimeHealthResult.value.app_version
            : currentBuildVersion;
        if (
          resolvedBuildVersion &&
          !(warmStartEligible && requiresPostUpdateValidation)
        ) {
          const pyannoteTrigger: PyannoteBackgroundActionTrigger =
            requiresPostUpdateValidation ||
            (previousSeenVersion && previousSeenVersion !== resolvedBuildVersion)
              ? "post_update"
              : "startup";
          void maybeStartPyannoteBackgroundAction(
            pyannoteTrigger,
            resolvedBuildVersion,
          );
        }

        void (async () => {
          try {
            const bootstrap = await loadInitialAppBootstrapData(
              {
                fetchSettingsSnapshot: async () => initialSettings,
                listRecentArtifacts,
                listDeletedArtifacts,
                provisioningStatus,
                provisioningModels,
                fetchRuntimeHealth,
              },
              { standaloneSettingsWindow },
            );

            if (disposed) return;

            startTransition(() => {
              if (bootstrap.activeArtifacts) {
                setArtifacts(bootstrap.activeArtifacts);
              }
              if (bootstrap.deletedArtifacts) {
                setDeletedArtifacts(bootstrap.deletedArtifacts);
              }
              if (bootstrap.provisioning) {
                setProvisioningState(bootstrap.provisioning);
              }
              if (bootstrap.modelCatalog) {
                setModelCatalog(bootstrap.modelCatalog);
              }
              if (bootstrap.runtimeHealth?.app_version) {
                writeLastSeenAppVersion(bootstrap.runtimeHealth.app_version);
              }
            });
          } catch {
            // keep app interactive even if non-essential bootstrap data fails
          }
        })();

        if (
          !standaloneSettingsWindow &&
          normalized.general.auto_update_enabled
        ) {
          deferredUpdatesTimer = window.setTimeout(() => {
            void refreshUpdates(true, () => disposed);
          }, 1200);
        }
      } catch (bootstrapError) {
        setError(
          formatUiError(
            "error.bootstrapFailed",
            "Bootstrap failed",
            bootstrapError,
          ),
        );
      }
    })();

    return () => {
      disposed = true;
      if (deferredUpdatesTimer !== null) {
        window.clearTimeout(deferredUpdatesTimer);
      }
    };
  }, [
    initialBootstrap?.setupReport,
    initialStandaloneSettingsPane,
    requiresPostUpdateValidation,
    setArtifacts,
    setError,
    setSettings,
    standaloneSettingsWindow,
    warmStartEligible,
  ]);

  useEffect(() => {
    writeSharedUpdateSnapshot({
      updateInfo,
      updateSource,
      statusMessage: updateStatusMessage,
      checking: checkingUpdates,
      installing: installingUpdate,
      downloadPercent: updateDownloadPercent,
      syncedAt: Date.now(),
    });
  }, [
    checkingUpdates,
    installingUpdate,
    updateDownloadPercent,
    updateInfo,
    updateSource,
    updateStatusMessage,
  ]);

  useEffect(() => {
    if (!currentBuildVersion) {
      return;
    }
    writeLastSeenAppVersion(currentBuildVersion);
  }, [currentBuildVersion]);

  useEffect(() => {
    const latestVersion = updateInfo?.latest_version ?? null;
    if (
      latestVersion &&
      dismissedUpdateVersion &&
      dismissedUpdateVersion !== latestVersion
    ) {
      setDismissedUpdateVersion(null);
      writeDismissedUpdateVersion(null);
    }
  }, [dismissedUpdateVersion, updateInfo?.latest_version]);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === null) {
        return;
      }

      if (event.key === "sbobino.update.dismissedVersion") {
        setDismissedUpdateVersion(readDismissedUpdateVersion());
        return;
      }

      if (event.key !== "sbobino.update.sharedState") {
        return;
      }

      const snapshot = readSharedUpdateSnapshot();
      if (!snapshot) {
        return;
      }

      setUpdateInfo(snapshot.updateInfo);
      setUpdateSource(snapshot.updateSource);
      setUpdateStatusMessage(snapshot.statusMessage);
      setCheckingUpdates(snapshot.checking);
      setInstallingUpdate(snapshot.installing);
      setUpdateDownloadPercent(snapshot.downloadPercent);

      if (snapshot.updateSource === "native" && snapshot.updateInfo?.has_update) {
        void syncNativeUpdateForVersion(snapshot.updateInfo.latest_version);
      } else if (!snapshot.updateInfo?.has_update) {
        setNativeUpdate(null);
      }
    };

    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  useEffect(() => {
    if (
      updateSource === "native" &&
      updateInfo?.has_update &&
      !nativeUpdate &&
      !installingUpdate
    ) {
      void syncNativeUpdateForVersion(updateInfo.latest_version);
    }
  }, [
    installingUpdate,
    nativeUpdate,
    updateInfo?.has_update,
    updateInfo?.latest_version,
    updateSource,
  ]);

  useEffect(() => {
    if (!standaloneSettingsWindow) {
      return;
    }

    setSettingsPane(parseStandaloneSettingsPaneFromLocation());
  }, [standaloneSettingsWindow]);

  useEffect(() => {
    if (!settings) {
      return;
    }
    if (settings.ai.active_provider !== "none") {
      return;
    }
    const hasActiveRemote =
      settings.ai.active_remote_service_id !== null &&
      settings.ai.remote_services.some(
        (service) =>
          service.id === settings.ai.active_remote_service_id &&
          service.enabled,
      );
    if (hasActiveRemote) {
      return;
    }
    const hasGoogleService = (settings.ai.remote_services ?? []).some(
      (service) => service.kind === "google",
    );
    if (
      platformIsAppleSilicon &&
      settings.ai.providers.foundation_apple.enabled
    ) {
      void patchAiSettings((current) => ({
        ...current,
        active_provider: "foundation_apple",
      }));
      return;
    }
    if (hasGoogleService && settings.ai.providers.gemini.has_api_key) {
      const googleServiceId =
        settings.ai.remote_services.find((service) => service.kind === "google")
          ?.id ?? null;
      void patchAiSettings((current) => ({
        ...current,
        active_provider: "gemini",
        active_remote_service_id: googleServiceId,
      }));
    }
  }, [
    platformIsAppleSilicon,
    settings?.ai.active_provider,
    settings?.ai.remote_services,
    settings?.ai.providers.foundation_apple.enabled,
    settings?.ai.providers.gemini.has_api_key,
  ]);

  useEffect(() => {
    if (!settings || transcriptionEngineOptions.length !== 1) {
      return;
    }

    const enforced = transcriptionEngineOptions[0].value;
    if (settings.transcription.engine === enforced) {
      return;
    }

    void patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        engine: enforced,
      },
      transcription_engine: enforced,
    }));
  }, [settings, transcriptionEngineOptions]);

  useEffect(() => {
    const googleServiceId =
      settings?.ai.remote_services?.find((service) => service.kind === "google")
        ?.id ?? null;
    if (!googleServiceId || aiServiceConfigOpen !== googleServiceId) {
      return;
    }
    const draftApiKey = geminiApiKeyDraft.trim();
    const hasStoredApiKey = Boolean(settings?.ai.providers.gemini.has_api_key);
    if (!draftApiKey && !hasStoredApiKey) {
      setGeminiModelChoices(fallbackGeminiModelOptions);
      return;
    }

    let cancelled = false;
    void (async () => {
      setLoadingGeminiModels(true);
      try {
        const models = draftApiKey
          ? await listGeminiModels(draftApiKey)
          : await listGeminiModels();

        if (!cancelled && models.length > 0) {
          const current = settings?.ai.providers.gemini.model;
          const merged = Array.from(
            new Set([
              ...(current ? [current] : []),
              ...models,
              ...fallbackGeminiModelOptions,
            ]),
          );
          setGeminiModelChoices(merged);
        }
      } catch {
        if (!cancelled) {
          setGeminiModelChoices((previous) =>
            previous.length > 0 ? previous : fallbackGeminiModelOptions,
          );
        }
      } finally {
        if (!cancelled) {
          setLoadingGeminiModels(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    aiServiceConfigOpen,
    geminiModelFetchNonce,
    geminiApiKeyDraft,
    settings?.ai.providers.gemini.has_api_key,
    settings?.ai.providers.gemini.model,
    settings?.ai.remote_services,
  ]);

  useEffect(() => {
    if (!standaloneSettingsWindow) {
      return;
    }

    let unlisten: (() => void) | undefined;
    void (async () => {
      unlisten = await subscribeSettingsNavigate((pane) => {
        if (SETTINGS_PANES.includes(pane as SettingsPane)) {
          setSettingsPane(pane as SettingsPane);
        }
      });
    })();

    return () => {
      unlisten?.();
    };
  }, [standaloneSettingsWindow]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void (async () => {
      unlisten = await subscribeSettingsUpdated((incoming) => {
        const normalized = normalizeSettings(incoming);
        setSettings(normalized);
        if (normalized.general.app_language) {
          changeLanguage(normalized.general.app_language);
        }
      });
    })();

    return () => {
      unlisten?.();
    };
  }, [setSettings]);

  useEffect(() => {
    const enabled = Boolean(settings?.general.auto_update_enabled);
    if (previousAutoUpdateEnabledRef.current === false && enabled) {
      void refreshUpdates(true);
    }
    previousAutoUpdateEnabledRef.current = enabled;
  }, [settings?.general.auto_update_enabled]);

  useEffect(() => {
    if (!settings?.general.auto_update_enabled) {
      return;
    }

    const triggerRefresh = () => {
      if (document.visibilityState !== "visible") {
        return;
      }
      void refreshUpdates(true);
    };

    const intervalId = window.setInterval(
      triggerRefresh,
      AUTO_UPDATE_POLL_INTERVAL_MS,
    );
    window.addEventListener("focus", triggerRefresh);
    document.addEventListener("visibilitychange", triggerRefresh);

    return () => {
      window.clearInterval(intervalId);
      window.removeEventListener("focus", triggerRefresh);
      document.removeEventListener("visibilitychange", triggerRefresh);
    };
  }, [settings?.general.auto_update_enabled]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        unlisten = await subscribeMenuCheckUpdates(() => {
          void onRefreshUpdates();
        });
      } catch {
        // menu listener is best-effort in non-native environments
      }
    })();

    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!settings) {
      return;
    }

    let cancelled = false;
    void (async () => {
      try {
        const status = await fetchAiCapabilityStatus();
        if (!cancelled) {
          setAiCapabilityStatus(status);
        }
      } catch {
        if (!cancelled) {
          setAiCapabilityStatus(null);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [settings]);

  useEffect(() => {
    if (!settings || standaloneSettingsWindow) {
      return;
    }

    if (warmStartEligible) {
      setStartupRequirementsLoaded(true);
      setStartupRequirementsError(null);
      return;
    }

    let cancelled = false;
    const runDiagnostics = () =>
      void (async () => {
        try {
          await loadStartupRequirements();
        } catch (startupError) {
          if (!cancelled) {
            const formatted = formatUiError(
              "error.startupRequirementsFailed",
              "Could not prepare local runtime requirements",
              startupError,
            );
            setStartupRequirementsLoaded(false);
            setStartupRequirementsError(formatted);
          }
        }
      })();

    runDiagnostics();

    return () => {
      cancelled = true;
    };
  }, [setError, settings, standaloneSettingsWindow, warmStartEligible]);

  useEffect(() => {
    if (
      standaloneSettingsWindow ||
      !settings ||
      !privacyPolicyAccepted ||
      !warmStartEligible ||
      !requiresPostUpdateValidation
    ) {
      clearPostUpdateValidationTimer();
      setPostUpdateValidationRunning(false);
      setPostUpdateValidationVisible(false);
      return;
    }

    let cancelled = false;
    clearPostUpdateValidationTimer();
    setPostUpdateValidationRunning(true);
    setPostUpdateValidationVisible(false);
    setPostUpdateValidationError(null);
    postUpdateValidationTimerRef.current = window.setTimeout(() => {
      if (!cancelled) {
        setPostUpdateValidationVisible(true);
      }
    }, 1200);

    void (async () => {
      let snapshot: StartupRequirementsSnapshot | null = null;
      try {
        snapshot = await loadStartupRequirements({ includeModelCatalog: false });
        if (cancelled) {
          return;
        }

        if (!snapshot.runtimeHealth.setup_complete) {
          const failureMessage = formatPostUpdateValidationFailure(snapshot);
          await persistInitialSetupReport((current) => ({
            ...current,
            startup_gate_complete:
              current.startup_gate_complete || current.setup_complete,
            last_validation_reason_code: inferPostUpdateValidationReasonCode(snapshot),
            last_validation_error: failureMessage,
            runtime_health: snapshot?.runtimeHealth ?? current.runtime_health,
            updated_at: new Date().toISOString(),
          }));
          setPostUpdateValidationError(failureMessage);
          setPostUpdateValidationVisible(true);
          return;
        }

        const validatedRuntimeHealth = snapshot.runtimeHealth;
        await persistInitialSetupReport((current) => ({
          ...current,
          setup_complete:
            current.setup_complete || validatedRuntimeHealth.setup_complete,
          startup_gate_complete: true,
          validated_build_version: validatedRuntimeHealth.app_version,
          last_validation_reason_code: inferPostUpdateValidationReasonCode(snapshot),
          last_validation_error: null,
          runtime_health: validatedRuntimeHealth,
          updated_at: new Date().toISOString(),
        }));
        setRequiresPostUpdateValidation(false);
        setPostUpdateValidationVisible(false);
        setPostUpdateValidationError(null);
        void maybeStartPyannoteBackgroundAction(
          "post_update",
          validatedRuntimeHealth.app_version,
        );
      } catch (validationError) {
        if (cancelled) {
          return;
        }
        const formatted = formatUiError(
          "error.postUpdateValidationFailed",
          "Could not finish post-update runtime checks",
          validationError,
        );
        await persistInitialSetupReport((current) => ({
          ...current,
          startup_gate_complete:
            current.startup_gate_complete || current.setup_complete,
          last_validation_reason_code: inferPostUpdateValidationReasonCode(snapshot),
          last_validation_error: formatted,
          runtime_health: snapshot?.runtimeHealth ?? current.runtime_health,
          updated_at: new Date().toISOString(),
        }));
        setPostUpdateValidationError(formatted);
        setPostUpdateValidationVisible(true);
      } finally {
        if (!cancelled) {
          clearPostUpdateValidationTimer();
          setPostUpdateValidationRunning(false);
        }
      }
    })();

    return () => {
      cancelled = true;
      clearPostUpdateValidationTimer();
    };
  }, [
    clearPostUpdateValidationTimer,
    postUpdateValidationAttempt,
    privacyPolicyAccepted,
    requiresPostUpdateValidation,
    settings,
    standaloneSettingsWindow,
    warmStartEligible,
  ]);

  useEffect(() => {
    if (standaloneSettingsWindow || !settings || !privacyPolicyAccepted) {
      return;
    }
    if (warmStartEligible) {
      return;
    }
    if (
      !startupRequirementsLoaded ||
      initialSetupRunning ||
      initialSetupReady
    ) {
      return;
    }
    if (autoInitialSetupAttemptedRef.current) {
      return;
    }

    autoInitialSetupAttemptedRef.current = true;
    void beginInitialSetup();
  }, [
    initialSetupReady,
    initialSetupRunning,
    privacyPolicyAccepted,
    settings,
    standaloneSettingsWindow,
    startupRequirementsLoaded,
    warmStartEligible,
  ]);

  useEffect(() => {
    const localizedDefaultInput = getDefaultPromptTestInput();
    setPromptTest((current) => {
      const previousDefaultInput = promptTestDefaultInputRef.current;
      promptTestDefaultInputRef.current = localizedDefaultInput;
      if (
        current.input.trim().length > 0 &&
        current.input !== previousDefaultInput
      ) {
        return current;
      }
      return {
        ...current,
        input: localizedDefaultInput,
      };
    });
  }, [language]);

  const resetPreparedImportAudio = useCallback(() => {
    setPreparedImportTrimDraft(null);
  }, []);

  const primeSelectedFileForHome = useCallback(
    (filePath: string) => {
      resetPreparedImportAudio();
      setSelectedFile(filePath);
      setSection("home");
      setError(null);
    },
    [resetPreparedImportAudio, setError, setSelectedFile],
  );

  // Tauri native drag-and-drop: load dropped audio files into Home for review
  useEffect(() => {
    const audioExtensions = [
      "wav",
      "mp3",
      "m4a",
      "flac",
      "ogg",
      "aac",
      "opus",
      "webm",
      "mp4",
      "mov",
      "mkv",
      "wma",
      "aiff",
    ];
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        unlisten = await getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === "drop") {
            const paths = event.payload.paths;
            if (!paths || paths.length === 0) return;

            const audioFile = paths.find((p) => {
              const ext = p.split(".").pop()?.toLowerCase() ?? "";
              return audioExtensions.includes(ext);
            });

            if (audioFile) {
              primeSelectedFileForHome(audioFile);
            }
          }
        });
      } catch {
        // Drag-drop not available (e.g. running in browser)
      }
    })();

    return () => {
      unlisten?.();
    };
  }, [primeSelectedFileForHome]);

  useEffect(() => {
    let unmounted = false;
    let unsubProgress: (() => void) | undefined;
    let unsubCompleted: (() => void) | undefined;
    let unsubFailed: (() => void) | undefined;
    let unsubTranscriptionDelta: (() => void) | undefined;
    let unsubRealtimeDelta: (() => void) | undefined;
    let unsubRealtimeInput: (() => void) | undefined;
    let unsubRealtimeStatus: (() => void) | undefined;
    let unsubRealtimeSaved: (() => void) | undefined;
    let unsubProvisioningProgress: (() => void) | undefined;
    let unsubProvisioningStatus: (() => void) | undefined;

    void (async () => {
      const uProgress = await subscribeJobProgress((event) => {
        const resolvedMessage = normalizeJobFailureMessage(
          event.message,
          formatJobMessage(event.stage),
        );
        const queueEvent = {
          ...event,
          message: resolvedMessage,
        };
        setQueueItems((previous) =>
          pushOrReplaceQueueItem(previous, queueEvent),
        );
        if (event.job_id === activeJobIdRef.current) {
          clearStartupWatchdog();
          setProgress(queueEvent);
          if (event.stage === "cancelled" || event.stage === "failed") {
            const wasFocused = focusedJobIdRef.current === event.job_id;
            clearStartupWatchdog();
            const failedContext = pendingTranscriptionContextRef.current.get(
              event.job_id,
            );
            pendingTranscriptionContextRef.current.delete(event.job_id);
            clearActiveJob();
            activeJobIdRef.current = null;
            setActiveJobTitle("");
            if (wasFocused) {
              setFocusedJobId(null);
              setActiveJobPreviewText("");
              activeJobDeltaSequenceRef.current = -1;
              setActiveDetailContext(null);
            }
            if (event.stage === "failed" && wasFocused) {
              presentTranscriptionFailure(
                resolvedMessage,
                failedContext?.detailContext,
              );
            } else if (event.stage === "failed") {
              setError(resolvedMessage);
            } else if (
              wasFocused &&
              !restoreDetailAfterFailedTranscription(
                failedContext?.detailContext,
              )
            ) {
              setSection("home");
            }
          }
        }
      });
      if (unmounted) {
        uProgress();
      } else {
        unsubProgress = uProgress;
      }

      const uCompleted = await subscribeJobCompleted((artifact) => {
        failedJobMessagesRef.current.delete(artifact.job_id);
        const pendingContext = pendingTranscriptionContextRef.current.get(
          artifact.job_id,
        );
        pendingTranscriptionContextRef.current.delete(artifact.job_id);
        const wasRunning = artifact.job_id === activeJobIdRef.current;
        const wasFocused = artifact.job_id === focusedJobIdRef.current;
        const hydratedArtifact = pendingContext
          ? {
              ...artifact,
              title: pendingContext.title?.trim()
                ? pendingContext.title
                : artifact.title,
              source_label: pendingContext.inputPath
                ? fileLabel(pendingContext.inputPath)
                : artifact.source_label,
              parent_artifact_id:
                pendingContext.parentId ?? artifact.parent_artifact_id,
            }
          : artifact;

        prependArtifact(hydratedArtifact);
        setQueueItems((previous) =>
          previous.filter((entry) => entry.job_id !== artifact.job_id),
        );

        if (wasRunning) {
          clearStartupWatchdog();
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobTitle("");
        }

        if (wasFocused) {
          setFocusedJobId(null);
          setActiveJobPreviewText("");
          activeJobDeltaSequenceRef.current = -1;
          setActiveDetailContext(null);
          hydrateDetail(hydratedArtifact);
          setSection("detail");
          setError(null);
        }
      });
      if (unmounted) {
        uCompleted();
      } else {
        unsubCompleted = uCompleted;
      }

      const uFailed = await subscribeJobFailed((payload) => {
        const resolvedFailureMessage = normalizeJobFailureMessage(
          payload.message,
          formatJobMessage("failed"),
        );
        failedJobMessagesRef.current.set(
          payload.job_id,
          resolvedFailureMessage,
        );
        const failedContext = pendingTranscriptionContextRef.current.get(
          payload.job_id,
        );
        pendingTranscriptionContextRef.current.delete(payload.job_id);
        const wasRunning = payload.job_id === activeJobIdRef.current;
        const wasFocused = payload.job_id === focusedJobIdRef.current;
        setQueueItems((previous) =>
          previous.map((entry) =>
            entry.job_id === payload.job_id
              ? {
                  ...entry,
                  stage: "failed",
                  message: resolvedFailureMessage,
                  percentage: 100,
                }
              : entry,
          ),
        );

        if (wasRunning) {
          clearStartupWatchdog();
          failedJobMessagesRef.current.delete(payload.job_id);
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobTitle("");
        }

        if (wasFocused) {
          setFocusedJobId(null);
          setActiveJobPreviewText("");
          activeJobDeltaSequenceRef.current = -1;
          setActiveDetailContext(null);
          presentTranscriptionFailure(
            resolvedFailureMessage,
            failedContext?.detailContext,
          );
        } else if (wasRunning) {
          setError(resolvedFailureMessage);
        }
      });
      if (unmounted) {
        uFailed();
      } else {
        unsubFailed = uFailed;
      }

      const uTranscriptionDelta = await subscribeTranscriptionDelta((delta) => {
        if (delta.job_id !== focusedJobIdRef.current) {
          return;
        }
        if (delta.sequence <= activeJobDeltaSequenceRef.current) {
          return;
        }
        activeJobDeltaSequenceRef.current = delta.sequence;

        setActiveJobPreviewText((previous) => {
          const next = delta.text ?? "";
          if (!next.trim()) return previous;

          // Some engines emit snapshot-style updates, others append finalized lines.
          if (delta.mode === "replace") {
            return next;
          }

          return mergeTranscriptionPreview(previous, next);
        });
      });
      if (unmounted) {
        uTranscriptionDelta();
      } else {
        unsubTranscriptionDelta = uTranscriptionDelta;
      }

      const uRealtimeDelta = await subscribeRealtimeDelta(
        (delta: RealtimeDelta) => {
          if (delta.kind === "append_final") {
            setRealtimeFinalLines((previous) => [...previous, delta.text]);
            setRealtimePreview("");
          }

          if (delta.kind === "replace_final") {
            setRealtimeFinalLines((previous) => {
              if (previous.length === 0) {
                return [delta.text];
              }
              const next = [...previous];
              next[next.length - 1] = delta.text;
              return next;
            });
            setRealtimePreview("");
          }

          if (delta.kind === "update_preview") {
            setRealtimePreview(delta.text);
          }
        },
      );
      if (unmounted) {
        uRealtimeDelta();
      } else {
        unsubRealtimeDelta = uRealtimeDelta;
      }

      const uRealtimeInput = await subscribeRealtimeInputLevel(
        (event: RealtimeInputLevelEvent) => {
          if (event.state === "running") {
            setRealtimePreviewState("running");
            setRealtimeInputLevels((previous) => {
              const next = [...previous, Math.max(0, Math.min(1, event.level ?? 0))];
              return next.length > 160 ? next.slice(next.length - 160) : next;
            });
            return;
          }

          if (event.state === "paused") {
            setRealtimePreviewState("paused");
            return;
          }

          if (event.state === "connecting") {
            setRealtimePreviewState("connecting");
            return;
          }

          if (event.state === "blocked" || event.state === "unavailable") {
            setRealtimePreviewState(event.state);
            setRealtimeInputLevels([]);
            return;
          }

          setRealtimePreviewState("idle");
          setRealtimeInputLevels([]);
        },
      );
      if (unmounted) {
        uRealtimeInput();
      } else {
        unsubRealtimeInput = uRealtimeInput;
      }

      const uRealtimeStatus = await subscribeRealtimeStatus((event) => {
        setRealtimeMessage(formatRealtimeStatusMessage(event.state));
        if (event.state === "running") {
          setRealtimeState("running");
        } else if (event.state === "paused") {
          setRealtimeState("paused");
          setRealtimePreviewState("paused");
        } else {
          setRealtimeState("idle");
          setRealtimePreviewState("idle");
        }
      });
      if (unmounted) {
        uRealtimeStatus();
      } else {
        unsubRealtimeStatus = uRealtimeStatus;
      }

      const uRealtimeSaved = await subscribeRealtimeSaved((artifact) => {
        prependArtifact(artifact);
      });
      if (unmounted) {
        uRealtimeSaved();
      } else {
        unsubRealtimeSaved = uRealtimeSaved;
      }

      const uProvisioningProgress = await subscribeProvisioningProgress(
        (event) => {
          provisioningProgressKindRef.current = event.asset_kind;
          if (
            event.asset_kind === "pyannote_runtime" ||
            event.asset_kind === "pyannote_model"
          ) {
            pyannoteProvisioningActiveRef.current = true;
          }
          setProvisioning((previous) => ({
            ...previous,
            running: true,
            progress: event,
            statusMessage: `${formatProvisioningAssetLabel(event)} (${event.current}/${event.total})`,
          }));
        },
      );
      if (unmounted) {
        uProvisioningProgress();
      } else {
        unsubProvisioningProgress = uProvisioningProgress;
      }

      const uProvisioningStatus = await subscribeProvisioningStatus((event) => {
        const pyannoteProvisioningFailed =
          event.state !== "completed" &&
          event.state !== "cancelled" &&
          PYANNOTE_AUTO_ACTION_FAILURE_REASON_CODES.has(
            event.reason_code ?? "",
          );
        const wasPyannoteProvisioning =
          pyannoteProvisioningActiveRef.current ||
          provisioningProgressKindRef.current === "pyannote_runtime" ||
          provisioningProgressKindRef.current === "pyannote_model";
        if (pyannoteProvisioningFailed || (wasPyannoteProvisioning && event.state !== "running")) {
          writeLastPyannoteAutoActionMarker(null);
        }
        if (wasPyannoteProvisioning && event.state !== "running") {
          pyannoteProvisioningActiveRef.current = false;
        }

        const localizedStatusMessage =
          event.state === "completed"
            ? provisioningProgressKindRef.current === "speech_runtime"
              ? t(
                  "provisioning.runtimeReady",
                  "Local transcription runtime is ready",
                )
              : t("settings.localModels.readyMessage", "Local models are ready")
            : event.state === "cancelled"
              ? t("provisioning.cancelled", "Provisioning cancelled")
              : event.reason_code === "pyannote_install_incomplete" ||
                  event.reason_code === "pyannote_checksum_invalid"
                ? event.message || t("settings.pyannote.desc")
                : event.reason_code === "pyannote_runtime_missing" ||
                    event.reason_code === "pyannote_model_missing" ||
                    event.reason_code === "pyannote_repair_required" ||
                    event.reason_code === "pyannote_version_mismatch" ||
                    event.reason_code === "pyannote_arch_mismatch"
                  ? t("settings.pyannote.desc")
                  : t("error.provisioningFailed", "Provisioning failed");
        setProvisioning((previous) => ({
          ...previous,
          running: false,
          statusMessage: localizedStatusMessage,
          progress: event.state === "completed" ? previous.progress : null,
          ready: event.state === "completed" ? true : previous.ready,
        }));
        if (event.state !== "running") {
          provisioningProgressKindRef.current = null;
        }

        if (event.state === "completed") {
          void refreshProvisioningModels();
        }
      });
      if (unmounted) {
        uProvisioningStatus();
      } else {
        unsubProvisioningStatus = uProvisioningStatus;
      }
    })();

    return () => {
      unmounted = true;
      unsubProgress?.();
      unsubCompleted?.();
      unsubFailed?.();
      unsubTranscriptionDelta?.();
      unsubRealtimeDelta?.();
      unsubRealtimeInput?.();
      unsubRealtimeStatus?.();
      unsubRealtimeSaved?.();
      unsubProvisioningProgress?.();
      unsubProvisioningStatus?.();
      clearStartupWatchdog();
    };
  }, [
    clearActiveJob,
    clearStartupWatchdog,
    prependArtifact,
    setError,
    setProgress,
  ]);

  useEffect(() => {
    if (!settings) return;

    const templates = settings.prompts.templates;
    if (templates.length === 0) {
      setActivePromptId("");
      setPromptDraft(null);
      return;
    }

    const selected =
      templates.find((item) => item.id === activePromptId) ?? templates[0];
    if (selected.id !== activePromptId) {
      setActivePromptId(selected.id);
    }

    setPromptDraft((previous) => {
      if (!previous || previous.id !== selected.id) {
        return { ...selected };
      }

      const hasServerChanges =
        previous.updated_at !== selected.updated_at ||
        previous.name !== selected.name ||
        previous.body !== selected.body ||
        previous.icon !== selected.icon ||
        previous.category !== selected.category;

      return hasServerChanges ? { ...selected } : previous;
    });
  }, [settings, activePromptId]);

  const filteredArtifacts = useMemo(() => {
    const needle = search.trim().toLowerCase();

    return artifacts.filter((artifact) => {
      if (historyKind !== "all" && artifact.kind !== historyKind) {
        return false;
      }

      if (!needle) {
        return true;
      }

      return (
        artifact.title?.toLowerCase().includes(needle) ||
        artifact.source_label?.toLowerCase().includes(needle) ||
        artifact.optimized_transcript?.toLowerCase().includes(needle) ||
        artifact.raw_transcript?.toLowerCase().includes(needle)
      );
    });
  }, [artifacts, historyKind, search]);

  const filteredDeletedArtifacts = useMemo(() => {
    const needle = deletedSearch.trim().toLowerCase();
    return deletedArtifacts.filter((artifact) => {
      if (!needle) return true;
      return (
        artifact.title?.toLowerCase().includes(needle) ||
        artifact.source_label?.toLowerCase().includes(needle) ||
        artifact.optimized_transcript?.toLowerCase().includes(needle) ||
        artifact.raw_transcript?.toLowerCase().includes(needle)
      );
    });
  }, [deletedArtifacts, deletedSearch]);

  const buildArtifactTree = useCallback(
    (items: TranscriptArtifact[]): GroupedArtifact[] => {
      const map = new Map<string, GroupedArtifact>();
      items.forEach((item) => {
        map.set(item.id, { ...item, children: [] });
      });

      const roots: GroupedArtifact[] = [];
      items.forEach((item) => {
        const node = map.get(item.id)!;
        const parentArtifactId =
          item.parent_artifact_id ?? item.metadata?.parent_id ?? null;
        if (parentArtifactId && map.has(parentArtifactId)) {
          map.get(parentArtifactId)!.children!.push(node);
        } else {
          roots.push(node);
        }
      });

      return roots;
    },
    [],
  );

  const groupedHistoryArtifacts = useMemo(() => {
    const groups: Array<{ label: string; items: GroupedArtifact[] }> = [];
    const roots = buildArtifactTree(filteredArtifacts);
    for (const root of roots) {
      const label = dayGroupLabel(root.updated_at);
      const existing = groups[groups.length - 1];
      if (!existing || existing.label !== label) {
        groups.push({ label, items: [root] });
      } else {
        existing.items.push(root);
      }
    }
    return groups;
  }, [filteredArtifacts, buildArtifactTree]);

  const groupedRecentArtifacts = useMemo(() => {
    const groups: Array<{ label: string; items: GroupedArtifact[] }> = [];
    const roots = buildArtifactTree(artifacts).slice(0, 6);

    for (const root of roots) {
      const label = dayGroupLabel(root.updated_at);
      const existing = groups[groups.length - 1];
      if (!existing || existing.label !== label) {
        groups.push({ label, items: [root] });
      } else {
        existing.items.push(root);
      }
    }
    return groups;
  }, [artifacts, buildArtifactTree]);

  const selectedArtifactIdSet = useMemo(
    () => new Set(selectedArtifactIds),
    [selectedArtifactIds],
  );

  const isSelectionMode = selectedArtifactIds.length > 0;

  const homeVisibleArtifactIds = useMemo(
    () =>
      groupedRecentArtifacts.flatMap((group) =>
        group.items.map((artifact) => artifact.id),
      ),
    [groupedRecentArtifacts],
  );

  const historyVisibleArtifactIds = useMemo(
    () =>
      groupedHistoryArtifacts.flatMap((group) =>
        group.items.map((artifact) => artifact.id),
      ),
    [groupedHistoryArtifacts],
  );

  const canStartFileTranscription = useMemo(() => {
    if (!settings || !selectedFile || isStarting) {
      return false;
    }

    const selectedModel = settings.transcription.model;
    const modelEntry = modelCatalog.find(
      (entry) => entry.key === selectedModel,
    );
    if (!modelEntry) {
      return true;
    }

    return modelEntry.installed;
  }, [isStarting, modelCatalog, selectedFile, settings]);

  const canStartRealtime = useMemo(() => {
    if (!settings || realtimeState !== "idle" || isStoppingRealtime) {
      return false;
    }

    const selectedModel = settings.transcription.model;
    const modelEntry = modelCatalog.find(
      (entry) => entry.key === selectedModel,
    );
    if (!modelEntry) {
      return true;
    }

    return modelEntry.installed;
  }, [isStoppingRealtime, modelCatalog, realtimeState, settings]);

  const aiFeaturesAvailable = useMemo(
    () => aiActionsAvailable(aiCapabilityStatus),
    [aiCapabilityStatus],
  );

  const aiUnavailableReason = t(
    "error.aiUnavailable",
    "No usable AI provider is available. Configure it in Settings > AI Services.",
  );

  useEffect(() => {
    if (section === "home" || section === "history") {
      return;
    }
    setSelectedArtifactIds([]);
  }, [section]);

  useEffect(() => {
    const artifactIds = new Set(artifacts.map((artifact) => artifact.id));
    setSelectedArtifactIds((previous) =>
      previous.filter((id) => artifactIds.has(id)),
    );
  }, [artifacts]);

  const detailSegments = useMemo(
    () => parseTimelineV2Segments(activeArtifact?.metadata?.timeline_v2),
    [activeArtifact?.metadata?.timeline_v2],
  );
  const speakerColorMap = useMemo(
    () =>
      sanitizeSpeakerColorMap(
        settings?.transcription.speaker_diarization?.speaker_colors ?? {},
      ),
    [settings?.transcription.speaker_diarization?.speaker_colors],
  );

  const selectedDetailSegment = useMemo(
    () =>
      selectedSegmentSourceIndex === null
        ? null
        : (detailSegments.find(
            (segment) => segment.sourceIndex === selectedSegmentSourceIndex,
          ) ?? null),
    [detailSegments, selectedSegmentSourceIndex],
  );

  const resolveEmotionSegmentSourceIndex = useCallback(
    (
      segmentIndex: number,
      evidenceText?: string | null,
      timeLabel?: string | null,
      startSeconds?: number | null,
    ): number | null => {
      if (
        detailSegments.some((segment) => segment.sourceIndex === segmentIndex)
      ) {
        return segmentIndex;
      }

      const normalizedEvidence = evidenceText?.trim().toLowerCase() ?? "";
      if (startSeconds !== null && startSeconds !== undefined) {
        const byStartSeconds = detailSegments.find(
          (segment) =>
            segment.startSeconds !== null &&
            Math.abs(segment.startSeconds - startSeconds) < 0.25 &&
            (!normalizedEvidence ||
              segment.line.toLowerCase().includes(normalizedEvidence) ||
              normalizedEvidence.includes(segment.line.trim().toLowerCase())),
        );
        if (byStartSeconds) {
          return byStartSeconds.sourceIndex;
        }
      }

      if (timeLabel) {
        const byTime = detailSegments.find(
          (segment) =>
            segment.time === timeLabel &&
            (!normalizedEvidence ||
              segment.line.toLowerCase().includes(normalizedEvidence) ||
              normalizedEvidence.includes(segment.line.trim().toLowerCase())),
        );
        if (byTime) {
          return byTime.sourceIndex;
        }
      }

      if (normalizedEvidence) {
        const byEvidence = detailSegments.find((segment) => {
          const normalizedLine = segment.line.trim().toLowerCase();
          return (
            normalizedLine.includes(normalizedEvidence) ||
            normalizedEvidence.includes(normalizedLine)
          );
        });
        if (byEvidence) {
          return byEvidence.sourceIndex;
        }
      }

      return null;
    },
    [detailSegments],
  );

  const setSegmentElementRef = useCallback(
    (sourceIndex: number, node: HTMLElement | null) => {
      if (node) {
        segmentElementMapRef.current.set(sourceIndex, node);
      } else {
        segmentElementMapRef.current.delete(sourceIndex);
      }
    },
    [],
  );

  const contextMenuSegment = useMemo(
    () =>
      segmentContextMenu === null
        ? null
        : (detailSegments.find(
            (segment) => segment.sourceIndex === segmentContextMenu.sourceIndex,
          ) ?? null),
    [detailSegments, segmentContextMenu],
  );

  const knownSpeakers = useMemo<KnownSpeaker[]>(() => {
    const speakersById = new Map<string, { id: string; label: string }>();

    detailSegments.forEach((segment) => {
      const label = segment.speakerLabel?.trim();
      if (!label) {
        return;
      }

      const id = normalizeSpeakerColorKey(segment.speakerId ?? label);
      if (!speakersById.has(id)) {
        speakersById.set(id, { id, label });
      }
    });

    return Array.from(speakersById.values())
      .sort((left, right) =>
        left.label.localeCompare(right.label, undefined, {
          sensitivity: "base",
        }),
      )
      .map((speaker) => ({
        ...speaker,
        color:
          resolveSpeakerColor({
            speakerId: speaker.id,
            speakerLabel: speaker.label,
            colorMap: speakerColorMap,
          }) ?? "#4F7CFF",
      }));
  }, [detailSegments, speakerColorMap]);
  const knownSpeakerLabels = useMemo(
    () => knownSpeakers.map((speaker) => speaker.label),
    [knownSpeakers],
  );
  const mergeTargetSpeakers = useMemo(
    () =>
      knownSpeakers.filter((speaker) => speaker.id !== mergeSpeakerSourceId),
    [knownSpeakers, mergeSpeakerSourceId],
  );
  const artifactDiarizationUiState = useMemo(
    () => getArtifactDiarizationUiState(activeArtifact, knownSpeakerLabels),
    [activeArtifact, knownSpeakerLabels],
  );
  const selectedSegmentSpeakerLabel =
    selectedDetailSegment?.speakerLabel?.trim() ?? "";
  const canRenameSelectedSpeaker =
    detailMode === "segments" &&
    Boolean(activeArtifact) &&
    selectedSegmentSourceIndex !== null &&
    selectedSegmentSpeakerLabel.length > 0 &&
    speakerDraft.trim().length > 0 &&
    speakerDraft.trim() !== selectedSegmentSpeakerLabel;
  const speakerDynamicsAvailable = knownSpeakerLabels.length > 1;
  const canMergeSpeakers =
    Boolean(activeArtifact) &&
    knownSpeakers.length > 1 &&
    mergeSpeakerSourceId.length > 0 &&
    mergeSpeakerTargetId.length > 0 &&
    mergeSpeakerSourceId !== mergeSpeakerTargetId;
  const showSpeakerManagement = detailMode === "segments";
  const emotionAnalysisGeneratedAt =
    activeArtifact?.metadata?.[EMOTION_ANALYSIS_GENERATED_AT_METADATA_KEY] ??
    "";

  useEffect(() => {
    if (selectedSegmentSourceIndex === null) {
      return;
    }
    const exists = detailSegments.some(
      (segment) => segment.sourceIndex === selectedSegmentSourceIndex,
    );
    if (!exists) {
      setSelectedSegmentSourceIndex(null);
      setSpeakerDraft("");
    }
  }, [detailSegments, selectedSegmentSourceIndex]);

  useEffect(() => {
    if (selectedSegmentSourceIndex === null) {
      setSpeakerDraft("");
      return;
    }
    const selected = detailSegments.find(
      (segment) => segment.sourceIndex === selectedSegmentSourceIndex,
    );
    setSpeakerDraft(selected?.speakerLabel ?? "");
  }, [detailSegments, selectedSegmentSourceIndex]);

  useEffect(() => {
    if (knownSpeakers.length === 0) {
      setMergeSpeakerSourceId("");
      return;
    }

    setMergeSpeakerSourceId((previous) =>
      knownSpeakers.some((speaker) => speaker.id === previous)
        ? previous
        : (knownSpeakers[0]?.id ?? ""),
    );
  }, [knownSpeakers]);

  useEffect(() => {
    if (mergeTargetSpeakers.length === 0) {
      setMergeSpeakerTargetId("");
      return;
    }

    setMergeSpeakerTargetId((previous) =>
      mergeTargetSpeakers.some((speaker) => speaker.id === previous)
        ? previous
        : (mergeTargetSpeakers[0]?.id ?? ""),
    );
  }, [mergeTargetSpeakers]);

  useEffect(() => {
    if (detailMode !== "segments" || selectedSegmentSourceIndex === null) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      const element = segmentElementMapRef.current.get(
        selectedSegmentSourceIndex,
      );
      element?.scrollIntoView({
        block: "center",
        inline: "nearest",
        behavior: "smooth",
      });
    });

    return () => window.cancelAnimationFrame(frame);
  }, [detailMode, selectedSegmentSourceIndex]);

  useEffect(() => {
    if (segmentContextMenu === null) {
      return;
    }
    const exists = detailSegments.some(
      (segment) => segment.sourceIndex === segmentContextMenu.sourceIndex,
    );
    if (!exists) {
      setSegmentContextMenu(null);
    }
  }, [detailSegments, segmentContextMenu]);

  useEffect(() => {
    setSegmentContextMenu(null);
  }, [activeArtifactId, detailMode, section]);

  useEffect(() => {
    if (!segmentContextMenu) {
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest(".segment-context-menu")
      ) {
        return;
      }
      setSegmentContextMenu(null);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setSegmentContextMenu(null);
      }
    };

    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [segmentContextMenu]);

  useEffect(() => {
    if (!error) return;
    const timer = setTimeout(() => {
      setError(null);
    }, 6000);
    return () => clearTimeout(timer);
  }, [error]);

  const activeTrimmedAudioDraft = useMemo(() => {
    if (!trimmedAudioDraft || !activeArtifact) {
      return null;
    }
    return trimmedAudioDraft.parentArtifactId === activeArtifact.id
      ? trimmedAudioDraft
      : null;
  }, [activeArtifact, trimmedAudioDraft]);

  const effectiveDetailContext = useMemo(() => {
    if (activeTrimmedAudioDraft) {
      return buildActiveDetailContext({
        artifactAudioId: null,
        inputPath: activeTrimmedAudioDraft.path,
        requestedTitle: activeTrimmedAudioDraft.title,
        sourceArtifact: activeArtifact,
        trimmedAudioDraft: activeTrimmedAudioDraft,
        restoreArtifactOnFailure: false,
      });
    }

    if (activeArtifact) {
      return buildActiveDetailContext({
        artifactAudioId: activeArtifact.audio_available
          ? activeArtifact.id
          : null,
        inputPath: null,
        requestedTitle: activeArtifact.title,
        sourceArtifact: activeArtifact,
        restoreArtifactOnFailure: false,
      });
    }

    return activeDetailContext;
  }, [activeArtifact, activeDetailContext, activeTrimmedAudioDraft]);

  const effectiveTrimmedAudioDraft = useMemo(
    () =>
      effectiveDetailContext?.trimmedAudioDraft ??
      activeTrimmedAudioDraft ??
      null,
    [activeTrimmedAudioDraft, effectiveDetailContext],
  );
  const realtimeTranscriptText = useMemo(
    () =>
      realtimeFinalLines.filter((line) => line.trim().length > 0).join("\n"),
    [realtimeFinalLines],
  );
  const realtimePreviewText = realtimePreview.trim();
  const realtimeTranscriptDisplayText = useMemo(
    () =>
      [...realtimeFinalLines, realtimePreviewText]
        .filter((line) => line.trim().length > 0)
        .join("\n"),
    [realtimeFinalLines, realtimePreviewText],
  );
  const realtimeHasAnyText =
    realtimeTranscriptText.trim().length > 0 || realtimePreviewText.length > 0;
  const isRealtimeDetailActive =
    realtimeSessionOpen && !activeArtifact && !focusedJobId;

  const detailAudioInputPath = useMemo(() => {
    if (effectiveDetailContext?.inputPath) {
      return effectiveDetailContext.inputPath;
    }
    return null;
  }, [effectiveDetailContext]);
  const detailAudioArtifactId = useMemo(
    () => effectiveDetailContext?.artifactAudioId ?? null,
    [effectiveDetailContext],
  );

  const detailAudioFileLabel = useMemo(
    () =>
      detailAudioInputPath
        ? fileLabel(detailAudioInputPath)
        : activeArtifact?.source_label ||
          effectiveDetailContext?.sourceArtifact?.source_label ||
          t("inspector.unknown", "Unknown"),
    [
      activeArtifact?.source_label,
      detailAudioInputPath,
      effectiveDetailContext,
      language,
    ],
  );

  const detailAudioFormat = useMemo(() => {
    const formatSource =
      detailAudioInputPath ??
      activeArtifact?.source_label ??
      effectiveDetailContext?.sourceArtifact?.source_label ??
      null;
    if (!formatSource) {
      return t("inspector.unknown", "Unknown");
    }
    const extension = formatSource.split(".").pop();
    if (!extension) {
      return t("inspector.unknown", "Unknown");
    }
    return extension.toUpperCase();
  }, [
    activeArtifact?.source_label,
    detailAudioInputPath,
    effectiveDetailContext,
    language,
  ]);

  const transcriptSeconds = useMemo(() => {
    if (audioDurationSeconds > 0) {
      return Math.round(audioDurationSeconds);
    }
    const persistedDuration = parseArtifactDurationSeconds(activeArtifact);
    if (persistedDuration > 0) {
      return persistedDuration;
    }
    const timelineDuration = detailSegments.reduce((maxSeconds, segment) => {
      const candidate = segment.endSeconds ?? segment.startSeconds ?? 0;
      return Math.max(maxSeconds, candidate);
    }, 0);
    if (timelineDuration > 0) {
      return Math.round(timelineDuration);
    }
    return 0;
  }, [activeArtifact, audioDurationSeconds, detailSegments]);

  const activeRawTranscript = activeArtifact?.raw_transcript ?? "";
  const persistedOptimizedTranscriptAvailable =
    hasPersistedOptimizedTranscript(activeArtifact);
  const hasOptimizedTranscript =
    !effectiveTrimmedAudioDraft &&
    Boolean(activeArtifact) &&
    (optimizedTranscriptAvailable || persistedOptimizedTranscriptAvailable);
  const visibleTranscript = useMemo(() => {
    if (transcriptViewMode === "original" && hasOptimizedTranscript) {
      return activeRawTranscript;
    }
    return draftTranscript;
  }, [
    activeRawTranscript,
    draftTranscript,
    hasOptimizedTranscript,
    transcriptViewMode,
  ]);
  const transcriptReadOnly =
    transcriptViewMode === "original" && hasOptimizedTranscript;
  const confidenceTranscriptDocument = useMemo(
    () =>
      buildConfidenceTranscript(
        activeRawTranscript,
        activeArtifact?.metadata?.timeline_v2,
      ),
    [activeArtifact?.metadata?.timeline_v2, activeRawTranscript],
  );
  const confidenceColorsAvailable = Boolean(confidenceTranscriptDocument);
  const showingConfidenceTranscript =
    showConfidenceColors &&
    transcriptViewMode === "original" &&
    Boolean(confidenceTranscriptDocument);

  const transcriptWordCount = useMemo(
    () => visibleTranscript.split(/\s+/).filter(Boolean).length,
    [visibleTranscript],
  );

  const optimizedTranscriptForPersistence = useMemo(() => {
    if (!activeArtifact) {
      return draftTranscript;
    }
    if (optimizedTranscriptAvailable || persistedOptimizedTranscriptAvailable) {
      return draftTranscript;
    }
    if (draftTranscript !== activeRawTranscript) {
      return draftTranscript;
    }
    return "";
  }, [
    activeArtifact,
    activeRawTranscript,
    draftTranscript,
    optimizedTranscriptAvailable,
    persistedOptimizedTranscriptAvailable,
  ]);

  const hasTranscriptDraftChanges = useMemo(() => {
    if (!activeArtifact) {
      return false;
    }
    const baselineTranscript = persistedOptimizedTranscriptAvailable
      ? activeArtifact.optimized_transcript
      : activeRawTranscript;
    return draftTranscript !== baselineTranscript;
  }, [
    activeArtifact,
    activeRawTranscript,
    draftTranscript,
    persistedOptimizedTranscriptAvailable,
  ]);

  const runningQueueJob = useMemo(
    () =>
      activeJobId
        ? (queueItems.find((item) => item.job_id === activeJobId) ?? null)
        : null,
    [activeJobId, queueItems],
  );

  const runningJobPercentage = useMemo(
    () => activeJobPercentage(activeJobId, runningQueueJob, progress),
    [activeJobId, progress, runningQueueJob],
  );

  const focusedQueueJob = useMemo(
    () =>
      focusedJobId
        ? (queueItems.find((item) => item.job_id === focusedJobId) ?? null)
        : null,
    [focusedJobId, queueItems],
  );

  const rawActiveTranscriptionPercentage = useMemo(
    () => activeJobPercentage(focusedJobId, focusedQueueJob, progress),
    [focusedJobId, focusedQueueJob, progress],
  );
  const [
    displayedTranscriptionPercentage,
    setDisplayedTranscriptionPercentage,
  ] = useState(0);
  const displayedTranscriptionJobIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!hasOptimizedTranscript && transcriptViewMode !== "original") {
      setTranscriptViewMode("original");
    }
  }, [hasOptimizedTranscript, transcriptViewMode]);

  useEffect(() => {
    if (!confidenceColorsAvailable && showConfidenceColors) {
      setShowConfidenceColors(false);
    }
  }, [confidenceColorsAvailable, showConfidenceColors]);

  useEffect(() => {
    if (showConfidenceColors && transcriptViewMode !== "original") {
      setTranscriptViewMode("original");
    }
  }, [showConfidenceColors, transcriptViewMode]);

  useEffect(() => {
    if (!focusedJobId) {
      displayedTranscriptionJobIdRef.current = null;
      setDisplayedTranscriptionPercentage(0);
      return;
    }
    if (displayedTranscriptionJobIdRef.current !== focusedJobId) {
      displayedTranscriptionJobIdRef.current = focusedJobId;
      setDisplayedTranscriptionPercentage(rawActiveTranscriptionPercentage);
      return;
    }
    setDisplayedTranscriptionPercentage((previous) =>
      Math.max(previous, rawActiveTranscriptionPercentage),
    );
  }, [focusedJobId, rawActiveTranscriptionPercentage]);

  const queueActiveItems = useMemo(
    () =>
      queueItems.filter(
        (entry) => !["completed", "cancelled", "failed"].includes(entry.stage),
      ),
    [queueItems],
  );

  const exportPreviewText = useMemo(() => {
    const transcript = visibleTranscript.trim();
    if (transcript) {
      return transcript;
    }
    return activeRawTranscript.trim();
  }, [activeRawTranscript, visibleTranscript]);
  const segmentsAlignedWithVisibleTranscript =
    transcriptViewMode === "original" || !hasOptimizedTranscript;

  const selectedPromptTemplate = useMemo(() => {
    if (!settings || !activePromptId) return null;
    return (
      settings.prompts.templates.find(
        (template) => template.id === activePromptId,
      ) ?? null
    );
  }, [activePromptId, settings]);

  const visibleSettingsPanes = useMemo(() => {
    const query = settingsQuery.trim().toLowerCase();
    if (!query) return settingsPaneDefinitions;

    return settingsPaneDefinitions.filter((pane) => {
      return (
        pane.label.toLowerCase().includes(query) ||
        pane.description.toLowerCase().includes(query)
      );
    });
  }, [settingsPaneDefinitions, settingsQuery]);

  const visibleSettingsPaneGroups = useMemo(() => {
    const groups = new Map<
      SettingsPaneDefinition["group"],
      SettingsPaneDefinition[]
    >();

    for (const pane of visibleSettingsPanes) {
      const existing = groups.get(pane.group);
      if (existing) {
        existing.push(pane);
      } else {
        groups.set(pane.group, [pane]);
      }
    }

    return Array.from(groups.entries()).map(([group, panes]) => ({
      group,
      panes,
    }));
  }, [visibleSettingsPanes]);

  const enabledRemoteServices = useMemo(() => {
    return (settings?.ai.remote_services ?? []).filter(
      (service) => service.enabled,
    );
  }, [settings?.ai.remote_services]);

  const activeAiServiceSelectValue = useMemo(() => {
    if (!settings) return AI_SERVICE_NONE;
    if (settings.ai.active_provider === "foundation_apple") {
      return AI_SERVICE_FOUNDATION;
    }

    const activeRemoteId = settings.ai.active_remote_service_id;
    if (
      activeRemoteId &&
      enabledRemoteServices.some((service) => service.id === activeRemoteId)
    ) {
      return `remote:${activeRemoteId}`;
    }

    if (settings.ai.active_provider === "gemini") {
      const googleService = enabledRemoteServices.find(
        (service) => service.kind === "google",
      );
      if (googleService) {
        return `remote:${googleService.id}`;
      }
    }

    return AI_SERVICE_NONE;
  }, [enabledRemoteServices, settings]);

  const aiServiceSelectOptions = useMemo(() => {
    const options: Array<{ value: string; label: string; disabled?: boolean }> =
      [
        {
          value: AI_SERVICE_NONE,
          label: t("settings.ai.noProvider", "No AI provider"),
        },
      ];

    if (platformIsAppleSilicon) {
      options.push({
        value: AI_SERVICE_FOUNDATION,
        label: t("settings.ai.foundationModel", "Foundation Model"),
        disabled: !settings?.ai.providers.foundation_apple.enabled,
      });
    }

    for (const service of enabledRemoteServices) {
      options.push({
        value: `remote:${service.id}`,
        label: formatRemoteServiceLabel(service, settings),
      });
    }

    return options;
  }, [
    enabledRemoteServices,
    platformIsAppleSilicon,
    settings?.ai.providers.foundation_apple.enabled,
    settings?.ai.providers.gemini.model,
    language,
  ]);

  const chatPromptSuggestions = useMemo<ChatPromptSuggestion[]>(() => {
    return (settings?.prompts.templates ?? []).map((prompt) => ({
      id: prompt.id,
      label: prompt.name,
      body: prompt.body,
    }));
  }, [settings?.prompts.templates]);

  useEffect(() => {
    setAudioDurationSeconds(parseArtifactDurationSeconds(activeArtifact));
  }, [activeArtifact, detailAudioArtifactId, detailAudioInputPath]);

  useEffect(() => {
    if (!realtimeSessionOpen || realtimeStartedAtMs === null) {
      setRealtimeElapsedSeconds(0);
      return;
    }

    const updateElapsed = (): void => {
      setRealtimeElapsedSeconds(
        Math.max(0, (Date.now() - realtimeStartedAtMs) / 1000),
      );
    };

    updateElapsed();
    const timer = window.setInterval(updateElapsed, 500);
    return () => {
      window.clearInterval(timer);
    };
  }, [realtimeSessionOpen, realtimeStartedAtMs]);

  useEffect(() => {
    if (visibleSettingsPanes.length === 0) return;
    if (!visibleSettingsPanes.some((pane) => pane.key === settingsPane)) {
      setSettingsPane(visibleSettingsPanes[0].key);
    }
  }, [settingsPane, visibleSettingsPanes]);

  function setProvisioningState(status: {
    ready: boolean;
    models_dir: string;
    missing_models: string[];
    missing_encoders: string[];
    pyannote: RuntimeHealth["pyannote"];
  }): void {
    setProvisioning((previous) => ({
      ...previous,
      ready: status.ready,
      modelsDir: status.models_dir,
      missing: [...status.missing_models, ...status.missing_encoders],
      pyannote: status.pyannote,
      running: false,
      progress: null,
      statusMessage: status.ready
        ? t("settings.localModels.readyMessage", "Local models are ready")
        : t(
            "settings.localModels.missingAssets",
            "{count} model assets missing",
            {
              count:
                status.missing_models.length + status.missing_encoders.length,
            },
      ),
    }));
  }

  function syncProvisioningFromRuntimeHealth(health: RuntimeHealth): void {
    const missingAssets = [...health.missing_models, ...health.missing_encoders];
    setProvisioning((previous) => ({
      ...previous,
      ready: missingAssets.length === 0,
      modelsDir: health.models_dir_resolved || previous.modelsDir,
      missing: missingAssets,
      pyannote: health.pyannote,
    }));
  }

  function syncArtifactDraftState(
    artifact: TranscriptArtifact,
    options?: {
      resetChat?: boolean;
      resetDetailMode?: boolean;
      clearDetailContext?: boolean;
    },
  ): void {
    const optimizedTranscriptExists = hasPersistedOptimizedTranscript(artifact);
    setActiveArtifact(artifact);
    if (options?.clearDetailContext) {
      setActiveDetailContext(null);
    }
    setDraftTitle(artifact.title);
    setDraftTranscript(
      optimizedTranscriptExists
        ? artifact.optimized_transcript
        : artifact.raw_transcript,
    );
    setOptimizedTranscriptAvailable(optimizedTranscriptExists);
    setTranscriptViewMode(optimizedTranscriptExists ? "optimized" : "original");
    setDraftSummary(artifact.summary);
    setDraftFaqs(artifact.faqs);
    setDraftEmotionAnalysis(parsePersistedEmotionAnalysis(artifact));
    setTrimRetranscriptionError(null);
    if (options?.resetChat) {
      if (copiedChatResetTimerRef.current !== null) {
        window.clearTimeout(copiedChatResetTimerRef.current);
        copiedChatResetTimerRef.current = null;
      }
      setChatInput("");
      setChatHistory([]);
      setCopiedChatMessageId(null);
    }
    if (options?.resetDetailMode) {
      setDetailMode("transcript");
      setInspectorMode("details");
    }
  }

  function hydrateDetail(artifact: TranscriptArtifact): void {
    syncArtifactDraftState(artifact, {
      resetChat: true,
      resetDetailMode: true,
      clearDetailContext: true,
    });
  }

  function nextChatMessageId(prefix: "user" | "assistant"): string {
    chatMessageSerialRef.current += 1;
    return `${prefix}-${chatMessageSerialRef.current}`;
  }

  function focusRunningJob(
    jobId: string,
    detailContext: ActiveDetailContext | null | undefined,
  ): void {
    clearStartupWatchdog();
    const switchingJob = focusedJobIdRef.current !== jobId;
    setFocusedJobId(jobId);
    setActiveDetailContext(detailContext ?? null);
    setActiveArtifact(null);
    setDetailMode("transcript");
    setInspectorMode("details");
    setSection("detail");
    setTrimRetranscriptionError(null);
    setError(null);
    if (switchingJob) {
      setActiveJobPreviewText("");
      activeJobDeltaSequenceRef.current = -1;
    }
  }

  useEffect(() => {
    return () => {
      if (copiedChatResetTimerRef.current !== null) {
        window.clearTimeout(copiedChatResetTimerRef.current);
        copiedChatResetTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!aiFeaturesAvailable) {
      return;
    }
    const artifactId = activeArtifact?.id ?? null;
    const persistedSummary = activeArtifact?.summary ?? "";
    if (
      !shouldAutostartSummary({
        enabled: summaryAutostart,
        artifactId,
        persistedSummary,
        draftSummary,
        hasActiveJob: Boolean(activeJobId),
        isGeneratingSummary,
        triggeredArtifactIds: summaryAutostartedArtifactIdsRef.current,
      })
    ) {
      return;
    }

    if (artifactId) {
      summaryAutostartedArtifactIdsRef.current.add(artifactId);
    }
    void onGenerateSummary(false);
  }, [
    activeArtifact?.id,
    activeArtifact?.summary,
    activeJobId,
    aiFeaturesAvailable,
    draftSummary,
    isGeneratingSummary,
    summaryAutostart,
  ]);

  async function onCopyChatExchange(messageId: string): Promise<void> {
    const assistantIndex = chatHistory.findIndex(
      (message) => message.id === messageId,
    );
    const assistantMessage =
      assistantIndex >= 0 ? chatHistory[assistantIndex] : null;
    if (
      !assistantMessage ||
      assistantMessage.role !== "assistant" ||
      !assistantMessage.canCopy
    ) {
      return;
    }

    const clipboardText = buildChatClipboardText({
      messages: chatHistory,
      assistantIndex,
      questionLabel: t("detail.chatCopyQuestionLabel", "Question"),
      answerLabel: t("detail.chatCopyAnswerLabel", "Answer"),
    });

    try {
      await navigator.clipboard.writeText(clipboardText);
      setCopiedChatMessageId(messageId);
      if (copiedChatResetTimerRef.current !== null) {
        window.clearTimeout(copiedChatResetTimerRef.current);
      }
      copiedChatResetTimerRef.current = window.setTimeout(() => {
        setCopiedChatMessageId((current) =>
          current === messageId ? null : current,
        );
        copiedChatResetTimerRef.current = null;
      }, 1800);
    } catch {
      setError(t("detail.chatCopyFailed", "Copy failed. Please try again."));
    }
  }

  function onStartSidebarResize(
    side: "left" | "right",
    event: ReactMouseEvent<HTMLDivElement>,
  ): void {
    event.preventDefault();
    event.stopPropagation();
    setActiveSidebarResize(side);
  }

  const windowFrameStyle = useMemo<CSSProperties | undefined>(
    () =>
      leftSidebarOpen
        ? ({ "--left-sidebar-width": `${leftSidebarWidth}px` } as CSSProperties)
        : undefined,
    [leftSidebarOpen, leftSidebarWidth],
  );

  const detailLayoutStyle = useMemo<CSSProperties | undefined>(
    () =>
      effectiveRightSidebarOpen
        ? ({
            "--detail-inspector-width": `${rightSidebarWidth}px`,
          } as CSSProperties)
        : undefined,
    [effectiveRightSidebarOpen, rightSidebarWidth],
  );

  function restoreDetailAfterFailedTranscription(
    detailContext: ActiveDetailContext | null | undefined,
  ): boolean {
    if (
      !detailContext?.restoreArtifactOnFailure ||
      !detailContext.sourceArtifact
    ) {
      return false;
    }

    if (detailContext.trimmedAudioDraft) {
      setTrimmedAudioDraft(detailContext.trimmedAudioDraft);
      setTrimRegions(detailContext.trimmedAudioDraft.regions);
    }
    hydrateDetail(detailContext.sourceArtifact);
    setSection("detail");
    return true;
  }

  function presentTranscriptionFailure(
    message: string | null | undefined,
    detailContext: ActiveDetailContext | null | undefined,
  ): void {
    const normalizedMessage = normalizeJobFailureMessage(
      message,
      t("error.transcriptionFailed", "Transcription failed."),
    );
    const isTrimFailure = Boolean(detailContext?.trimmedAudioDraft);
    const restored = restoreDetailAfterFailedTranscription(detailContext);

    if (isTrimFailure) {
      setTrimRetranscriptionError(normalizedMessage);
      setError(null);
      if (!restored) {
        setSection("detail");
      }
      return;
    }

    setTrimRetranscriptionError(null);
    setError(normalizedMessage);
    if (!restored) {
      setSection("home");
    }
  }

  async function persistSettings(
    updated: AppSettings,
    previous: AppSettings | null,
  ): Promise<void> {
    const sequence = ++settingsSaveSequenceRef.current;
    const normalized = normalizeSettings(updated);
    setSettings(normalized);

    try {
      const persisted = await saveSettings(normalized);
      if (sequence === settingsSaveSequenceRef.current) {
        setSettings(normalizeSettings(persisted));
      }
    } catch (settingsError) {
      if (sequence === settingsSaveSequenceRef.current && previous) {
        setSettings(previous);
      }
      setError(
        formatUiError(
          "error.saveSettings",
          "Could not save settings",
          settingsError,
        ),
      );
    }
  }

  async function patchSettings(
    mutator: (current: AppSettings) => AppSettings,
  ): Promise<void> {
    if (!settings) return;
    const previous = normalizeSettings(settings);
    const next = normalizeSettings(mutator(normalizeSettings(settings)));
    await persistSettings(next, previous);
  }

  async function patchAiSettings(
    mutator: (current: AppSettings["ai"]) => AppSettings["ai"],
  ): Promise<void> {
    if (!settings) return;
    const previous = normalizeSettings(settings);
    const next = normalizeSettings({
      ...previous,
      ai: mutator(previous.ai),
    });

    const sequence = ++settingsSaveSequenceRef.current;
    setSettings(next);

    try {
      const persisted = await saveSettingsPartial({ ai: next.ai });
      if (sequence === settingsSaveSequenceRef.current) {
        setSettings(normalizeSettings(persisted));
      }
    } catch (settingsError) {
      if (sequence === settingsSaveSequenceRef.current) {
        setSettings(previous);
      }
      setError(
        formatUiError(
          "error.saveSettings",
          "Could not save settings",
          settingsError,
        ),
      );
    }
  }

  async function refreshSettingsFromDisk(): Promise<void> {
    try {
      const next = await fetchSettingsSnapshot();
      const normalized = normalizeSettings(next);
      setSettings(normalized);
      if (normalized.general.app_language) {
        changeLanguage(normalized.general.app_language);
      }
    } catch (error) {
      setError(
        formatUiError(
          "error.reloadSettings",
          "Could not reload settings",
          error,
        ),
      );
    }
  }

  function promptForBackupPassword(message: string): string | null {
    const first = window.prompt(message, "");
    if (first === null) return null;
    if (first.trim().length < 8) {
      setError(
        t(
          "settings.advanced.backupPasswordTooShort",
          "Backup password must be at least 8 characters long.",
        ),
      );
      return null;
    }
    return first;
  }

  async function onExportAppBackup(): Promise<void> {
    const destination = await save({
      defaultPath: `sbobino-backup-${new Date().toISOString().slice(0, 10)}.sbobino-backup`,
      filters: [
        {
          name: t("settings.advanced.backupFile", "Sbobino backup"),
          extensions: ["sbobino-backup"],
        },
      ],
    });

    if (!destination) {
      return;
    }

    const password = promptForBackupPassword(
      t(
        "settings.advanced.backupPasswordPrompt",
        "Enter a password to encrypt this portable backup.",
      ),
    );
    if (!password) {
      return;
    }

    const confirmation = window.prompt(
      t(
        "settings.advanced.backupPasswordConfirmPrompt",
        "Re-enter the backup password to confirm.",
      ),
      "",
    );
    if (confirmation === null) {
      return;
    }
    if (confirmation !== password) {
      setError(
        t(
          "settings.advanced.backupPasswordMismatch",
          "Backup passwords do not match.",
        ),
      );
      return;
    }

    setIsRunningBackupAction(true);
    try {
      const result = await exportAppBackup({
        destination_path: destination,
        password,
      });
      setError(null);
      window.alert(
        t(
          "settings.advanced.backupExported",
          `Backup exported to ${result.path}`,
        ),
      );
    } catch (backupError) {
      setError(
        formatUiError(
          "error.backupExportFailed",
          "Backup export failed",
          backupError,
        ),
      );
    } finally {
      setIsRunningBackupAction(false);
    }
  }

  async function onImportAppBackup(): Promise<void> {
    const picked = await open({
      multiple: false,
      directory: false,
      filters: [
        {
          name: t("settings.advanced.backupFile", "Sbobino backup"),
          extensions: ["sbobino-backup"],
        },
      ],
    });

    if (!picked || Array.isArray(picked)) {
      return;
    }

    const confirmed = await confirmDialog(
      t(
        "settings.advanced.backupImportConfirm",
        "Importing a backup will replace the current local archive on this device. Continue?",
      ),
      {
        title: t("settings.advanced.backupImportTitle", "Import backup"),
        kind: "warning",
        okLabel: t("settings.advanced.importBackup", "Import backup"),
        cancelLabel: t("action.cancel", "Cancel"),
      },
    );
    if (!confirmed) {
      return;
    }

    const password = promptForBackupPassword(
      t(
        "settings.advanced.backupImportPasswordPrompt",
        "Enter the password used to encrypt this backup.",
      ),
    );
    if (!password) {
      return;
    }

    setIsRunningBackupAction(true);
    try {
      await importAppBackup({
        backup_path: picked,
        password,
      });
      window.location.reload();
    } catch (backupError) {
      setError(
        formatUiError(
          "error.backupImportFailed",
          "Backup import failed",
          backupError,
        ),
      );
    } finally {
      setIsRunningBackupAction(false);
    }
  }

  async function refreshRuntimeHealth(): Promise<void> {
    try {
      const health = await fetchRuntimeHealth();
      setRuntimeHealth(health);
      writeLastSeenAppVersion(health.app_version);
      syncProvisioningFromRuntimeHealth(health);
    } catch (healthError) {
      setError(
        formatUiError(
          "error.readRuntimeHealth",
          "Could not read transcription runtime health",
          healthError,
        ),
      );
    }
  }

  async function refreshProvisioningModels(): Promise<void> {
    try {
      const [models, health] = await Promise.all([
        provisioningModels(),
        fetchRuntimeHealth(),
      ]);
      setModelCatalog(models);
      setRuntimeHealth(health);
      writeLastSeenAppVersion(health.app_version);
      syncProvisioningFromRuntimeHealth(health);
    } catch (modelsError) {
      setError(
        formatUiError(
          "error.readModelsCatalog",
          "Could not read models catalog",
          modelsError,
        ),
      );
    }
  }

  async function loadStartupRequirements(options?: {
    includeModelCatalog?: boolean;
  }): Promise<StartupRequirementsSnapshot> {
    const includeModelCatalog = options?.includeModelCatalog !== false;
    const [models, health] = await Promise.all([
      includeModelCatalog ? provisioningModels() : Promise.resolve(modelCatalog),
      fetchRuntimeHealth(),
    ]);

    syncProvisioningFromRuntimeHealth(health);
    if (includeModelCatalog) {
      setModelCatalog(models);
    }
    setRuntimeHealth(health);
    writeLastSeenAppVersion(health.app_version);
    setStartupRequirementsLoaded(true);
    setStartupRequirementsError(null);

    return {
      modelCatalog: models,
      runtimeHealth: health,
    };
  }

  async function persistInitialSetupReport(
    mutator: (current: InitialSetupReport) => InitialSetupReport,
  ): Promise<void> {
    const next = mutator(initialSetupReportRef.current);
    initialSetupReportRef.current = next;
    try {
      await writeSetupReport(next);
    } catch (reportError) {
      console.warn("Failed to persist setup report", reportError);
    }
  }

  async function updateInitialSetupStepState(
    stepId: InitialSetupStepId,
    status: InitialSetupReport["steps"][number]["status"],
    label: string,
    detail?: string | null,
  ): Promise<void> {
    initialSetupStepIdRef.current = stepId;
    setInitialSetupStepLabel(label);
    setInitialSetupStepDetail(detail ?? null);
    await persistInitialSetupReport((current) =>
      updateInitialSetupReportStep(current, stepId, status, detail, label),
    );
  }

  function inferInitialSetupReasonCode(
    snapshot: StartupRequirementsSnapshot | null,
  ): string {
    if (!snapshot) {
      return "setup_incomplete";
    }
    if (!isRuntimeToolchainReady(snapshot.runtimeHealth)) {
      return "runtime_repair_required";
    }
    if (
      getInitialSetupMissingModels(
        snapshot.modelCatalog,
        snapshot.runtimeHealth.is_apple_silicon,
      ).length > 0
    ) {
      return "models_missing";
    }
    return snapshot.runtimeHealth.setup_complete
      ? "setup_complete"
      : "setup_incomplete";
  }

  function inferPostUpdateValidationReasonCode(
    snapshot: StartupRequirementsSnapshot | null,
  ): string {
    if (!snapshot) {
      return "post_update_validation_failed";
    }
    if (snapshot.runtimeHealth.setup_complete) {
      return "post_update_validation_complete";
    }
    if (!isRuntimeToolchainReady(snapshot.runtimeHealth)) {
      return "runtime_repair_required";
    }
    return "post_update_validation_incomplete";
  }

  function formatPostUpdateValidationFailure(
    snapshot: StartupRequirementsSnapshot | null,
  ): string {
    if (!snapshot) {
      return t(
        "startup.postUpdate.validationFailed",
        "Sbobino could not finish the post-update checks. You can keep using the app and retry from Settings > Local Models.",
      );
    }
    if (!isRuntimeToolchainReady(snapshot.runtimeHealth)) {
      return formatRuntimeNotReadyMessage(snapshot.runtimeHealth);
    }
    return t(
      "startup.postUpdate.validationIncomplete",
      "Sbobino found missing local components after the update. You can keep using the app and repair them from Settings > Local Models.",
    );
  }

  async function maybeStartPyannoteBackgroundAction(
    trigger: PyannoteBackgroundActionTrigger,
    appVersionOverride?: string | null,
  ): Promise<void> {
    const appVersion = appVersionOverride ?? currentBuildVersion;
    if (!appVersion) {
      return;
    }

    try {
      const action = await planPyannoteBackgroundAction(trigger);
      if (action.status === "migrate_manifest") {
        void refreshRuntimeHealth();
        return;
      }
      if (!action.should_start) {
        return;
      }

      const marker = {
        appVersion,
        trigger,
        reasonCode: action.reason_code || "pyannote_repair_required",
      };
      if (
        matchesPyannoteAutoActionMarker(
          readLastPyannoteAutoActionMarker(),
          marker,
        )
      ) {
        return;
      }

      writeLastPyannoteAutoActionMarker(marker);
      pyannoteProvisioningActiveRef.current = true;
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        progress: null,
        statusMessage: getPyannoteBackgroundActionStatusMessage(action),
      }));

      const result = await provisioningInstallPyannote(action.force_reinstall);
      if (result.started) {
        return;
      }

      pyannoteProvisioningActiveRef.current = false;
      writeLastPyannoteAutoActionMarker(null);
      setProvisioning((previous) => ({
        ...previous,
        running: false,
        statusMessage: action.message || t("settings.pyannote.desc"),
      }));
    } catch (error) {
      pyannoteProvisioningActiveRef.current = false;
      writeLastPyannoteAutoActionMarker(null);
      console.warn(`Automatic pyannote action '${trigger}' failed:`, error);
    }
  }

  async function waitForProvisioningRun(
    starter: () => Promise<{ started: boolean }>,
    options?: { waitForExistingRun?: boolean },
  ): Promise<void> {
    let unlisten: (() => void) | undefined;

    try {
      const completion = new Promise<void>((resolve, reject) => {
        void (async () => {
          unlisten = await subscribeProvisioningStatus((event) => {
            if (event.state === "completed") {
              resolve();
              return;
            }

            if (event.state === "cancelled") {
              reject(
                new Error(
                  event.message ||
                    t("provisioning.cancelled", "Provisioning cancelled"),
                ),
              );
              return;
            }

            if (event.state === "error") {
              reject(
                new Error(
                  event.message ||
                    t("error.provisioningFailed", "Provisioning failed"),
                ),
              );
            }
          });

          const result = await starter();
          if (!result.started && !options?.waitForExistingRun) {
            resolve();
          }
        })().catch(reject);
      });

      await completion;
    } finally {
      unlisten?.();
      await loadStartupRequirements();
    }
  }

  async function ensurePyannoteReadyForDiarizedJob(): Promise<void> {
    const action = await planPyannoteBackgroundAction("job_requires_diarization");
    if (action.status === "migrate_manifest") {
      await refreshRuntimeHealth();
      return;
    }
    if (!action.should_start) {
      return;
    }

    setProvisioning((previous) => ({
      ...previous,
      running: true,
      progress: null,
      statusMessage: getPyannoteBackgroundActionStatusMessage(action),
    }));

    if (pyannoteProvisioningActiveRef.current) {
      await waitForProvisioningRun(
        async () => ({ started: false }),
        { waitForExistingRun: true },
      );
      return;
    }

    pyannoteProvisioningActiveRef.current = true;
    await waitForProvisioningRun(
      () => provisioningInstallPyannote(action.force_reinstall),
      { waitForExistingRun: false },
    );
  }

  async function acceptPrivacyPolicy(): Promise<void> {
    if (!settings) {
      return;
    }

    setAcceptingPrivacyPolicy(true);
    try {
      await updateInitialSetupStepState(
        "privacy",
        "running",
        t("setup.firstLaunch.accept", "Accept and continue"),
        t(
          "setup.firstLaunch.privacyIntro",
          "Review the privacy summary before enabling local setup.",
        ),
      );
      await patchSettings((current) => ({
        ...current,
        general: {
          ...current.general,
          privacy_policy_version_accepted: PRIVACY_POLICY_VERSION,
          privacy_policy_accepted_at: new Date().toISOString(),
        },
      }));
      await updateInitialSetupStepState(
        "privacy",
        "completed",
        t("setup.firstLaunch.accept", "Accept and continue"),
        t("setup.firstLaunch.policyVersion", "Policy version {version}", {
          version: PRIVACY_POLICY_VERSION,
        }),
      );
      await persistInitialSetupReport((current) => ({
        ...current,
        privacy_accepted: true,
        updated_at: new Date().toISOString(),
      }));
      autoInitialSetupAttemptedRef.current = false;
    } finally {
      setAcceptingPrivacyPolicy(false);
    }
  }

  async function beginInitialSetup(): Promise<void> {
    setInitialSetupRunning(true);
    setInitialSetupError(null);
    setInitialSetupStepLabel(null);
    setInitialSetupStepDetail(null);
    setStartupRequirementsError(null);

    let snapshot: StartupRequirementsSnapshot | null = null;

    try {
      await persistInitialSetupReport(() => ({
        ...createInitialSetupReport(),
        privacy_accepted: privacyPolicyAccepted,
        runtime_health: runtimeHealth,
        setup_complete: false,
        startup_gate_complete: false,
        validated_build_version: null,
        updated_at: new Date().toISOString(),
      }));

      snapshot = await loadStartupRequirements();
      const loadedSnapshot = snapshot;
      await persistInitialSetupReport((current) => ({
        ...current,
        runtime_health: loadedSnapshot.runtimeHealth,
        updated_at: new Date().toISOString(),
      }));

      if (!isRuntimeToolchainReady(snapshot.runtimeHealth)) {
        await updateInitialSetupStepState(
          "speech-runtime",
          "running",
          t(
            "setup.firstLaunch.preparingRuntime",
            "Installing local transcription runtime...",
          ),
          t(
            "setup.firstLaunch.inspectingDesc",
            "Checking local tools and prerequisites.",
          ),
        );
        await waitForProvisioningRun(() => provisioningInstallRuntime(false));
        snapshot = await loadStartupRequirements();
        await updateInitialSetupStepState(
          "speech-runtime",
          "completed",
          t(
            "setup.firstLaunch.preparingRuntime",
            "Installing local transcription runtime...",
          ),
          t(
            "provisioning.runtimeReady",
            "Local transcription runtime is ready",
          ),
        );
      }

      if (!isRuntimeToolchainReady(snapshot.runtimeHealth)) {
        throw new Error(formatRuntimeNotReadyMessage(snapshot.runtimeHealth));
      }

      await updateInitialSetupStepState(
        "whisper-models",
        "running",
        t("setup.firstLaunch.downloading", "Downloading local models..."),
        t(
          "setup.firstLaunch.downloadingDesc",
          "This can take a few minutes the first time.",
        ),
      );
      for (const model of ["base", "large_turbo"] as SpeechModel[]) {
        const entry = findProvisioningModelEntry(snapshot.modelCatalog, model);
        if (
          isProvisionedModelReady(
            entry,
            snapshot.runtimeHealth.is_apple_silicon,
          )
        ) {
          continue;
        }

        const currentSnapshot = snapshot;
        initialSetupStepIdRef.current = "whisper-models";
        setInitialSetupStepLabel(
          t("setup.firstLaunch.downloadingModel", "Downloading {model}...", {
            model: entry?.label ?? model,
          }),
        );
        setInitialSetupStepDetail(
          t(
            "setup.firstLaunch.downloadingDesc",
            "This can take a few minutes the first time.",
          ),
        );
        await waitForProvisioningRun(() =>
          provisioningDownloadModel({
            model,
            include_coreml: currentSnapshot.runtimeHealth.is_apple_silicon,
          }),
        );
        snapshot = await loadStartupRequirements();
      }
      await updateInitialSetupStepState(
        "whisper-models",
        "completed",
        t("setup.firstLaunch.downloading", "Downloading local models..."),
        t("settings.localModels.readyMessage", "Local models are ready"),
      );

      await updateInitialSetupStepState(
        "final-validation",
        "running",
        t("setup.firstLaunch.inspecting", "Inspecting local runtime..."),
        INITIAL_SETUP_REQUIRES_PYANNOTE
          ? t(
              "setup.firstLaunch.inspectingDesc",
              "Checking local tools and prerequisites.",
            )
          : t(
              "setup.firstLaunch.inspectingRuntimeOnlyDesc",
              "Checking local transcription tools and required models.",
            ),
      );

      if (
        getInitialSetupMissingModels(
          snapshot.modelCatalog,
          snapshot.runtimeHealth.is_apple_silicon,
        ).length > 0
      ) {
        throw new Error(
          t(
            "setup.firstLaunch.assetsStillMissing",
            "Initial setup did not complete correctly. Please try again.",
          ),
        );
      }
      if (!snapshot.runtimeHealth.setup_complete) {
        throw new Error(
          t(
            "setup.firstLaunch.assetsStillMissing",
            "Initial setup did not complete correctly. Please try again.",
          ),
        );
      }

      await updateInitialSetupStepState(
        "final-validation",
        "completed",
        t("setup.firstLaunch.inspecting", "Inspecting local runtime..."),
        t("settings.localModels.readyMessage", "Local models are ready"),
      );
      await persistInitialSetupReport((current) => ({
        ...current,
        setup_complete: snapshot?.runtimeHealth.setup_complete ?? false,
        startup_gate_complete: snapshot?.runtimeHealth.setup_complete ?? false,
        validated_build_version: snapshot?.runtimeHealth.app_version ?? null,
        final_reason_code: inferInitialSetupReasonCode(snapshot),
        final_error: null,
        last_validation_reason_code: "post_update_validation_complete",
        last_validation_error: null,
        runtime_health: snapshot?.runtimeHealth ?? null,
        updated_at: new Date().toISOString(),
      }));
      setRequiresPostUpdateValidation(false);
      setPostUpdateValidationError(null);
      setPostUpdateValidationVisible(false);
      initialSetupStepIdRef.current = null;
      setInitialSetupStepLabel(null);
      setInitialSetupStepDetail(null);
    } catch (setupError) {
      const finalError = formatUiError(
        "error.firstLaunchSetupFailed",
        "Initial setup failed",
        setupError,
      );
      const failedStepId = initialSetupStepIdRef.current;
      if (failedStepId) {
        const failedStep = initialSetupReportRef.current.steps.find(
          (step) => step.id === failedStepId,
        );
        await updateInitialSetupStepState(
          failedStepId,
          "failed",
          failedStep?.label ??
            initialSetupStepLabel ??
            t("setup.firstLaunch.setupError", "Setup could not finish"),
          finalError,
        );
      }
      await persistInitialSetupReport((current) => ({
        ...current,
        setup_complete: false,
        startup_gate_complete: false,
        validated_build_version: null,
        final_reason_code: inferInitialSetupReasonCode(snapshot),
        final_error: finalError,
        last_validation_reason_code: inferPostUpdateValidationReasonCode(snapshot),
        last_validation_error: finalError,
        runtime_health: snapshot?.runtimeHealth ?? current.runtime_health,
        updated_at: new Date().toISOString(),
      }));
      setInitialSetupError(finalError);
    } finally {
      setInitialSetupRunning(false);
    }
  }

  async function refreshActiveArtifacts(): Promise<void> {
    const activeArtifactsSnapshot = await listRecentArtifacts();
    setArtifacts(activeArtifactsSnapshot);
  }

  async function onSelectAiService(value: string): Promise<void> {
    if (!settings) return;

    if (value === AI_SERVICE_NONE) {
      await patchAiSettings((current) => ({
        ...current,
        active_provider: "none",
        active_remote_service_id: null,
      }));
      return;
    }

    if (value === AI_SERVICE_FOUNDATION) {
      await patchAiSettings((current) => ({
        ...current,
        active_provider: "foundation_apple",
        providers: {
          ...current.providers,
          foundation_apple: {
            ...current.providers.foundation_apple,
            enabled: true,
          },
        },
      }));
      return;
    }

    if (!value.startsWith("remote:")) {
      return;
    }

    const targetId = value.slice("remote:".length);
    await patchAiSettings((current) => {
      const targetService = (current.remote_services ?? []).find(
        (service) => service.id === targetId,
      );
      if (!targetService) return current;

      return {
        ...current,
        active_provider: targetService.kind === "google" ? "gemini" : "none",
        active_remote_service_id: targetId,
      };
    });
  }

  async function refreshDeletedArtifactsList(): Promise<void> {
    const deletedArtifactsSnapshot = await listDeletedArtifacts({ limit: 200 });
    setDeletedArtifacts(deletedArtifactsSnapshot);
  }

  async function onPickFile(): Promise<void> {
    const picked = await open({
      multiple: false,
      filters: [
        {
          name: t("home.audioVideoFiles", "Audio/Video"),
          extensions: [
            "m4a",
            "mp3",
            "wav",
            "aac",
            "flac",
            "ogg",
            "opus",
            "webm",
            "mp4",
            "mov",
            "mkv",
            "wma",
            "aiff",
          ],
        },
      ],
    });

    if (picked && !Array.isArray(picked)) {
      primeSelectedFileForHome(picked);
    }
  }

  async function onChangeLanguage(language: LanguageCode): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        language,
      },
    }));
  }

  async function onChangeModel(model: SpeechModel): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        model,
      },
    }));
  }

  async function onChangeTranscriptionEngine(
    engine: TranscriptionEngine,
  ): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        engine,
      },
    }));
  }

  async function onPatchSpeakerDiarizationSettings(
    mutator: (
      current: SpeakerDiarizationSettings,
    ) => SpeakerDiarizationSettings,
  ): Promise<void> {
    if (!settings) {
      return;
    }

    const previousDiarization = sanitizeSpeakerDiarizationSettings(
      settings.transcription.speaker_diarization ??
        getDefaultSpeakerDiarizationSettings(),
    );
    const nextDiarization = sanitizeSpeakerDiarizationSettings(
      mutator(previousDiarization),
    );

    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        speaker_diarization: nextDiarization,
      },
    }));

    if (!previousDiarization.enabled && nextDiarization.enabled) {
      void maybeStartPyannoteBackgroundAction("enable_diarization");
    }
    void refreshRuntimeHealth();
  }

  async function onPatchSpeakerDiarizationPreferences(
    mutator: (
      current: SpeakerDiarizationSettings,
    ) => SpeakerDiarizationSettings,
  ): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        speaker_diarization: sanitizeSpeakerDiarizationSettings(
          mutator(
            current.transcription.speaker_diarization ??
              getDefaultSpeakerDiarizationSettings(),
          ),
        ),
      },
    }));
  }

  async function onSetSpeakerColor(
    speakerKey: string,
    nextColor: string,
  ): Promise<void> {
    try {
      await onPatchSpeakerDiarizationPreferences((current) => ({
        ...current,
        speaker_colors: setSpeakerColorForKey(
          current.speaker_colors,
          speakerKey,
          nextColor,
        ),
      }));
      setError(null);
    } catch (colorError) {
      setError(
        formatUiError(
          "error.speakerColorFailed",
          "Failed to save speaker color",
          colorError,
        ),
      );
    }
  }

  async function onPatchWhisperOptions(
    mutator: (current: WhisperOptions) => WhisperOptions,
  ): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        whisper_options: sanitizeWhisperOptions(
          mutator(
            current.transcription.whisper_options ??
              getDefaultWhisperOptions(platformIsAppleSilicon),
          ),
        ),
      },
    }));
  }

  const launchTranscriptionStart = useCallback(
    async (
      request: TranscriptionStartRequest,
      options?: { queuedJobId?: string },
    ): Promise<void> => {
      if (!settings) return;

      const targetFile = request.inputPath;
      const parentId = request.parentId;
      const requestedTitle = request.title?.trim()
        ? request.title.trim()
        : undefined;
      const nextDetailContext = request.detailContext ?? null;
      const preserveCurrentArtifact = Boolean(
        activeArtifact && section === "detail",
      );

      clearStartupWatchdog();
      setIsStarting(true);
      setError(null);
      setTrimRetranscriptionError(null);
      try {
        const trimValidationError = validateTrimmedAudioDraftForTranscription(
          request.trimValidationSnapshot ?? null,
        );
        if (trimValidationError) {
          if (preserveCurrentArtifact) {
            setError(trimValidationError);
          } else {
            presentTranscriptionFailure(trimValidationError, nextDetailContext);
          }
          if (options?.queuedJobId) {
            setQueueItems((previous) =>
              previous.filter((entry) => entry.job_id !== options.queuedJobId),
            );
          }
          return;
        }

        const runtimeStatus = await withTimeout(
          ensureTranscriptionRuntime(),
          20_000,
          t("error.runtimeSetupTimedOut", "Runtime setup timed out."),
        );
        if (!runtimeStatus.ready) {
          const failureMessage =
            runtimeStatus.message ||
            formatRuntimeNotReadyMessage(runtimeHealth);
          if (preserveCurrentArtifact) {
            setError(failureMessage);
          } else {
            presentTranscriptionFailure(failureMessage, nextDetailContext);
          }
          if (options?.queuedJobId) {
            setQueueItems((previous) =>
              previous.filter((entry) => entry.job_id !== options.queuedJobId),
            );
          }
          return;
        }
        if (runtimeStatus.did_setup) {
          void refreshRuntimeHealth();
        }

        if (settings.transcription.speaker_diarization?.enabled) {
          try {
            await ensurePyannoteReadyForDiarizedJob();
          } catch (pyannoteError) {
            const failureMessage = formatUiError(
              "error.pyannoteInstallFailed",
              "Pyannote install failed",
              pyannoteError,
            );
            if (preserveCurrentArtifact) {
              setError(failureMessage);
            } else {
              presentTranscriptionFailure(
                failureMessage,
                nextDetailContext,
              );
            }
            if (options?.queuedJobId) {
              setQueueItems((previous) =>
                previous.filter((entry) => entry.job_id !== options.queuedJobId),
              );
            }
            return;
          }
        }

        let preflight: TranscriptionStartPreflight | null = null;
        try {
          preflight = await withTimeout(
            fetchTranscriptionStartPreflight({
              model: settings.transcription.model,
            }),
            8_000,
            t("error.preflightTimedOut", "Preflight timed out."),
          );
        } catch (preflightError) {
          console.warn(
            "Transcription preflight failed, continuing with backend start:",
            preflightError,
          );
        }
        if (preflight && !preflight.allowed) {
          const failureMessage = formatTranscriptionPreflightMessage(preflight);
          if (preserveCurrentArtifact) {
            setError(failureMessage);
          } else {
            presentTranscriptionFailure(failureMessage, nextDetailContext);
          }
          if (options?.queuedJobId) {
            setQueueItems((previous) =>
              previous.filter((entry) => entry.job_id !== options.queuedJobId),
            );
          }
          return;
        }

        const startResult = await withTimeout(
          startTranscription({
            input_path: targetFile,
            engine: settings.transcription.engine,
            language: settings.transcription.language,
            model: settings.transcription.model,
            enable_ai: false,
            whisper_options: sanitizeWhisperOptions(
              settings.transcription.whisper_options ??
                getDefaultWhisperOptions(platformIsAppleSilicon),
            ),
            title: requestedTitle,
            parent_id: parentId,
          }),
          12_000,
          t(
            "error.startRequestTimedOut",
            "Start request timed out while waiting for backend response.",
          ),
        );

        const { job_id } = startResult;
        pendingTranscriptionContextRef.current.set(job_id, {
          inputPath: targetFile,
          parentId,
          title: requestedTitle,
          detailContext: nextDetailContext,
        });

        if (failedJobMessagesRef.current.has(job_id)) {
          const earlyFailure =
            failedJobMessagesRef.current.get(job_id) ??
            t("error.transcriptionFailed", "Transcription failed.");
          failedJobMessagesRef.current.delete(job_id);
          const failedContext =
            pendingTranscriptionContextRef.current.get(job_id);
          pendingTranscriptionContextRef.current.delete(job_id);
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobTitle("");
          if (options?.queuedJobId) {
            setQueueItems((previous) =>
              previous.filter((entry) => entry.job_id !== options.queuedJobId),
            );
          }
          if (preserveCurrentArtifact) {
            setError(earlyFailure);
          } else {
            setFocusedJobId(null);
            setActiveJobPreviewText("");
            activeJobDeltaSequenceRef.current = -1;
            setActiveDetailContext(null);
            presentTranscriptionFailure(
              earlyFailure,
              failedContext?.detailContext,
            );
          }
          return;
        }

        activeJobIdRef.current = job_id;
        setJobStarted(job_id);
        setQueueItems((previous) => {
          const startedJob = buildQueuedTranscriptionJob(
            job_id,
            t("queue.queuedJob", "Queued transcription job."),
          );
          if (options?.queuedJobId) {
            const replaced = replaceQueuedTranscriptionJob(
              previous,
              options.queuedJobId,
              startedJob,
            );
            return replaced.some((entry) => entry.job_id === job_id)
              ? replaced
              : pushOrReplaceQueueItem(replaced, startedJob);
          }
          if (previous.some((entry) => entry.job_id === job_id)) {
            return previous;
          }
          return pushOrReplaceQueueItem(previous, startedJob);
        });
        setActiveJobTitle(requestedTitle ?? fileLabel(targetFile));

        if (!preserveCurrentArtifact) {
          setShowExportSheet(false);
          focusRunningJob(job_id, nextDetailContext);
        }

        startupWatchdogRef.current = window.setTimeout(() => {
          if (activeJobIdRef.current !== job_id) {
            return;
          }
          const stalledContext =
            pendingTranscriptionContextRef.current.get(job_id);
          pendingTranscriptionContextRef.current.delete(job_id);
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobTitle("");
          if (focusedJobIdRef.current === job_id) {
            setFocusedJobId(null);
            setActiveJobPreviewText("");
            activeJobDeltaSequenceRef.current = -1;
            setActiveDetailContext(null);
            presentTranscriptionFailure(
              t(
                "error.transcriptionStartupProblem",
                "Transcription is not starting correctly. Check Whisper CLI/Whisper Stream paths in Settings > Local Models.",
              ),
              stalledContext?.detailContext,
            );
          } else {
            setError(
              t(
                "error.transcriptionStartupProblem",
                "Transcription is not starting correctly. Check Whisper CLI/Whisper Stream paths in Settings > Local Models.",
              ),
            );
          }
        }, 120_000);
      } catch (startError) {
        clearStartupWatchdog();
        if (options?.queuedJobId) {
          setQueueItems((previous) =>
            previous.filter((entry) => entry.job_id !== options.queuedJobId),
          );
        }
        if (preserveCurrentArtifact) {
          setError(formatAppError(startError));
        } else {
          setActiveDetailContext(null);
          presentTranscriptionFailure(
            formatAppError(startError),
            nextDetailContext,
          );
        }
      } finally {
        setIsStarting(false);
      }
    },
    [
      activeArtifact,
      clearStartupWatchdog,
      focusRunningJob,
      platformIsAppleSilicon,
      presentTranscriptionFailure,
      refreshRuntimeHealth,
      section,
      settings,
      t,
    ],
  );

  async function onStartTranscription(
    fileToProcess?: string,
    options?: { parentId?: string; title?: string },
  ): Promise<void> {
    const preparedHomeTrim =
      !fileToProcess &&
      preparedImportTrimDraft &&
      preparedImportTrimDraft.sourcePath === selectedFile
        ? preparedImportTrimDraft
        : null;
    const targetFile =
      fileToProcess && typeof fileToProcess === "string"
        ? fileToProcess
        : (preparedHomeTrim?.path ?? selectedFile);
    if (!settings || !targetFile) return;
    const parentId = options?.parentId;
    const requestedTitle = options?.title?.trim()
      ? options.title.trim()
      : preparedHomeTrim?.title;
    const sourceArtifactForContext = parentId
      ? activeArtifact
      : section === "detail"
        ? activeArtifact
        : null;
    const isTrimRetranscription =
      trimmedAudioDraft?.path === targetFile &&
      trimmedAudioDraft.parentArtifactId === parentId;
    const nextDetailContext = buildActiveDetailContext({
      inputPath: targetFile,
      requestedTitle,
      sourceArtifact: sourceArtifactForContext,
      trimmedAudioDraft: isTrimRetranscription ? trimmedAudioDraft : null,
      restoreArtifactOnFailure: isTrimRetranscription,
    });
    const trimValidationSnapshot = isTrimRetranscription
      ? trimmedAudioDraft
      : preparedHomeTrim;
    const request: TranscriptionStartRequest = {
      inputPath: targetFile,
      parentId,
      title: requestedTitle,
      detailContext: nextDetailContext,
      trimValidationSnapshot,
    };

    if (!isTrimRetranscription) {
      setTrimmedAudioDraft(null);
      setTrimRegions([]);
    }

    if (activeJobId || isStarting) {
      const queueId = buildQueuedTranscriptionJobId(
        ++queuedTranscriptionSequenceRef.current,
      );
      setQueuedTranscriptionStarts((previous) => [
        ...previous,
        { ...request, queueId },
      ]);
      setQueueItems((previous) =>
        pushOrReplaceQueueItem(
          previous,
          buildQueuedTranscriptionJob(
            queueId,
            t("queue.queuedJob", "Queued transcription job."),
          ),
        ),
      );
      setError(null);
      setTrimRetranscriptionError(null);
      return;
    }

    await launchTranscriptionStart(request);
  }

  useEffect(() => {
    if (activeJobId || isStarting || queuedTranscriptionStarts.length === 0) {
      return;
    }

    const [next, ...rest] = queuedTranscriptionStarts;
    if (!next) {
      return;
    }

    setQueuedTranscriptionStarts(rest);
    void launchTranscriptionStart(next, { queuedJobId: next.queueId });
  }, [
    activeJobId,
    isStarting,
    launchTranscriptionStart,
    queuedTranscriptionStarts,
  ]);

  async function onCancel(): Promise<void> {
    if (!activeJobId) return;

    try {
      await cancelTranscription(activeJobId);
    } catch (cancelError) {
      setError(
        formatUiError(
          "error.cancelTranscriptionFailed",
          "Failed to cancel transcription",
          cancelError,
        ),
      );
    }
  }

  async function onOpenArtifact(artifactId: string): Promise<void> {
    try {
      const artifact = await getArtifact(artifactId);
      if (!artifact) {
        setError(t("error.transcriptNotFound", "Transcript not found."));
        return;
      }

      hydrateDetail(artifact);
      setSection("detail");
      setError(null);
    } catch (artifactError) {
      setError(
        formatUiError(
          "error.openTranscriptFailed",
          "Failed to open transcript",
          artifactError,
        ),
      );
    }
  }

  async function onOpenStandaloneSettingsWindow(
    pane?: SettingsPane,
  ): Promise<void> {
    try {
      await openSettingsWindow(pane);
    } catch (windowError) {
      setError(
        formatUiError(
          "error.openSettingsFailed",
          "Failed to open settings window",
          windowError,
        ),
      );
    }
  }

  function onFocusQueueJob(item: JobProgress): void {
    if (isQueuedTranscriptionJobId(item.job_id)) {
      return;
    }
    const pendingContext = pendingTranscriptionContextRef.current.get(
      item.job_id,
    );
    focusRunningJob(item.job_id, pendingContext?.detailContext ?? null);
  }

  function onCloseOpenedArtifact(): void {
    setShowExportSheet(false);
    setActiveArtifact(null);
    setActiveDetailContext(null);
    setTrimRetranscriptionError(null);
    setSection("history");
    setError(null);
  }

  async function onSaveArtifact(): Promise<void> {
    if (!activeArtifact) return;

    setIsSavingArtifact(true);
    try {
      const updated = await updateArtifact({
        id: activeArtifact.id,
        optimized_transcript: optimizedTranscriptForPersistence,
        summary: draftSummary,
        faqs: draftFaqs,
      });

      if (!updated) {
        setError(
          t("error.artifactNotFoundSaving", "Artifact not found while saving."),
        );
        return;
      }

      if (draftTitle.trim() && draftTitle.trim() !== updated.title) {
        const renamed = await renameArtifact({
          id: updated.id,
          new_title: draftTitle.trim(),
        });
        if (renamed) {
          upsertArtifact(renamed);
          hydrateDetail(renamed);
        } else {
          upsertArtifact(updated);
          hydrateDetail(updated);
        }
      } else {
        upsertArtifact(updated);
        hydrateDetail(updated);
      }

      setError(null);
    } catch (saveError) {
      setError(
        formatUiError(
          "error.saveChangesFailed",
          "Failed to save changes",
          saveError,
        ),
      );
    } finally {
      setIsSavingArtifact(false);
    }
  }

  async function onAssignSpeakerToSegment(
    sourceIndex: number,
    speakerLabel: string | null,
    propagateToNeighbors: boolean,
  ): Promise<void> {
    if (!activeArtifact) {
      setError(t("inspector.noTranscript"));
      return;
    }

    const parsedTimeline = parseTimelineV2Document(
      activeArtifact.metadata?.timeline_v2,
    );
    if (!parsedTimeline) {
      setError(
        t(
          "error.segmentTimelineInvalid",
          "Segment timeline metadata is missing or invalid.",
        ),
      );
      return;
    }
    if (sourceIndex < 0 || sourceIndex >= parsedTimeline.segments.length) {
      setError(
        t("error.segmentOutOfRange", "Selected segment is out of range."),
      );
      return;
    }

    const normalizedSpeakerLabel = speakerLabel?.trim() ?? "";
    const shouldSetSpeaker = normalizedSpeakerLabel.length > 0;
    const speakerId = shouldSetSpeaker
      ? normalizeSpeakerId(normalizedSpeakerLabel)
      : null;

    const nextSegments = parsedTimeline.segments.map((segment) => ({
      ...segment,
    }));
    const assignSpeaker = (index: number) => {
      const target = { ...nextSegments[index] };
      if (shouldSetSpeaker) {
        target.speaker_label = normalizedSpeakerLabel;
        target.speaker_id = speakerId;
      } else {
        delete target.speaker_label;
        delete target.speaker_id;
      }
      nextSegments[index] = target;
    };

    assignSpeaker(sourceIndex);

    if (shouldSetSpeaker && propagateToNeighbors) {
      const maxGapSeconds = 8;

      for (let index = sourceIndex - 1; index >= 0; index -= 1) {
        const segment = nextSegments[index];
        if (readSegmentSpeakerLabel(segment)) {
          break;
        }

        const currentEnd = readSegmentEndSeconds(segment);
        const nextStart = readSegmentStartSeconds(nextSegments[index + 1]);
        if (
          currentEnd !== null &&
          nextStart !== null &&
          nextStart - currentEnd > maxGapSeconds
        ) {
          break;
        }

        assignSpeaker(index);
      }

      for (
        let index = sourceIndex + 1;
        index < nextSegments.length;
        index += 1
      ) {
        const segment = nextSegments[index];
        if (readSegmentSpeakerLabel(segment)) {
          break;
        }

        const previousEnd = readSegmentEndSeconds(nextSegments[index - 1]);
        const currentStart = readSegmentStartSeconds(segment);
        if (
          previousEnd !== null &&
          currentStart !== null &&
          currentStart - previousEnd > maxGapSeconds
        ) {
          break;
        }

        assignSpeaker(index);
      }
    }

    const nextTimeline = {
      ...parsedTimeline,
      segments: nextSegments,
    };

    setIsAssigningSpeaker(true);
    try {
      const updated = await updateArtifactTimeline({
        id: activeArtifact.id,
        timeline_v2: JSON.stringify(nextTimeline),
      });
      if (!updated) {
        setError(
          t(
            "error.transcriptNotFoundAssigning",
            "Transcript not found while assigning speaker.",
          ),
        );
        return;
      }
      upsertArtifact(updated);
      syncArtifactDraftState(updated);
      setError(null);
    } catch (assignError) {
      setError(
        formatUiError(
          "error.assignSpeakerFailed",
          "Failed to assign speaker",
          assignError,
        ),
      );
    } finally {
      setIsAssigningSpeaker(false);
    }
  }

  async function onAssignSpeakerToSelectedSegment(): Promise<void> {
    if (selectedSegmentSourceIndex === null) {
      setError(t("error.selectSegmentFirst", "Select a segment first."));
      return;
    }
    const speakerLabel = speakerDraft.trim();
    if (!speakerLabel) {
      setError(t("error.speakerNameEmpty", "Speaker name cannot be empty."));
      return;
    }
    await onAssignSpeakerToSegment(
      selectedSegmentSourceIndex,
      speakerLabel,
      propagateSpeakerAssignment,
    );
  }

  async function onRenameSelectedSpeaker(): Promise<void> {
    if (!activeArtifact) {
      setError(t("inspector.noTranscript"));
      return;
    }
    if (selectedSegmentSourceIndex === null) {
      setError(t("error.selectSegmentFirst", "Select a segment first."));
      return;
    }

    const renameResult = renameSpeakerInTimeline(
      activeArtifact.metadata?.timeline_v2,
      selectedSegmentSourceIndex,
      speakerDraft,
    );

    if (!renameResult.ok) {
      switch (renameResult.reason) {
        case "speaker_name_empty":
          setError(
            t("error.speakerNameEmpty", "Speaker name cannot be empty."),
          );
          break;
        case "speaker_missing":
          setError(
            t(
              "error.selectLabeledSpeakerFirst",
              "Select a labeled speaker first.",
            ),
          );
          break;
        case "segment_out_of_range":
          setError(
            t("error.segmentOutOfRange", "Selected segment is out of range."),
          );
          break;
        case "missing_timeline":
        default:
          setError(
            t(
              "error.segmentTimelineInvalid",
              "Segment timeline metadata is missing or invalid.",
            ),
          );
          break;
      }
      return;
    }

    setIsAssigningSpeaker(true);
    try {
      const updated = await updateArtifactTimeline({
        id: activeArtifact.id,
        timeline_v2: JSON.stringify(renameResult.timeline),
      });
      if (!updated) {
        setError(
          t(
            "error.transcriptNotFoundAssigning",
            "Transcript not found while assigning speaker.",
          ),
        );
        return;
      }
      if (
        renameResult.previousSpeakerId &&
        renameResult.previousSpeakerId !== renameResult.nextSpeakerId
      ) {
        try {
          await onPatchSpeakerDiarizationPreferences((current) => ({
            ...current,
            speaker_colors: moveSpeakerColorMapEntry(
              current.speaker_colors,
              renameResult.previousSpeakerId,
              renameResult.nextSpeakerId,
            ),
          }));
        } catch (colorError) {
          console.warn(
            "failed to move speaker color mapping after rename",
            colorError,
          );
        }
      }
      upsertArtifact(updated);
      syncArtifactDraftState(updated);
      setError(null);
    } catch (renameError) {
      setError(
        formatUiError(
          "error.renameSpeakerFailed",
          "Failed to rename speaker",
          renameError,
        ),
      );
    } finally {
      setIsAssigningSpeaker(false);
    }
  }

  async function onMergeSelectedSpeaker(): Promise<void> {
    if (!activeArtifact) {
      setError(t("inspector.noTranscript"));
      return;
    }

    const sourceSpeaker = knownSpeakers.find(
      (speaker) => speaker.id === mergeSpeakerSourceId,
    );
    const targetSpeaker = knownSpeakers.find(
      (speaker) => speaker.id === mergeSpeakerTargetId,
    );

    if (!sourceSpeaker) {
      setError(
        t("error.selectLabeledSpeakerFirst", "Select a labeled speaker first."),
      );
      return;
    }

    if (!targetSpeaker) {
      setError(
        t("error.selectDifferentSpeaker", "Select a different target speaker."),
      );
      return;
    }

    const mergeResult = mergeSpeakerInTimeline(
      activeArtifact.metadata?.timeline_v2,
      sourceSpeaker.id,
      sourceSpeaker.label,
      targetSpeaker.id,
      targetSpeaker.label,
    );

    if (!mergeResult.ok) {
      switch (mergeResult.reason) {
        case "same_speaker":
        case "target_missing":
          setError(
            t(
              "error.selectDifferentSpeaker",
              "Select a different target speaker.",
            ),
          );
          break;
        case "speaker_missing":
          setError(
            t(
              "error.selectLabeledSpeakerFirst",
              "Select a labeled speaker first.",
            ),
          );
          break;
        case "missing_timeline":
        default:
          setError(
            t(
              "error.segmentTimelineInvalid",
              "Segment timeline metadata is missing or invalid.",
            ),
          );
          break;
      }
      return;
    }

    setIsAssigningSpeaker(true);
    try {
      const updated = await updateArtifactTimeline({
        id: activeArtifact.id,
        timeline_v2: JSON.stringify(mergeResult.timeline),
      });
      if (!updated) {
        setError(
          t(
            "error.transcriptNotFoundAssigning",
            "Transcript not found while assigning speaker.",
          ),
        );
        return;
      }

      if (mergeResult.sourceSpeakerId !== mergeResult.targetSpeakerId) {
        try {
          await onPatchSpeakerDiarizationPreferences((current) => ({
            ...current,
            speaker_colors: removeSpeakerColorMapEntry(
              current.speaker_colors,
              mergeResult.sourceSpeakerId,
            ),
          }));
        } catch (colorError) {
          console.warn(
            "failed to remove merged speaker color mapping",
            colorError,
          );
        }
      }

      upsertArtifact(updated);
      syncArtifactDraftState(updated);
      setSpeakerDraft(mergeResult.targetSpeakerLabel);
      setMergeSpeakerSourceId(mergeResult.targetSpeakerId);
      setError(null);
    } catch (mergeError) {
      setError(
        formatUiError(
          "error.mergeSpeakerFailed",
          "Failed to merge speakers",
          mergeError,
        ),
      );
    } finally {
      setIsAssigningSpeaker(false);
    }
  }

  async function onRemoveDetectedSpeaker(speaker: KnownSpeaker): Promise<void> {
    if (!activeArtifact) {
      setError(t("inspector.noTranscript"));
      return;
    }

    const confirmed = await confirmDialog(
      t(
        "inspector.removeSpeakerConfirm",
        'Remove speaker "{speaker}" from this transcript?\n\nAll segments currently assigned to this speaker will become unlabeled.',
        { speaker: speaker.label },
      ),
      {
        title: t("inspector.removeSpeakerConfirmTitle", "Remove speaker"),
        kind: "warning",
      },
    );
    if (!confirmed) {
      return;
    }

    const removeResult = removeSpeakerFromTimeline(
      activeArtifact.metadata?.timeline_v2,
      speaker.id,
      speaker.label,
    );

    if (!removeResult.ok) {
      switch (removeResult.reason) {
        case "speaker_missing":
          setError(
            t(
              "error.selectLabeledSpeakerFirst",
              "Select a labeled speaker first.",
            ),
          );
          break;
        case "missing_timeline":
        default:
          setError(
            t(
              "error.segmentTimelineInvalid",
              "Segment timeline metadata is missing or invalid.",
            ),
          );
          break;
      }
      return;
    }

    setIsAssigningSpeaker(true);
    try {
      const updated = await updateArtifactTimeline({
        id: activeArtifact.id,
        timeline_v2: JSON.stringify(removeResult.timeline),
      });
      if (!updated) {
        setError(
          t(
            "error.transcriptNotFoundAssigning",
            "Transcript not found while assigning speaker.",
          ),
        );
        return;
      }

      try {
        await onPatchSpeakerDiarizationPreferences((current) => ({
          ...current,
          speaker_colors: removeSpeakerColorMapEntry(
            current.speaker_colors,
            removeResult.removedSpeakerId,
          ),
        }));
      } catch (colorError) {
        console.warn(
          "failed to remove speaker color mapping after speaker removal",
          colorError,
        );
      }

      upsertArtifact(updated);
      syncArtifactDraftState(updated);
      if (speakerDraft.trim() === speaker.label.trim()) {
        setSpeakerDraft("");
      }
      setError(null);
    } catch (removeError) {
      setError(
        formatUiError(
          "error.removeSpeakerFailed",
          "Failed to remove speaker",
          removeError,
        ),
      );
    } finally {
      setIsAssigningSpeaker(false);
    }
  }

  async function onClearSpeakerForSegment(sourceIndex: number): Promise<void> {
    await onAssignSpeakerToSegment(sourceIndex, null, false);
  }

  function onStartRenameSpeakerLabel(speakerLabel: string): void {
    const normalizedLabel = speakerLabel.trim();
    if (!normalizedLabel) {
      return;
    }

    const matchingSegment = detailSegments.find(
      (segment) => segment.speakerLabel?.trim() === normalizedLabel,
    );
    const matchingSpeaker = knownSpeakers.find(
      (speaker) => speaker.label.trim() === normalizedLabel,
    );
    setSpeakerDraft(normalizedLabel);
    if (matchingSpeaker) {
      setMergeSpeakerSourceId(matchingSpeaker.id);
    }
    setError(null);

    if (matchingSegment) {
      focusSpeakerInputForSegment(matchingSegment.sourceIndex);
      return;
    }

    setDetailMode("segments");
    setInspectorMode("details");
    setRightSidebarOpen(true);
  }

  function focusSpeakerInputForSegment(sourceIndex: number): void {
    setSelectedSegmentSourceIndex(sourceIndex);
    setDetailMode("segments");
    setInspectorMode("details");
    setRightSidebarOpen(true);
    setSegmentContextMenu(null);
    const focusInput = () => {
      if (!peopleSpeakerInputRef.current) {
        return false;
      }
      peopleSpeakerInputRef.current.focus();
      peopleSpeakerInputRef.current.select();
      return true;
    };
    window.requestAnimationFrame(() => {
      if (!focusInput()) {
        window.requestAnimationFrame(() => {
          focusInput();
        });
      }
    });
  }

  function openSegmentContextMenu(
    event: ReactMouseEvent<HTMLElement>,
    sourceIndex: number,
  ): void {
    event.preventDefault();
    event.stopPropagation();
    setSelectedSegmentSourceIndex(sourceIndex);

    const containerBounds = detailMainRef.current?.getBoundingClientRect();
    if (!containerBounds) {
      setSegmentContextMenu(null);
      return;
    }

    const menuWidth = 252;
    const menuHeight = 320;
    const margin = 10;
    const rawX = event.clientX - containerBounds.left;
    const rawY = event.clientY - containerBounds.top;
    const x = Math.min(
      Math.max(rawX, margin),
      Math.max(margin, containerBounds.width - menuWidth - margin),
    );
    const y = Math.min(
      Math.max(rawY, margin),
      Math.max(margin, containerBounds.height - menuHeight - margin),
    );
    setSegmentContextMenu({ x, y, sourceIndex });
  }

  function onAddSpeakerFromContextMenu(): void {
    if (!segmentContextMenu) {
      return;
    }
    focusSpeakerInputForSegment(segmentContextMenu.sourceIndex);
  }

  function onAssignKnownSpeakerFromContextMenu(speakerLabel: string): void {
    if (!segmentContextMenu) {
      return;
    }
    setSelectedSegmentSourceIndex(segmentContextMenu.sourceIndex);
    setSpeakerDraft(speakerLabel);
    setSegmentContextMenu(null);
    void onAssignSpeakerToSegment(
      segmentContextMenu.sourceIndex,
      speakerLabel,
      false,
    );
  }

  function onClearSpeakerFromContextMenu(): void {
    if (!segmentContextMenu) {
      return;
    }
    setSelectedSegmentSourceIndex(segmentContextMenu.sourceIndex);
    setSpeakerDraft("");
    setSegmentContextMenu(null);
    void onClearSpeakerForSegment(segmentContextMenu.sourceIndex);
  }

  function onRenameArtifact(artifact: TranscriptArtifact): void {
    setRenameTarget(artifact);
    setRenameDraft(artifact.title);
  }

  function closeRenameDialog(): void {
    if (isRenamingArtifact) return;
    setRenameTarget(null);
    setRenameDraft("");
  }

  async function confirmRenameArtifact(): Promise<void> {
    if (!renameTarget || isRenamingArtifact) return;

    const newTitle = renameDraft.trim();
    if (!newTitle) {
      setError(t("error.titleEmpty", "Title cannot be empty."));
      return;
    }

    if (newTitle === renameTarget.title) {
      closeRenameDialog();
      return;
    }

    setIsRenamingArtifact(true);
    try {
      const updated = await renameArtifact({
        id: renameTarget.id,
        new_title: newTitle,
      });
      if (!updated) {
        setError(
          t(
            "error.transcriptNotFoundRenaming",
            "Transcript not found while renaming.",
          ),
        );
        return;
      }

      upsertArtifact(updated);
      if (activeArtifact?.id === updated.id) {
        hydrateDetail(updated);
      }

      setError(null);
      closeRenameDialog();
    } catch (renameError) {
      setError(
        formatUiError("error.renameFailed", "Rename failed", renameError),
      );
    } finally {
      setIsRenamingArtifact(false);
    }
  }

  async function onDeleteArtifactsWithConsent(ids: string[]): Promise<void> {
    const uniqueIds = Array.from(new Set(ids));
    if (uniqueIds.length === 0) return;

    const targets = artifacts.filter((artifact) =>
      uniqueIds.includes(artifact.id),
    );
    if (targets.length === 0) return;

    const details = `${targets
      .slice(0, 5)
      .map((artifact) => `- ${artifact.title}`)
      .join("\n")}${targets.length > 5 ? "\n- ..." : ""}`;

    const confirmed = await confirmDialog(
      targets.length === 1
        ? t(
            "deleted.confirmMove",
            'Move "{title}" to Recently Deleted?\n\nYou can restore this item later from Recently Deleted.',
            {
              title: targets[0].title,
            },
          )
        : t(
            "deleted.confirmMoveMany",
            "Move {count} transcriptions to Recently Deleted?\n\n{items}\n\nYou can restore these items later from Recently Deleted.",
            {
              count: targets.length,
              items: details,
            },
          ),
      {
        title: t("deleted.confirmMoveTitle", "Move to Recently Deleted"),
        kind: "warning",
      },
    );
    if (!confirmed) return;

    try {
      const result = await deleteArtifacts(uniqueIds);
      if (result.deleted <= 0) {
        setError(t("deleted.noneDeleted", "No transcriptions were deleted."));
        return;
      }

      removeArtifacts(uniqueIds);
      await refreshDeletedArtifactsList();

      if (activeArtifact && uniqueIds.includes(activeArtifact.id)) {
        setActiveArtifact(null);
        setActiveDetailContext(null);
        setSection("history");
      }

      setError(null);
    } catch (deleteError) {
      setError(
        formatUiError("error.deleteFailed", "Delete failed", deleteError),
      );
    }
  }

  async function onDeleteArtifact(artifact: TranscriptArtifact): Promise<void> {
    await onDeleteArtifactsWithConsent([artifact.id]);
  }

  function toggleArtifactSelection(id: string): void {
    setSelectedArtifactIds((previous) => {
      const next = new Set(previous);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return Array.from(next);
    });
  }

  function clearArtifactSelection(): void {
    setSelectedArtifactIds([]);
  }

  function selectAllVisibleArtifacts(): void {
    const visibleIds =
      section === "history"
        ? historyVisibleArtifactIds
        : homeVisibleArtifactIds;
    setSelectedArtifactIds(Array.from(new Set(visibleIds)));
  }

  async function onDeleteSelectedArtifacts(): Promise<void> {
    if (selectedArtifactIds.length === 0) {
      return;
    }
    await onDeleteArtifactsWithConsent(selectedArtifactIds);
    setSelectedArtifactIds([]);
  }

  async function onRestoreArtifactsWithConsent(ids: string[]): Promise<void> {
    const uniqueIds = Array.from(new Set(ids));
    if (uniqueIds.length === 0) return;

    const confirmed = await confirmDialog(
      uniqueIds.length === 1
        ? t(
            "deleted.confirmRestore",
            "Restore this transcription from Recently Deleted?",
          )
        : t(
            "deleted.confirmRestoreMany",
            "Restore {count} transcriptions from Recently Deleted?",
            {
              count: uniqueIds.length,
            },
          ),
      { title: t("deleted.confirmRestoreTitle", "Restore transcription") },
    );
    if (!confirmed) return;

    try {
      const result = await restoreArtifacts(uniqueIds);
      if (result.restored <= 0) {
        setError(t("deleted.noneRestored", "No transcriptions were restored."));
        return;
      }

      await Promise.all([
        refreshActiveArtifacts(),
        refreshDeletedArtifactsList(),
      ]);
      setError(null);
    } catch (restoreError) {
      setError(
        formatUiError("error.restoreFailed", "Restore failed", restoreError),
      );
    }
  }

  async function onHardDeleteArtifactsWithConsent(
    ids: string[],
  ): Promise<void> {
    const uniqueIds = Array.from(new Set(ids));
    if (uniqueIds.length === 0) return;

    const confirmed = await confirmDialog(
      uniqueIds.length === 1
        ? t(
            "deleted.confirmPermanentDelete",
            "Permanently delete this transcription from Recently Deleted? This action cannot be undone.",
          )
        : t(
            "deleted.confirmPermanentDeleteMany",
            "Permanently delete {count} transcriptions from Recently Deleted? This action cannot be undone.",
            {
              count: uniqueIds.length,
            },
          ),
      {
        title: t("deleted.confirmPermanentDeleteTitle", "Permanent delete"),
        kind: "warning",
      },
    );
    if (!confirmed) return;

    try {
      const result = await hardDeleteArtifacts(uniqueIds);
      if (result.deleted <= 0) {
        await refreshDeletedArtifactsList();
        setError(
          t(
            "deleted.nonePermanentlyDeleted",
            "No transcriptions were permanently deleted.",
          ),
        );
        return;
      }

      await refreshDeletedArtifactsList();
      setError(null);
    } catch (deleteError) {
      setError(
        formatUiError(
          "error.permanentDeleteFailed",
          "Permanent delete failed",
          deleteError,
        ),
      );
    }
  }

  async function onEmptyTrash(): Promise<void> {
    const confirmed = await confirmDialog(
      t(
        "deleted.confirmEmpty",
        "Empty Recently Deleted? This permanently deletes all trashed transcriptions.",
      ),
      {
        title: t("deleted.confirmEmptyTitle", "Empty Recently Deleted"),
        kind: "warning",
      },
    );
    if (!confirmed) return;

    try {
      const result = await emptyDeletedArtifacts();
      if (result.deleted <= 0) {
        await refreshDeletedArtifactsList();
        setError(
          t("deleted.alreadyEmpty", "Recently Deleted is already empty."),
        );
        return;
      }
      await refreshDeletedArtifactsList();
      setError(null);
    } catch (emptyError) {
      setError(
        formatUiError(
          "error.emptyTrashFailed",
          "Empty trash failed",
          emptyError,
        ),
      );
    }
  }

  function formatHomeTime(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  async function onExport(payload: ExportRequest): Promise<boolean> {
    if (!activeArtifact) return false;

    let artifactForExport = activeArtifact;
    const hasDraftChanges =
      hasTranscriptDraftChanges ||
      draftSummary !== activeArtifact.summary ||
      draftFaqs !== activeArtifact.faqs;

    if (hasDraftChanges) {
      try {
        const updated = await updateArtifact({
          id: activeArtifact.id,
          optimized_transcript: optimizedTranscriptForPersistence,
          summary: draftSummary,
          faqs: draftFaqs,
        });
        if (updated) {
          artifactForExport = updated;
          upsertArtifact(updated);
          setActiveArtifact(updated);
        }
      } catch (syncError) {
        setError(
          formatUiError(
            "error.copyBeforeExportSync",
            "Could not sync changes before export",
            syncError,
          ),
        );
        return false;
      }
    }

    const destination = await save({
      defaultPath: `${artifactForExport.title.replace(/\s+/g, "_")}.${payload.format}`,
    });

    if (!destination) {
      return false;
    }

    try {
      await exportArtifact({
        id: artifactForExport.id,
        format: payload.format,
        destination_path: destination,
        language,
        style: payload.style,
        options: {
          include_timestamps: payload.options.includeTimestamps,
          grouping: payload.options.grouping,
          include_speaker_names: payload.options.includeSpeakerNames,
        },
        segments: payload.segments,
        content_override: payload.contentOverride,
      });
      setError(null);
      return true;
    } catch (exportError) {
      setError(
        formatUiError("error.exportFailed", "Export failed", exportError),
      );
      throw exportError;
    }
  }

  async function onStartRealtime(): Promise<void> {
    if (!settings) return;

    try {
      setRealtimePreviewState("connecting");
      setRealtimeInputLevels([]);

      const readiness = await withTimeout(
        fetchRealtimeStartReadiness({
          model: settings.transcription.model,
        }),
        8_000,
        t("error.preflightTimedOut", "Preflight timed out."),
      );
      if (!readiness.allowed) {
        setError(
          readiness.message || formatRuntimeNotReadyMessage(runtimeHealth),
        );
        return;
      }

      const sessionTitle = buildLiveSessionTitle();
      setRealtimeFinalLines([]);
      setRealtimePreview("");
      setRealtimeSessionOpen(false);
      await startRealtime({
        model: settings.transcription.model,
        language: settings.transcription.language,
      });
      setDraftTitle(sessionTitle);
      setDraftTranscript("");
      setDraftSummary("");
      setDraftFaqs("");
      setOptimizedTranscriptAvailable(false);
      setTranscriptViewMode("original");
      setActiveArtifact(null);
      setActiveDetailContext(
        buildActiveDetailContext({
          inputPath: null,
          requestedTitle: sessionTitle,
          sourceArtifact: null,
          restoreArtifactOnFailure: false,
        }),
      );
      setDetailMode("transcript");
      setInspectorMode("details");
      setRealtimeSessionOpen(true);
      setRealtimeStartedAtMs(Date.now());
      setSection("detail");
      setError(null);
    } catch (startError) {
      setRealtimeSessionOpen(false);
      setRealtimeStartedAtMs(null);
      setRealtimePreviewState("idle");
      setRealtimeInputLevels([]);
      setError(
        formatUiError(
          "error.realtimeStartFailed",
          "Realtime start failed",
          startError,
        ),
      );
    }
  }

  async function onPauseRealtime(): Promise<void> {
    try {
      await pauseRealtime();
    } catch (pauseError) {
      setError(
        formatUiError(
          "error.realtimePauseFailed",
          "Realtime pause failed",
          pauseError,
        ),
      );
    }
  }

  async function onResumeRealtime(): Promise<void> {
    try {
      await resumeRealtime();
    } catch (resumeError) {
      setError(
        formatUiError(
          "error.realtimeResumeFailed",
          "Realtime resume failed",
          resumeError,
        ),
      );
    }
  }

  async function onStopRealtime(saveResult: boolean): Promise<void> {
    setIsStoppingRealtime(true);
    try {
      const currentLiveTranscript = [...realtimeFinalLines, realtimePreviewText]
        .filter((line) => line.trim().length > 0)
        .join("\n")
        .trim();
      const result = await stopRealtime(
        saveResult,
        draftTitle,
        Math.max(0, Math.round(realtimeElapsedSeconds)),
      );
      if (result.artifact) {
        prependArtifact(result.artifact);
        hydrateDetail(result.artifact);
        setTranscriptViewMode("original");
        setSection("detail");
      } else if (saveResult && currentLiveTranscript) {
        setError(
          t(
            "error.realtimeSaveIncomplete",
            "Live transcription stopped, but the final transcript was not returned. The current live text is still visible here.",
          ),
        );
        return;
      } else {
        setActiveDetailContext(null);
        setSection("home");
      }
      setRealtimePreview("");
      setRealtimeFinalLines([]);
      setRealtimeSessionOpen(false);
      setRealtimeStartedAtMs(null);
      setError(null);
    } catch (stopError) {
      setError(
        formatUiError(
          "error.realtimeStopFailed",
          "Realtime stop failed",
          stopError,
        ),
      );
    } finally {
      setIsStoppingRealtime(false);
    }
  }

  async function onGenerateSummary(revealOnSuccess = true): Promise<void> {
    if (!activeArtifact || isGeneratingSummary) return;
    if (!aiFeaturesAvailable) {
      setError(aiUnavailableReason);
      return;
    }

    setIsGeneratingSummary(true);
    try {
      const answer = await summarizeArtifact(
        buildSummaryArtifactPayload({
          id: activeArtifact.id,
          language: summaryLanguage,
          includeTimestamps: summaryIncludeTimestamps,
          includeSpeakers: summaryIncludeSpeakers,
          sections: summarySections,
          bulletPoints: summaryBulletPoints,
          actionItems: summaryActionItems,
          keyPointsOnly: summaryKeyPointsOnly,
          customPrompt: summaryCustomPrompt,
        }),
      );
      setDraftSummary(answer);
      if (revealOnSuccess) {
        setDetailMode("summary");
      }
      setError(null);
    } catch (summaryError) {
      const code = formatAppErrorCode(summaryError);
      if (code === "missing_ai_provider" || code === "missing_api_key") {
        setError(
          t(
            "error.summaryConfigureProvider",
            "Summary failed: configure an AI provider in Settings > AI Services.",
          ),
        );
      } else {
        setError(t("error.summaryFailed", "Summary failed"));
      }
    } finally {
      setIsGeneratingSummary(false);
    }
  }

  async function onGenerateEmotionAnalysis(
    revealOnSuccess = true,
  ): Promise<void> {
    if (!activeArtifact || isGeneratingEmotionAnalysis) return;
    if (!aiFeaturesAvailable) {
      setError(aiUnavailableReason);
      return;
    }

    setIsGeneratingEmotionAnalysis(true);
    try {
      const result = await analyzeArtifactEmotions(
        buildEmotionAnalysisPayload({
          id: activeArtifact.id,
          language: emotionLanguage,
          includeTimestamps: emotionIncludeTimestamps,
          includeSpeakers: emotionIncludeSpeakers,
          speakerDynamics: speakerDynamicsAvailable
            ? emotionSpeakerDynamics
            : false,
        }),
      );

      const refreshedArtifact = await getArtifact(activeArtifact.id);
      if (refreshedArtifact) {
        upsertArtifact(refreshedArtifact);
        syncArtifactDraftState(refreshedArtifact);
      } else {
        const patchedArtifact: TranscriptArtifact = {
          ...activeArtifact,
          metadata: {
            ...activeArtifact.metadata,
            [EMOTION_ANALYSIS_METADATA_KEY]: JSON.stringify(result),
            [EMOTION_ANALYSIS_GENERATED_AT_METADATA_KEY]:
              new Date().toISOString(),
          },
        };
        upsertArtifact(patchedArtifact);
        syncArtifactDraftState(patchedArtifact);
      }

      setDraftEmotionAnalysis(result);
      if (revealOnSuccess) {
        setDetailMode("emotion");
      }
      setError(null);
    } catch (emotionError) {
      const code = formatAppErrorCode(emotionError);
      if (code === "missing_ai_provider" || code === "missing_api_key") {
        setError(
          t(
            "error.emotionConfigureProvider",
            "Emotion analysis failed: configure an AI provider in Settings > AI Services.",
          ),
        );
      } else {
        const detail = formatAppError(emotionError).trim();
        setError(detail || t("error.emotionFailed", "Emotion analysis failed"));
      }
    } finally {
      setIsGeneratingEmotionAnalysis(false);
    }
  }

  function onJumpToEmotionSegment({
    segmentIndex,
    evidenceText,
    timeLabel,
    startSeconds,
  }: {
    segmentIndex: number;
    evidenceText?: string | null;
    timeLabel?: string | null;
    startSeconds?: number | null;
  }): void {
    const sourceIndex = resolveEmotionSegmentSourceIndex(
      segmentIndex,
      evidenceText,
      timeLabel,
      startSeconds,
    );
    if (sourceIndex === null) {
      setError(
        t(
          "error.segmentTimelineInvalid",
          "Segment timeline metadata is missing or invalid.",
        ),
      );
      return;
    }
    setSelectedSegmentSourceIndex(sourceIndex);
    setDetailMode("segments");
    setInspectorMode("details");
  }

  function onAskEmotionQuestion(prompt: string): void {
    setDetailMode("chat");
    setInspectorMode("details");
    void onSendChat({ prefilledPrompt: prompt, origin: "emotion" });
  }

  async function onImproveText(): Promise<void> {
    if (!activeArtifact || isImprovingText) return;
    if (!aiFeaturesAvailable) {
      setError(aiUnavailableReason);
      return;
    }

    const transcriptToOptimize =
      transcriptViewMode === "original" ? activeRawTranscript : draftTranscript;

    if (transcriptToOptimize.trim() === "") {
      return;
    }

    setIsImprovingText(true);
    try {
      const optimizedText = await optimizeArtifact({
        id: activeArtifact.id,
        text: transcriptToOptimize,
      });
      setDraftTranscript(optimizedText);
      setOptimizedTranscriptAvailable(true);
      setTranscriptViewMode("optimized");
      setError(null);
    } catch (optimizeError) {
      const code = formatAppErrorCode(optimizeError);
      if (code === "missing_ai_provider" || code === "missing_api_key") {
        setError(
          t(
            "error.improveConfigureProvider",
            "Improve text failed: configure an AI provider in Settings > AI Services.",
          ),
        );
      } else {
        setError(t("error.improveFailed", "Improve text failed"));
      }
    } finally {
      setIsImprovingText(false);
    }
  }

  async function onSendChat(options?: {
    prefilledPrompt?: string;
    origin?: ChatMessageOrigin;
  }): Promise<void> {
    if (!activeArtifact || isAskingChat) return;
    if (!aiFeaturesAvailable) {
      setError(aiUnavailableReason);
      return;
    }

    const prompt = (options?.prefilledPrompt ?? chatInput).trim();
    const origin = options?.origin ?? "typed";
    if (!prompt) return;

    if (!options?.prefilledPrompt) {
      setChatInput("");
    }

    const userMessageId = nextChatMessageId("user");
    const assistantMessageId = nextChatMessageId("assistant");
    setIsAskingChat(true);
    setChatHistory((previous) => [
      ...previous,
      {
        id: userMessageId,
        role: "user",
        text: prompt,
        status: "complete",
        origin,
        canCopy: false,
      },
      {
        id: assistantMessageId,
        role: "assistant",
        text: "",
        status: "pending",
        origin,
        canCopy: false,
      },
    ]);

    try {
      const answer = await chatArtifact(
        buildChatArtifactPayload({
          id: activeArtifact.id,
          prompt,
          includeTimestamps: chatIncludeTimestamps,
          includeSpeakers: chatIncludeSpeakers,
        }),
      );
      setChatHistory((previous) =>
        previous.map((message) =>
          message.id === assistantMessageId
            ? {
                ...message,
                text: answer,
                status: "complete",
                canCopy: true,
              }
            : message,
        ),
      );
    } catch (chatError) {
      const code = formatAppErrorCode(chatError);
      const providerMissing =
        code === "missing_ai_provider" || code === "missing_api_key";
      setChatHistory((previous) =>
        previous.map((message) =>
          message.id === assistantMessageId
            ? {
                ...message,
                text: providerMissing
                  ? aiUnavailableReason
                  : t("error.chatFailed", "Chat failed"),
                status: "error",
                canCopy: true,
              }
            : message,
        ),
      );
      setError(
        providerMissing
          ? aiUnavailableReason
          : t("error.chatFailed", "Chat failed"),
      );
    } finally {
      setIsAskingChat(false);
    }
  }

  async function onProvisionModels(): Promise<void> {
    try {
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        progress: null,
        statusMessage: t("provisioning.started", "Provisioning started..."),
      }));
      await provisioningStart(true);
    } catch (provisionError) {
      setProvisioning((previous) => ({
        ...previous,
        running: false,
      }));
      setError(
        formatUiError(
          "error.provisioningFailed",
          "Provisioning failed",
          provisionError,
        ),
      );
    }
  }

  async function onDownloadModel(model: SpeechModel): Promise<void> {
    try {
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        progress: null,
        statusMessage: t(
          "provisioning.downloadingModel",
          "Downloading {model}...",
          { model },
        ),
      }));
      await provisioningDownloadModel({ model, include_coreml: true });
    } catch (downloadError) {
      setProvisioning((previous) => ({
        ...previous,
        running: false,
      }));
      setError(
        formatUiError(
          "error.modelDownloadFailed",
          "Model download failed",
          downloadError,
        ),
      );
    }
  }

  async function onInstallRuntime(force = false): Promise<void> {
    try {
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        progress: null,
        statusMessage: force
          ? t(
              "provisioning.repairingRuntime",
              "Repairing local transcription runtime...",
            )
          : t(
              "provisioning.installingRuntime",
              "Installing local transcription runtime...",
            ),
      }));
      await provisioningInstallRuntime(force);
    } catch (installError) {
      setProvisioning((previous) => ({
        ...previous,
        running: false,
      }));
      setError(
        formatUiError(
          "error.runtimeInstallFailed",
          "Local runtime install failed",
          installError,
        ),
      );
    }
  }

  async function onInstallPyannote(force = false): Promise<void> {
    try {
      pyannoteProvisioningActiveRef.current = true;
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        progress: null,
        statusMessage: force
          ? t(
              "provisioning.repairingPyannote",
              "Repairing pyannote diarization runtime...",
            )
          : t(
              "provisioning.installingPyannote",
              "Installing pyannote diarization runtime...",
            ),
      }));
      await provisioningInstallPyannote(force);
    } catch (installError) {
      pyannoteProvisioningActiveRef.current = false;
      setProvisioning((previous) => ({
        ...previous,
        running: false,
      }));
      setError(
        formatUiError(
          "error.pyannoteInstallFailed",
          "Pyannote install failed",
          installError,
        ),
      );
    }
  }

  async function onCancelProvisioning(): Promise<void> {
    try {
      await provisioningCancel();
      setProvisioning((previous) => ({
        ...previous,
        running: false,
        statusMessage: t("provisioning.cancelled", "Provisioning cancelled"),
      }));
    } catch (cancelError) {
      setError(
        formatUiError(
          "error.provisioningCancelFailed",
          "Provisioning cancel failed",
          cancelError,
        ),
      );
    }
  }

  async function refreshUpdates(
    silent = false,
    shouldCancel?: () => boolean,
  ): Promise<void> {
    if (shouldCancel?.()) {
      return;
    }
    setCheckingUpdates(true);
    setUpdateStatusMessage(null);
    setUpdateDownloadPercent(null);
    setNativeUpdate(null);
    const isDevRuntime = import.meta.env.DEV;
    if (!isDevRuntime) {
      try {
        const native = await checkAppUpdate();
        if (shouldCancel?.()) {
          return;
        }
        if (native) {
          setNativeUpdate(native);
          setUpdateSource("native");
          setUpdateInfo({
            has_update: true,
            current_version: native.currentVersion,
            latest_version: native.version,
            download_url: null,
          });
          try {
            const fallback = await checkUpdates();
            if (shouldCancel?.()) {
              return;
            }
            if (fallback.download_url) {
              setUpdateInfo((previous) =>
                previous
                  ? {
                      ...previous,
                      download_url: fallback.download_url,
                    }
                  : previous,
              );
            }
          } catch {
            // optional GitHub download fallback is best-effort
          }
          return;
        }
      } catch {
        // fallback to GitHub release polling
      }
    }

    try {
      const update = await checkUpdates();
      if (shouldCancel?.()) {
        return;
      }
      setUpdateInfo(update);
      setUpdateSource("github");
    } catch (updateError) {
      if (!silent) {
        setError(
          formatUiError(
            "error.updateCheckFailed",
            "Update check failed",
            updateError,
          ),
        );
      }
    } finally {
      if (!shouldCancel?.()) {
        setCheckingUpdates(false);
      }
    }
  }

  async function syncNativeUpdateForVersion(
    expectedVersion?: string | null,
  ): Promise<void> {
    if (import.meta.env.DEV || !expectedVersion) {
      return;
    }

    try {
      const native = await checkAppUpdate();
      if (native && native.version === expectedVersion) {
        setNativeUpdate(native);
        return;
      }
    } catch {
      // best-effort hydration for secondary windows
    }

    setNativeUpdate(null);
  }

  async function onRefreshUpdates(): Promise<void> {
    setDismissedUpdateVersion(null);
    writeDismissedUpdateVersion(null);
    await refreshUpdates(false);
  }

  async function onInstallUpdate(): Promise<void> {
    if (!nativeUpdate) {
      return;
    }

    setInstallingUpdate(true);
    setUpdateStatusMessage(
      t("settings.general.downloadingUpdate", "Downloading update..."),
    );
    setUpdateDownloadPercent(0);
    try {
      let expectedBytes = 0;
      let downloadedBytes = 0;
      let installStage = "download";
      let lastEvent: string | null = null;
      console.info("[updater] native install started", {
        version: nativeUpdate.version,
      });
      const downloadAndInstallPromise = nativeUpdate.downloadAndInstall((event) => {
        lastEvent = event.event;
        if (event.event === "Started") {
          installStage = "download";
          expectedBytes = event.data.contentLength ?? 0;
          downloadedBytes = 0;
          console.info("[updater] download started", {
            contentLength: expectedBytes,
            version: nativeUpdate.version,
          });
          setUpdateStatusMessage(
            t("settings.general.downloadingUpdate", "Downloading update..."),
          );
          return;
        }
        if (event.event === "Progress") {
          installStage = "download";
          downloadedBytes += event.data.chunkLength;
          if (expectedBytes > 0) {
            const percent = Math.max(
              0,
              Math.min(
                100,
                Math.round((downloadedBytes / expectedBytes) * 100),
              ),
            );
            setUpdateDownloadPercent(percent);
          }
          return;
        }
        if (event.event === "Finished") {
          installStage = "install";
          console.info("[updater] download finished; handing off to installer", {
            downloadedBytes,
            expectedBytes,
            version: nativeUpdate.version,
          });
          setUpdateDownloadPercent(100);
          setUpdateStatusMessage(
            t("settings.general.installingUpdate", "Installing update..."),
          );
        }
      });
      void downloadAndInstallPromise.then(
        () => {
          console.info("[updater] native install resolved", {
            stage: installStage,
            lastEvent,
            version: nativeUpdate.version,
          });
        },
        (error) => {
          console.error("[updater] native install rejected", {
            stage: installStage,
            lastEvent,
            version: nativeUpdate.version,
            error,
          });
        },
      );
      await Promise.race([
        downloadAndInstallPromise,
        new Promise<never>((_, reject) => {
          window.setTimeout(() => {
            reject(
              createNativeUpdateTimeoutError(
                NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
                installStage,
                lastEvent,
              ),
            );
          }, NATIVE_UPDATE_INSTALL_TIMEOUT_MS);
        }),
      ]);

      setUpdateStatusMessage(
        t(
          "settings.general.updateInstalled",
          "Update installed. Restart the app to apply it.",
        ),
      );
      setUpdateInfo((previous) =>
        previous
          ? {
              ...previous,
              has_update: false,
              current_version:
                previous.latest_version ?? previous.current_version,
            }
          : previous,
      );
      setNativeUpdate(null);
      setUpdateStatusMessage(
        t(
          "settings.general.restartingAfterUpdate",
          "Restarting to apply update...",
        ),
      );
      try {
        console.info("[updater] requesting relaunch", {
          version: nativeUpdate.version,
        });
        await relaunch();
        return;
      } catch (relaunchError) {
        console.error("[updater] relaunch failed after install", {
          version: nativeUpdate.version,
          error: relaunchError,
        });
        setError(
          formatUiError(
            "error.updateInstallFailed",
            "Update install failed",
            relaunchError,
          ),
        );
        setUpdateStatusMessage(
          t(
            "settings.general.updateInstalled",
            "Update installed. Restart the app to apply it.",
          ),
        );
      }
    } catch (installError) {
      if (isNativeUpdateTimeoutError(installError)) {
        console.error("[updater] native install timed out", installError);
        setNativeUpdate(null);
        setUpdateStatusMessage(
          t(
            "settings.general.updateInstallTimedOut",
            "Update installation timed out. Retry or download it manually.",
          ),
        );
      } else {
        setUpdateStatusMessage(null);
      }
      setError(
        formatUiError(
          "error.updateInstallFailed",
          "Update install failed",
          installError,
        ),
      );
    } finally {
      setInstallingUpdate(false);
    }
  }

  function dismissUpdateBanner(): void {
    const version = updateInfo?.latest_version ?? null;
    setDismissedUpdateVersion(version);
    writeDismissedUpdateVersion(version);
  }

  async function onRunPromptTest(): Promise<void> {
    if (!promptDraft) return;

    setPromptTest((current) => ({ ...current, running: true }));
    try {
      const result = await testPromptTemplate({
        input: promptTest.input,
        task: promptBindingTask,
        prompt_override: promptDraft.body,
        model_override: settings?.ai.providers.gemini.model,
        language: settings?.transcription.language,
      });

      setPromptTest((current) => ({
        ...current,
        output: result.output,
        running: false,
      }));
    } catch (error) {
      setPromptTest((current) => ({
        ...current,
        output: t("error.promptTestFailed", "Prompt test failed"),
        running: false,
      }));
    }
  }

  async function onSavePromptTemplate(): Promise<void> {
    if (!settings || !promptDraft) return;

    const updatedTemplates = settings.prompts.templates.map((template) =>
      template.id === promptDraft.id
        ? {
            ...promptDraft,
            updated_at: String(Date.now()),
          }
        : template,
    );

    await patchSettings((current) => ({
      ...current,
      prompts: {
        ...current.prompts,
        templates: updatedTemplates,
      },
    }));
  }

  async function onResetPrompts(): Promise<void> {
    try {
      await resetPromptTemplates();
      await refreshSettingsFromDisk();
    } catch (error) {
      setError(
        formatUiError(
          "error.resetPromptsFailed",
          "Could not reset prompts",
          error,
        ),
      );
    }
  }

  function renderHome(): JSX.Element {
    const homeAudioInputPath =
      preparedImportTrimDraft?.path ?? selectedFile ?? null;
    const homeAudioIsTrimmed = Boolean(preparedImportTrimDraft);

    const renderHomeTree = (
      artifact: GroupedArtifact,
      depth = 0,
    ): React.ReactNode => {
      return (
        <React.Fragment key={artifact.id}>
          <article
            className={`history-item home-history-item${depth > 0 ? " history-child-item" : ""}${selectedArtifactIdSet.has(artifact.id) ? " selected" : ""}`}
          >
            <button
              className="home-history-main"
              onClick={() => {
                if (isSelectionMode) {
                  toggleArtifactSelection(artifact.id);
                  return;
                }
                void onOpenArtifact(artifact.id);
              }}
            >
              <span className="history-audio-dot">
                <AudioLines size={12} />
              </span>
              <div className="home-history-copy">
                <div
                  className="home-history-head"
                  style={{ display: "flex", alignItems: "center" }}
                >
                  {artifact.children && artifact.children.length > 0 && (
                    <button
                      className="tree-expand-button"
                      onClick={(e) => toggleArtifactExpansion(artifact.id, e)}
                      title={
                        expandedArtifactIds.has(artifact.id)
                          ? t(
                              "history.collapseChildTrims",
                              "Collapse child trims",
                            )
                          : t("history.expandChildTrims", "Expand child trims")
                      }
                    >
                      {expandedArtifactIds.has(artifact.id) ? (
                        <ChevronDown size={14} />
                      ) : (
                        <ChevronRight size={14} />
                      )}
                    </button>
                  )}
                  <strong>
                    <HighlightMatch text={artifact.title} search={search} />
                  </strong>
                  <span
                    className="history-inline-time"
                    style={{ marginLeft: "auto" }}
                  >
                    {formatHomeTime(artifact.updated_at)}
                  </span>
                </div>
                <p
                  className="history-preview"
                  style={
                    artifact.children && artifact.children.length > 0
                      ? { paddingLeft: 22 }
                      : {}
                  }
                >
                  <HighlightMatch
                    text={previewSnippet(
                      artifact.optimized_transcript || artifact.raw_transcript,
                      210,
                    )}
                    search={search}
                  />
                </p>
              </div>
            </button>
            <div className="home-history-actions">
              <label
                className="home-history-select"
                title={
                  depth > 0
                    ? t(
                        "history.selectTrimTranscription",
                        "Select trim transcription",
                      )
                    : t("history.selectTranscription", "Select transcription")
                }
                onClick={(e) => e.stopPropagation()}
              >
                <input
                  type="checkbox"
                  checked={selectedArtifactIdSet.has(artifact.id)}
                  onChange={() => toggleArtifactSelection(artifact.id)}
                />
              </label>
              {!isSelectionMode ? (
                <button
                  className="icon-button danger-icon-button home-history-delete"
                  onClick={(event) => {
                    event.stopPropagation();
                    void onDeleteArtifact(artifact);
                  }}
                  title={t("history.moveToTrashTitle", "Move to trash")}
                  aria-label={t(
                    "history.moveToTrashAria",
                    "Move {title} to trash",
                    { title: artifact.title },
                  )}
                >
                  <Trash2 size={14} />
                </button>
              ) : null}
            </div>
          </article>

          {artifact.children &&
            artifact.children.length > 0 &&
            expandedArtifactIds.has(artifact.id) && (
              <div className="history-children-list">
                {artifact.children.map((child) =>
                  renderHomeTree(child, depth + 1),
                )}
              </div>
            )}
        </React.Fragment>
      );
    };

    return (
      <div className="view-body home-view">
        <section className="main-input-bar">
          <input
            type="text"
            placeholder={t("home.audioFilePlaceholder", "Audio file")}
            value={selectedFile ? fileLabel(selectedFile) : ""}
            readOnly
          />
          <button
            className="icon-button"
            onClick={() => void onPickFile()}
            title={t("home.openLocalFile", "Open Local File")}
          >
            <Upload size={16} />
          </button>
        </section>

        <div className="quick-actions-grid">
          <button
            className="quick-action"
            onClick={() => void onStartTranscription()}
            disabled={!canStartFileTranscription}
          >
            <FileAudio size={18} strokeWidth={2.5} />
            {isStarting
              ? t("home.starting", "Starting...")
              : homeAudioIsTrimmed
                ? t(
                    "home.startTrimmedTranscription",
                    "Start Trimmed Transcription",
                  )
                : t("home.startTranscription", "Start Transcription")}
          </button>
          <button
            className="quick-action"
            onClick={() => void onStartRealtime()}
            disabled={!canStartRealtime}
          >
            <Mic size={18} strokeWidth={2.5} />
            {t("home.startLive", "Start Live")}
          </button>
          <button
            className="quick-action"
            onClick={() => void onStopRealtime(true)}
            disabled={realtimeState === "idle" || isStoppingRealtime}
          >
            <Radio size={18} strokeWidth={2.5} />
            {isStoppingRealtime
              ? t("home.stopping", "Stopping...")
              : t("home.stopLiveSave", "Stop Live & Save")}
          </button>
          <button className="quick-action" onClick={() => setSection("queue")}>
            <ListChecks size={18} strokeWidth={2.5} />
            {t("home.queue", "Queue")}
          </button>
          <button
            className="quick-action"
            onClick={() => setSection("history")}
          >
            <HistoryIcon size={18} strokeWidth={2.5} />
            {t("home.history", "History")}
          </button>
        </div>

        {homeAudioInputPath ? (
          <section className="panel-card home-audio-player-card">
            <div className="detail-audio-stack">
              <div className="detail-audio-player-group">
                <AudioPlayer
                  inputPath={homeAudioInputPath}
                  trimEnabled
                  onTrimApplied={(trimmedAudio, regions) => {
                    if (!selectedFile) {
                      return;
                    }
                    const sourceLabel = fileLabel(selectedFile);
                    setPreparedImportTrimDraft({
                      path: trimmedAudio.path,
                      durationSeconds: trimmedAudio.duration_seconds,
                      fileSizeBytes: trimmedAudio.file_size_bytes,
                      sourcePath: selectedFile,
                      title: buildTrimArtifactTitle(sourceLabel, regions),
                      regions: [...regions],
                    });
                  }}
                />
              </div>
            </div>
          </section>
        ) : null}

        <section className="panel-card">
          {isSelectionMode ? (
            <div className="history-selection-toolbar">
              <button
                className="secondary-button"
                onClick={() => setSelectedArtifactIds([])}
              >
                {t("action.cancel", "Cancel")}
              </button>
              <button
                className="secondary-button"
                onClick={selectAllVisibleArtifacts}
                disabled={homeVisibleArtifactIds.length === 0}
              >
                {t("selection.selectAll", "Select all")}
              </button>
              <button
                className="secondary-button history-action-danger"
                onClick={() => void onDeleteSelectedArtifacts()}
                disabled={selectedArtifactIds.length === 0}
              >
                {t("selection.deleteSelected", "Delete selected")} (
                {selectedArtifactIds.length})
              </button>
            </div>
          ) : null}
          {groupedRecentArtifacts.length === 0 ? (
            <div className="center-empty compact">
              <h3>{t("home.noTranscripts")}</h3>
              <p>{t("home.noTranscriptsDesc")}</p>
            </div>
          ) : (
            <div className="history-groups">
              {groupedRecentArtifacts.map((group) => (
                <section key={group.label} className="history-group">
                  <h3 className="history-group-label">{group.label}</h3>
                  <div
                    className={`history-list ${isSelectionMode ? "selection-active" : ""}`}
                  >
                    {group.items.map((artifact) => renderHomeTree(artifact))}
                  </div>
                </section>
              ))}
            </div>
          )}
        </section>
      </div>
    );
  }

  function renderQueue(): JSX.Element {
    return (
      <div className="view-body">
        <div className="view-toolbar">
          <h2>{t("queue.title")}</h2>
          <div className="toolbar-actions">
            <button
              className="secondary-button"
              onClick={() => setQueueItems([])}
            >
              {t("queue.clearFinished", "Clear Finished")}
            </button>
            <button
              className="secondary-button"
              onClick={() => void onCancel()}
              disabled={!activeJobId}
            >
              {t("queue.cancelActiveJob", "Cancel Active Job")}
            </button>
          </div>
        </div>

        {queueActiveItems.length === 0 ? (
          <div className="center-empty">
            <div className="center-empty-icon">
              <ListChecks size={28} />
            </div>
            <h3>{t("queue.noActive")}</h3>
            <p>{t("queue.noActiveDesc")}</p>
          </div>
        ) : (
          <div className="queue-list">
            {queueActiveItems.map((item) => {
              const isQueuedPlaceholder = isQueuedTranscriptionJobId(
                item.job_id,
              );
              const queuedStart = queuedTranscriptionStarts.find(
                (entry) => entry.queueId === item.job_id,
              );
              const pendingContext = pendingTranscriptionContextRef.current.get(
                item.job_id,
              );
              const queueItemTitle =
                queuedStart?.title ??
                (queuedStart?.inputPath
                  ? fileLabel(queuedStart.inputPath)
                  : undefined) ??
                pendingContext?.title ??
                (pendingContext?.inputPath
                  ? fileLabel(pendingContext.inputPath)
                  : undefined) ??
                (item.job_id === activeJobId && activeJobTitle
                  ? activeJobTitle
                  : t("queue.activeJobFallback", "Transcription in progress"));
              const displayPercentage =
                item.job_id === activeJobId
                  ? runningJobPercentage
                  : percentageFromJobProgress(item);

              return (
                <article
                  key={item.job_id}
                  className={`queue-card ${isQueuedPlaceholder ? "" : "queue-card-clickable"}`}
                  onClick={
                    isQueuedPlaceholder
                      ? undefined
                      : () => onFocusQueueJob(item)
                  }
                  role={isQueuedPlaceholder ? undefined : "button"}
                  tabIndex={isQueuedPlaceholder ? undefined : 0}
                  onKeyDown={
                    isQueuedPlaceholder
                      ? undefined
                      : (event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            onFocusQueueJob(item);
                          }
                        }
                  }
                >
                  <div className="queue-card-head">
                    <strong>{queueItemTitle}</strong>
                    <span className="queue-stage">
                      <ProgressRing percentage={displayPercentage} size={18} />
                      <small>{formatJobStageLabel(item.stage)}</small>
                    </span>
                  </div>
                  <p>{item.message}</p>
                  <div className="queue-progress">
                    <div style={{ width: `${displayPercentage}%` }} />
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>
    );
  }

  function renderHistory(): JSX.Element {
    const renderHistoryTree = (
      artifact: GroupedArtifact,
      depth = 0,
    ): React.ReactNode => {
      return (
        <React.Fragment key={artifact.id}>
          <article
            className={`history-item ${depth > 0 ? "history-child-item" : ""}${selectedArtifactIdSet.has(artifact.id) ? " selected" : ""}`}
          >
            <button
              className="history-main history-main-rich"
              onClick={() => {
                if (isSelectionMode) {
                  toggleArtifactSelection(artifact.id);
                  return;
                }
                void onOpenArtifact(artifact.id);
              }}
            >
              <span className="history-audio-dot">
                <AudioLines size={12} />
              </span>
              <div className="history-main-copy">
                <div
                  className="home-history-head"
                  style={{ display: "flex", alignItems: "center" }}
                >
                  {artifact.children && artifact.children.length > 0 && (
                    <button
                      className="tree-expand-button"
                      onClick={(e) => toggleArtifactExpansion(artifact.id, e)}
                      title={
                        expandedArtifactIds.has(artifact.id)
                          ? t(
                              "history.collapseChildTrims",
                              "Collapse child trims",
                            )
                          : t("history.expandChildTrims", "Expand child trims")
                      }
                    >
                      {expandedArtifactIds.has(artifact.id) ? (
                        <ChevronDown size={14} />
                      ) : (
                        <ChevronRight size={14} />
                      )}
                    </button>
                  )}
                  <strong>
                    <HighlightMatch text={artifact.title} search={search} />
                  </strong>
                  <span
                    className="history-inline-time"
                    style={{ marginLeft: "auto" }}
                  >
                    {formatHomeTime(artifact.updated_at)}
                  </span>
                </div>
                <p
                  className="history-preview"
                  style={
                    artifact.children && artifact.children.length > 0
                      ? { paddingLeft: 22 }
                      : {}
                  }
                >
                  <HighlightMatch
                    text={previewSnippet(
                      artifact.optimized_transcript || artifact.raw_transcript,
                      220,
                    )}
                    search={search}
                  />
                </p>
              </div>
            </button>
            <div className="history-actions">
              <label
                className="home-history-select history-select"
                title={
                  depth > 0
                    ? t(
                        "history.selectTrimTranscription",
                        "Select trim transcription",
                      )
                    : t("history.selectTranscription", "Select transcription")
                }
                onClick={(e) => e.stopPropagation()}
              >
                <input
                  type="checkbox"
                  checked={selectedArtifactIdSet.has(artifact.id)}
                  onChange={() => toggleArtifactSelection(artifact.id)}
                />
              </label>
              {!isSelectionMode ? (
                <>
                  <button
                    className="secondary-button history-action-button"
                    onClick={() => void onRenameArtifact(artifact)}
                  >
                    <Pencil size={14} />
                    {t("history.rename", "Rename")}
                  </button>
                  <button
                    className="icon-button danger-icon-button history-inline-delete"
                    onClick={(event) => {
                      event.stopPropagation();
                      void onDeleteArtifact(artifact);
                    }}
                    title={t("history.moveToTrashTitle", "Move to trash")}
                    aria-label={t(
                      "history.moveToTrashAria",
                      "Move {title} to trash",
                      { title: artifact.title },
                    )}
                  >
                    <Trash2 size={14} />
                  </button>
                </>
              ) : null}
            </div>
          </article>

          {artifact.children &&
            artifact.children.length > 0 &&
            expandedArtifactIds.has(artifact.id) && (
              <div className="history-children-list">
                {artifact.children.map((child) =>
                  renderHistoryTree(child, depth + 1),
                )}
              </div>
            )}
        </React.Fragment>
      );
    };

    return (
      <div className="view-body history-view">
        {isSelectionMode ? (
          <div className="history-selection-toolbar">
            <button
              className="secondary-button"
              onClick={() => setSelectedArtifactIds([])}
            >
              {t("selection.cancel", "Cancel")}
            </button>
            <button
              className="secondary-button"
              onClick={selectAllVisibleArtifacts}
              disabled={historyVisibleArtifactIds.length === 0}
            >
              {t("selection.selectAll", "Select all")}
            </button>
            <button
              className="secondary-button history-action-danger"
              onClick={() => void onDeleteSelectedArtifacts()}
              disabled={selectedArtifactIds.length === 0}
            >
              {t("selection.deleteSelected", "Delete selected")} (
              {selectedArtifactIds.length})
            </button>
          </div>
        ) : null}
        {groupedHistoryArtifacts.length === 0 ? (
          <div className="center-empty">
            <div className="center-empty-icon">
              <Clock3 size={28} />
            </div>
            <h3>{t("history.noTranscriptions")}</h3>
          </div>
        ) : (
          <div className="history-groups">
            {groupedHistoryArtifacts.map((group) => (
              <section key={group.label} className="history-group">
                <h3 className="history-group-label">{group.label}</h3>
                <div
                  className={`history-list ${isSelectionMode ? "selection-active" : ""}`}
                >
                  {group.items.map((artifact) => renderHistoryTree(artifact))}
                </div>
              </section>
            ))}
          </div>
        )}
      </div>
    );
  }

  function renderDeletedHistory(): JSX.Element {
    return (
      <div className="view-body">
        {filteredDeletedArtifacts.length === 0 ? (
          <div className="center-empty">
            <div className="center-empty-icon">
              <Trash2 size={28} />
            </div>
            <h3>{t("deleted.empty")}</h3>
            <p>{t("deleted.emptyDesc")}</p>
          </div>
        ) : (
          <div className="history-list">
            {filteredDeletedArtifacts.map((artifact) => (
              <article
                key={artifact.id}
                className="history-item deleted-history-item"
              >
                <div className="history-main">
                  <div>
                    <strong>
                      <HighlightMatch text={artifact.title} search={search} />
                    </strong>
                    <p>
                      <HighlightMatch
                        text={previewSnippet(
                          artifact.optimized_transcript ||
                            artifact.raw_transcript,
                          220,
                        )}
                        search={search}
                      />
                    </p>
                  </div>
                  <small>{formatDate(artifact.updated_at)}</small>
                </div>
                <div className="history-actions">
                  <span className="kind-chip">
                    {artifact.kind === "realtime"
                      ? t("history.live", "Live")
                      : t("history.file", "File")}
                  </span>
                  <button
                    className="secondary-button history-action-button"
                    onClick={() =>
                      void onRestoreArtifactsWithConsent([artifact.id])
                    }
                  >
                    {t("deleted.restore", "Restore")}
                  </button>
                  <button
                    className="secondary-button history-action-button history-action-danger"
                    onClick={() =>
                      void onHardDeleteArtifactsWithConsent([artifact.id])
                    }
                  >
                    <Trash2 size={14} />
                    {t("deleted.deletePermanently", "Delete Permanently")}
                  </button>
                </div>
              </article>
            ))}
          </div>
        )}
      </div>
    );
  }
  function renderDetailMain(): JSX.Element {
    if (isRealtimeDetailActive) {
      return (
        <div className="transcript-shell realtime-transcript-shell">
          <div
            className="transcript-container realtime-transcript-container"
            style={{ position: "relative", height: "100%" }}
          >
            <textarea
              className="detail-editor realtime-detail-editor"
              value={realtimeTranscriptDisplayText}
              readOnly
              style={{
                fontSize: `${fontSize}px`,
                width: "100%",
                height: "100%",
              }}
            />
            {!realtimeTranscriptDisplayText ? (
              <div className="center-empty compact realtime-detail-empty">
                <h3>{t("realtime.noTranscript")}</h3>
                <p>{t("realtime.noTranscriptDesc")}</p>
              </div>
            ) : null}
          </div>
        </div>
      );
    }

    if (!activeArtifact) {
      if (focusedJobId) {
        if (!activeJobPreviewText) {
          return <LoadingAnimation />;
        }
        return (
          <TranscriptionPreview
            text={activeJobPreviewText}
            fontSize={fontSize}
            previewRef={activeJobPreviewTextareaRef}
          />
        );
      }

      return (
        <div className="center-empty">
          <h3>{t("detail.selectTranscript")}</h3>
        </div>
      );
    }

    if (detailMode === "summary") {
      if (isGeneratingSummary) {
        return (
          <LoadingAnimation
            icon={Sparkles}
            title={t("detail.summarizing", "Summarizing...")}
            description={t(
              "detail.summaryGenerating",
              "Generating a summary from your transcript...",
            )}
            variant="summarizing"
          />
        );
      }

      if (!draftSummary) {
        return (
          <div className="detail-empty">
            <div className="center-empty-icon">
              <Sparkles size={28} />
            </div>
            <h2>{t("detail.aiSummaryTitle")}</h2>
            <p>{t("detail.summaryEmptyDesc")}</p>
          </div>
        );
      }

      return (
        <textarea
          className="detail-editor summary-editor"
          value={draftSummary}
          onChange={(event) => setDraftSummary(event.target.value)}
        />
      );
    }

    if (detailMode === "emotion") {
      if (isGeneratingEmotionAnalysis) {
        return (
          <LoadingAnimation
            icon={HeartPulse}
            title={t("emotion.generating", "Analyzing emotions...")}
            description={t(
              "emotion.generatingDesc",
              "Building an emotional map of your transcript...",
            )}
            variant="summarizing"
          />
        );
      }

      if (!draftEmotionAnalysis) {
        return (
          <div className="detail-empty">
            <div className="center-empty-icon">
              <HeartPulse size={28} />
            </div>
            <h2>{t("detail.emotion", "Emotion Analysis")}</h2>
            <p>
              {t(
                "emotion.emptyDesc",
                "Generate a structured emotional reading from the right panel, then use it to reflect or continue in chat.",
              )}
            </p>
          </div>
        );
      }

      const timelineEntries = draftEmotionAnalysis.timeline;
      const timelineStart = timelineEntries[0] ?? null;
      const timelineEnd =
        timelineEntries.length > 1
          ? timelineEntries[timelineEntries.length - 1]
          : null;
      const canCollapseTimeline = timelineEntries.length > 2;
      const emotionNarrative = normalizeEmotionNarrative(
        draftEmotionAnalysis.narrative_markdown,
      );

      return (
        <div className="emotion-view">
          <section className="emotion-section">
            <div className="emotion-section-header">
              <h3>{t("emotion.overview", "General reading")}</h3>
              <div className="emotion-chip-row">
                {draftEmotionAnalysis.overview.primary_emotions.map(
                  (emotion) => (
                    <span key={emotion} className="kind-chip">
                      {emotion}
                    </span>
                  ),
                )}
              </div>
            </div>
            <p className="emotion-overview-copy">
              {draftEmotionAnalysis.overview.emotional_arc}
            </p>
            {draftEmotionAnalysis.overview.speaker_dynamics ? (
              <p className="emotion-overview-copy">
                {draftEmotionAnalysis.overview.speaker_dynamics}
              </p>
            ) : null}
            {draftEmotionAnalysis.overview.confidence_note ? (
              <p className="muted">
                {draftEmotionAnalysis.overview.confidence_note}
              </p>
            ) : null}
          </section>

          {emotionNarrative ? (
            <section className="emotion-section">
              <div className="emotion-section-header">
                <h3>{t("emotion.reading", "Narrative reading")}</h3>
              </div>
              <div className="emotion-narrative">{emotionNarrative}</div>
            </section>
          ) : null}

          <section className="emotion-section">
            <div className="emotion-section-header">
              <div className="emotion-title-row">
                <h3>{t("emotion.timeline", "Emotional arc")}</h3>
                <InlineInfoHint
                  label={t("emotion.timeline", "Emotional arc")}
                  description={t(
                    "emotion.timelineHelp",
                    "This timeline breaks the conversation into segments so you can spot where the tone changes, who is speaking, and which moments feel more charged or flat.",
                  )}
                />
              </div>
            </div>
            <p className="emotion-section-help">
              {t(
                "emotion.timelineHelpShort",
                "Use this section to see where the discussion stays neutral, becomes tense, or shifts toward relief or confidence.",
              )}
            </p>
            {emotionTimelineExpanded || !canCollapseTimeline ? (
              <>
                <div className="emotion-card-grid">
                  {timelineEntries.map((entry) =>
                    renderEmotionTimelineCard(entry),
                  )}
                </div>
                {canCollapseTimeline ? (
                  <button
                    className="secondary-button emotion-timeline-toggle"
                    onClick={() => setEmotionTimelineExpanded(false)}
                  >
                    {t("emotion.hideTimeline", "Hide full timeline")}
                  </button>
                ) : null}
              </>
            ) : (
              <div className="emotion-timeline-preview">
                {timelineStart
                  ? renderEmotionTimelineCard(timelineStart)
                  : null}
                <div className="emotion-timeline-connector" aria-hidden="true">
                  <div className="emotion-timeline-connector-line" />
                  <span className="emotion-timeline-connector-arrow">→</span>
                  <p>
                    {t(
                      "emotion.timelineCollapsedSummary",
                      "Showing the start and end of the conversation.",
                    )}
                  </p>
                  <small>
                    {t(
                      "emotion.timelineCollapsedCount",
                      "{count} moments hidden in between",
                      {
                        count: Math.max(
                          0,
                          timelineEntries.length - (timelineEnd ? 2 : 1),
                        ),
                      },
                    )}
                  </small>
                </div>
                {timelineEnd ? renderEmotionTimelineCard(timelineEnd) : null}
                <button
                  className="secondary-button emotion-timeline-toggle"
                  onClick={() => setEmotionTimelineExpanded(true)}
                >
                  {t("emotion.showTimeline", "Show full timeline")}
                </button>
              </div>
            )}
          </section>

          <section className="emotion-section">
            <div className="emotion-section-header">
              <h3>{t("emotion.semanticMap", "Semantic clusters")}</h3>
            </div>
            <div className="emotion-card-grid">
              {draftEmotionAnalysis.semantic_map.clusters.map((cluster) => (
                <article key={cluster.id} className="emotion-card">
                  <strong>{cluster.label}</strong>
                  <p>{cluster.summary}</p>
                  <div className="emotion-chip-row">
                    {cluster.node_ids.slice(0, 6).map((nodeId) => (
                      <span key={nodeId} className="kind-chip">
                        {nodeId.replace(/^.*:/, "")}
                      </span>
                    ))}
                  </div>
                  <div className="emotion-card-actions">
                    <button
                      className="secondary-button"
                      disabled={cluster.segment_indices.length === 0}
                      onClick={() => {
                        const target = cluster.segment_indices[0];
                        if (target !== undefined) {
                          onJumpToEmotionSegment({ segmentIndex: target });
                        }
                      }}
                    >
                      {t("emotion.jump", "Jump to segment")}
                    </button>
                    <button
                      className="secondary-button"
                      onClick={() =>
                        onAskEmotionQuestion(
                          `How does the semantic cluster "${cluster.label}" connect to the emotional dynamics in this transcript?`,
                        )
                      }
                    >
                      {t("emotion.ask", "Ask in chat")}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </section>

          <section className="emotion-section">
            <div className="emotion-section-header">
              <h3>{t("emotion.evidence", "Evidence and bridges")}</h3>
            </div>
            <div className="emotion-card-grid">
              {draftEmotionAnalysis.bridges.map((bridge, index) => (
                <article
                  key={`${bridge.from_segment_index}-${bridge.to_segment_index}-${index}`}
                  className="emotion-card"
                >
                  <strong>{bridge.bridge_theme}</strong>
                  <p>{bridge.reason}</p>
                  <small>
                    {t("emotion.bridgeRange", "Segments")}{" "}
                    {bridge.from_segment_index + 1} →{" "}
                    {bridge.to_segment_index + 1}
                  </small>
                  <div className="emotion-chip-row">
                    {bridge.shared_keywords.map((keyword) => (
                      <span key={keyword} className="kind-chip">
                        {keyword}
                      </span>
                    ))}
                  </div>
                  <div className="emotion-card-actions">
                    <button
                      className="secondary-button"
                      onClick={() =>
                        onJumpToEmotionSegment({
                          segmentIndex: bridge.from_segment_index,
                        })
                      }
                    >
                      {t("emotion.jump", "Jump to segment")}
                    </button>
                    <button
                      className="secondary-button"
                      onClick={() =>
                        onAskEmotionQuestion(
                          `Help me understand why the transcript reconnects around "${bridge.bridge_theme}" between segments ${bridge.from_segment_index + 1} and ${bridge.to_segment_index + 1}.`,
                        )
                      }
                    >
                      {t("emotion.ask", "Ask in chat")}
                    </button>
                  </div>
                </article>
              ))}
            </div>
          </section>

          <section className="emotion-section">
            <div className="emotion-section-header">
              <h3>{t("emotion.reflection", "Reflection prompts")}</h3>
            </div>
            <div className="emotion-reflection-list">
              {draftEmotionAnalysis.reflection_prompts.map((prompt, index) => (
                <button
                  key={`emotion-reflection-${index}`}
                  className="secondary-button emotion-reflection-button"
                  onClick={() => onAskEmotionQuestion(prompt)}
                >
                  {prompt}
                </button>
              ))}
            </div>
          </section>
        </div>
      );
    }

    if (detailMode === "chat") {
      return (
        <div className="chat-view">
          <ChatConversation
            messages={chatHistory}
            copiedMessageId={copiedChatMessageId}
            emptyTitle={t("detail.aiChatTitle", "AI Chat")}
            emptyDescription={t(
              "detail.chatEmptyDesc",
              "Ask questions on the current transcript.",
            )}
            scrollToLatestLabel={t(
              "detail.chatScrollToBottom",
              "Scroll to latest message",
            )}
            pendingLabel={t("detail.thinking", "Thinking...")}
            pendingDescription={t(
              "detail.thinkingDesc",
              "AI is analyzing your transcript and generating a response...",
            )}
            promptOriginLabel={t("detail.chatOriginPrompt", "Prompt")}
            emotionOriginLabel={t(
              "detail.chatOriginEmotion",
              "Emotion insight",
            )}
            copyLabel={t("detail.chatCopyExchange", "Copy question and answer")}
            copiedLabel={t("detail.chatCopied", "Copied")}
            onCopyMessage={(messageId) => {
              void onCopyChatExchange(messageId);
            }}
          />
          <ChatComposer
            inputValue={chatInput}
            inputPlaceholder={t(
              "detail.chatPlaceholder",
              "Chat with your transcript...",
            )}
            suggestionsTitle={t("detail.chatSuggestions", "Quick prompts")}
            serviceSelectValue={activeAiServiceSelectValue}
            serviceOptions={aiServiceSelectOptions}
            promptSuggestions={chatPromptSuggestions}
            submitLabel={t("detail.submitChat", "Submit Chat")}
            disabled={!aiFeaturesAvailable}
            submitDisabled={
              !aiFeaturesAvailable ||
              isAskingChat ||
              chatInput.trim().length === 0
            }
            footerMessage={!aiFeaturesAvailable ? aiUnavailableReason : null}
            onInputChange={setChatInput}
            onSelectService={(value) => {
              void onSelectAiService(value);
            }}
            onSelectPrompt={(suggestion) => {
              void onSendChat({
                prefilledPrompt: suggestion.body,
                origin: "prompt",
              });
            }}
            onSubmit={() => {
              void onSendChat();
            }}
          />
        </div>
      );
    }

    if (detailMode === "segments") {
      return (
        <div className="segments-view">
          {detailSegments.length === 0 ? (
            <p className="muted">{t("detail.noSegments")}</p>
          ) : null}
          {detailSegments.map((segment, index) => (
            <article
              ref={(node) => {
                setSegmentElementRef(segment.sourceIndex, node);
              }}
              className={`segment-row ${
                selectedSegmentSourceIndex === segment.sourceIndex
                  ? "selected"
                  : ""
              } ${segmentContextMenu?.sourceIndex === segment.sourceIndex ? "context-open" : ""}`}
              key={`${segment.time}-${index}`}
              onClick={() => setSelectedSegmentSourceIndex(segment.sourceIndex)}
              onContextMenu={(event) =>
                openSegmentContextMenu(event, segment.sourceIndex)
              }
              role="button"
              tabIndex={0}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  setSelectedSegmentSourceIndex(segment.sourceIndex);
                }
              }}
            >
              {segment.speakerLabel ? (
                <div style={{ marginBottom: "8px" }}>
                  <span
                    className="kind-chip speaker-kind-chip"
                    style={buildSpeakerAccentStyle(
                      resolveSpeakerColor({
                        speakerId: segment.speakerId,
                        speakerLabel: segment.speakerLabel,
                        colorMap: speakerColorMap,
                      }),
                    )}
                  >
                    {segment.speakerLabel}
                  </span>
                </div>
              ) : null}
              <p style={{ fontSize: `${fontSize}px` }}>{segment.line}</p>
              <small>{segment.time}</small>
            </article>
          ))}
        </div>
      );
    }

    if (isImprovingText) {
      return (
        <LoadingAnimation
          icon={Sparkles}
          title={t("detail.improvingText", "Improving Text...")}
          description={t(
            "detail.improvingTextDesc",
            "AI is correcting punctuation and likely transcription mistakes...",
          )}
          variant="transcribing"
        />
      );
    }

    const transcriptSpeakerBanner = (() => {
      if (!activeArtifact || !artifactDiarizationUiState) {
        return null;
      }

      if (artifactDiarizationUiState.kind === "speakers_detected") {
        return (
          <div className="transcript-speaker-banner">
            <div className="transcript-speaker-banner-copy">
              <strong>
                {t("detail.speakersDetected", "{count} speakers detected", {
                  count: artifactDiarizationUiState.speakerCount,
                })}
              </strong>
              <span>
                {formatSpeakerSummary(artifactDiarizationUiState.speakerLabels)}
              </span>
            </div>
            <div className="transcript-speaker-banner-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => setDetailMode("segments")}
              >
                {t("inspector.openSegments", "Open Segments")}
              </button>
            </div>
          </div>
        );
      }

      if (artifactDiarizationUiState.kind === "failed") {
        const diarizationError =
          artifactDiarizationUiState.error ??
          t(
            "detail.diarizationFailedFallback",
            "Speaker diarization failed after transcription.",
          );
        return (
          <div className="transcript-speaker-banner is-warning">
            <div className="transcript-speaker-banner-copy">
              <strong>
                {t(
                  "detail.diarizationFailed",
                  "Speaker diarization failed for this transcript.",
                )}
              </strong>
              <span>{diarizationError}</span>
            </div>
            <div className="transcript-speaker-banner-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() =>
                  void onOpenStandaloneSettingsWindow(
                    shouldOfferLocalModelsCta(diarizationError)
                      ? "local_models"
                      : "transcription",
                  )
                }
              >
                {shouldOfferLocalModelsCta(diarizationError)
                  ? t("action.openLocalModels", "Open Local Models")
                  : t(
                      "action.openTranscriptionDefaults",
                      "Open Transcription Defaults",
                    )}
              </button>
            </div>
          </div>
        );
      }

      if (artifactDiarizationUiState.kind === "no_speakers_detected") {
        return (
          <div className="transcript-speaker-banner">
            <div className="transcript-speaker-banner-copy">
              <strong>
                {t(
                  "detail.noSpeakersDetected",
                  "Speaker diarization completed, but no speaker labels were assigned.",
                )}
              </strong>
              <span>
                {t(
                  "detail.noSpeakersDetectedHint",
                  "Open Segments to inspect the timeline or assign speakers manually.",
                )}
              </span>
            </div>
            <div className="transcript-speaker-banner-actions">
              <button
                type="button"
                className="secondary-button"
                onClick={() => setDetailMode("segments")}
              >
                {t("inspector.openSegments", "Open Segments")}
              </button>
            </div>
          </div>
        );
      }

      return null;
    })();

    return (
      <div className="transcript-shell">
        {transcriptSpeakerBanner}
        <div
          className="transcript-container"
          style={{ position: "relative", height: "100%" }}
        >
          {search.trim() ? (
            <div
              className="detail-editor highlight-layer"
              style={{
                fontSize: `${fontSize}px`,
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: "100%",
                pointerEvents: "none",
                whiteSpace: "pre-wrap",
                wordWrap: "break-word",
                color: "transparent",
              }}
            >
              <HighlightMatch text={visibleTranscript} search={search} />
            </div>
          ) : null}
          {showingConfidenceTranscript && confidenceTranscriptDocument ? (
            <ConfidenceTranscript
              document={confidenceTranscriptDocument}
              fontSize={fontSize}
            />
          ) : (
            <textarea
              className="detail-editor"
              value={visibleTranscript}
              onChange={(event) => setDraftTranscript(event.target.value)}
              readOnly={transcriptReadOnly}
              title={
                transcriptReadOnly
                  ? t(
                      "detail.originalTranscriptReadonly",
                      "Original transcript is read-only. Switch back to the optimized version to edit.",
                    )
                  : undefined
              }
              style={{
                fontSize: `${fontSize}px`,
                position: search.trim() ? "absolute" : "relative",
                top: 0,
                left: 0,
                width: "100%",
                height: "100%",
                background: search.trim() ? "transparent" : undefined,
                color: search.trim() ? "black" : undefined,
                opacity: search.trim() ? 0.7 : 1,
              }}
            />
          )}
        </div>
      </div>
    );
  }

  function renderDefaultInspector(): JSX.Element {
    const peoplePillText = (() => {
      if (detailMode === "segments" && selectedDetailSegment) {
        return (
          selectedDetailSegment.speakerLabel ??
          t("inspector.unknown", "Unknown")
        );
      }

      if (!artifactDiarizationUiState) {
        return t("inspector.unknown", "Unknown");
      }

      switch (artifactDiarizationUiState.kind) {
        case "speakers_detected":
          return t("detail.speakersDetected", "{count} speakers detected", {
            count: artifactDiarizationUiState.speakerCount,
          });
        case "failed":
          return t("detail.diarizationFailedShort", "Diarization failed");
        case "no_speakers_detected":
          return t("detail.noSpeakersShort", "No speakers detected");
        case "not_requested":
        default:
          return t("detail.noSpeakerLabels", "No speaker labels");
      }
    })();

    const peopleSummaryText = (() => {
      if (detailMode === "segments") {
        if (
          selectedSegmentSpeakerLabel.length > 0 &&
          speakerDraft.trim().length > 0 &&
          speakerDraft.trim() !== selectedSegmentSpeakerLabel
        ) {
          return t(
            "inspector.renameSpeakerHint",
            "Rename updates this speaker everywhere in the transcript. Assign only changes the selected segment.",
          );
        }
        return selectedSegmentSourceIndex === null
          ? t(
              "inspector.selectSegmentHint",
              "Select a segment to assign a speaker manually.",
            )
          : t(
              "inspector.speakerSavedHint",
              "Speaker label is saved into segment metadata. Right-click a segment for quick actions.",
            );
      }

      if (!artifactDiarizationUiState) {
        return t(
          "detail.manualSpeakerHint",
          "Assign speakers manually in Segments. You can decide later whether to run speaker diarization.",
        );
      }

      switch (artifactDiarizationUiState.kind) {
        case "speakers_detected":
          return formatSpeakerSummary(artifactDiarizationUiState.speakerLabels);
        case "failed":
          return (
            artifactDiarizationUiState.error ??
            t(
              "detail.diarizationFailedFallback",
              "Speaker diarization failed after transcription.",
            )
          );
        case "no_speakers_detected":
          return t(
            "detail.noSpeakersDetectedHint",
            "Open Segments to inspect the timeline or assign speakers manually.",
          );
        case "not_requested":
        default:
          return t(
            "detail.manualSpeakerHint",
            "Assign speakers manually in Segments. You can decide later whether to run speaker diarization.",
          );
      }
    })();

    return (
      <div className="inspector-body">
        <button
          className="secondary-button"
          onClick={() => void navigator.clipboard.writeText(visibleTranscript)}
        >
          {t("inspector.copy", "Copy")}
        </button>

        <TranscriptSegmentsTileSwitch
          detailMode={detailMode}
          onSelectMode={(mode) => setDetailMode(mode)}
        />

        {detailMode === "transcript" && hasOptimizedTranscript ? (
          <div className="inspector-block transcript-version-block">
            <div
              className="segmented-control transcript-version-toggle"
              role="group"
              aria-label={t("detail.transcriptVersion", "Transcript version")}
            >
              <button
                type="button"
                className={
                  transcriptViewMode === "optimized" ? "seg active" : "seg"
                }
                onClick={() => {
                  setShowConfidenceColors(false);
                  setTranscriptViewMode("optimized");
                }}
                title={t(
                  "detail.showOptimizedTranscript",
                  "Show optimized transcript",
                )}
              >
                {t("detail.showOptimized", "Show optimized")}
              </button>
              <button
                type="button"
                className={
                  transcriptViewMode === "original" ? "seg active" : "seg"
                }
                onClick={() => setTranscriptViewMode("original")}
                title={t(
                  "detail.showOriginalTranscript",
                  "Show original transcript",
                )}
              >
                {t("detail.showOriginal", "Show original")}
              </button>
            </div>
          </div>
        ) : null}

        {detailMode === "transcript" && confidenceColorsAvailable ? (
          <div className="inspector-block confidence-toggle-block">
            <label
              className={`confidence-toggle-card ${showConfidenceColors ? "is-on" : "is-off"}`}
            >
              <input
                className="confidence-toggle-input"
                type="checkbox"
                role="switch"
                aria-label={t(
                  "sidebar.confidenceColorsToggle",
                  "Toggle confidence colors",
                )}
                checked={showConfidenceColors}
                onChange={(event) => {
                  const enabled = event.target.checked;
                  setShowConfidenceColors(enabled);
                  if (enabled) {
                    setTranscriptViewMode("original");
                  }
                }}
              />
              <div className="confidence-toggle-copy">
                <span className="confidence-toggle-title">
                  {t("sidebar.confidenceColors", "Confidence colors")}
                </span>
              </div>
              <span className="confidence-power-switch" aria-hidden="true">
                <span className="confidence-power-track">
                  <span className="confidence-power-spectrum" />
                  <span className="confidence-power-thumb" />
                </span>
              </span>
            </label>
          </div>
        ) : null}

        <div className="inspector-block">
          <h4>{t("inspector.audio")}</h4>
          <div className="property-line">
            <span>{t("inspector.file")}</span>
            <strong className="truncate-value" title={detailAudioFileLabel}>
              {detailAudioFileLabel}
            </strong>
          </div>
          <div className="property-line">
            <span>{t("inspector.format")}</span>
            <strong>{detailAudioFormat}</strong>
          </div>
          <div className="property-line">
            <span>{t("inspector.duration")}</span>
            <strong>{formatShortDuration(transcriptSeconds)}</strong>
          </div>
        </div>

        <div className="inspector-block">
          <h4>{t("inspector.title")}</h4>
          <input
            className="inspector-input"
            value={draftTitle}
            onChange={(event) => setDraftTitle(event.target.value)}
          />
        </div>

        <div className="inspector-block">
          <h4>{t("inspector.people")}</h4>
          <div
            className={`pill ${
              artifactDiarizationUiState?.kind === "failed"
                ? "pill-warning"
                : artifactDiarizationUiState?.kind === "speakers_detected"
                  ? "pill-success"
                  : ""
            }`}
          >
            {peoplePillText}
          </div>
          {showSpeakerManagement && knownSpeakerLabels.length > 0 ? (
            <div className="speaker-known-list">
              <span className="speaker-known-label">
                {t("inspector.detectedSpeakers", "Detected speakers")}
              </span>
              <div className="speaker-chip-list">
                {knownSpeakers.map((speaker) => (
                  <div key={speaker.id} className="speaker-chip-row">
                    <button
                      type="button"
                      className={`speaker-chip-button ${
                        speaker.label === selectedSegmentSpeakerLabel
                          ? "is-active"
                          : ""
                      }`}
                      style={buildSpeakerAccentStyle(speaker.color)}
                      onClick={() => onStartRenameSpeakerLabel(speaker.label)}
                    >
                      <span className="speaker-chip-button-label">
                        {speaker.label}
                      </span>
                    </button>
                    <label
                      className="speaker-color-control"
                      title={t("inspector.speakerColor", "Speaker color")}
                    >
                      <span className="sr-only">
                        {t(
                          "inspector.speakerColorFor",
                          "Speaker color for {speaker}",
                          {
                            speaker: speaker.label,
                          },
                        )}
                      </span>
                      <input
                        type="color"
                        value={speaker.color}
                        onChange={(event) => {
                          void onSetSpeakerColor(
                            speaker.id,
                            event.target.value,
                          );
                        }}
                      />
                    </label>
                    <button
                      type="button"
                      className="icon-button danger-icon-button speaker-chip-inline-remove"
                      title={t(
                        "inspector.removeSpeakerFor",
                        "Remove speaker {speaker}",
                        {
                          speaker: speaker.label,
                        },
                      )}
                      aria-label={t(
                        "inspector.removeSpeakerFor",
                        "Remove speaker {speaker}",
                        {
                          speaker: speaker.label,
                        },
                      )}
                      disabled={isAssigningSpeaker}
                      onClick={(event) => {
                        event.stopPropagation();
                        void onRemoveDetectedSpeaker(speaker);
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                ))}
              </div>
            </div>
          ) : null}
          {showSpeakerManagement && knownSpeakers.length > 1 ? (
            <div className="speaker-merge-panel">
              <span className="speaker-known-label">
                {t("inspector.mergeSpeakers", "Merge speakers")}
              </span>
              <div className="speaker-merge-row">
                <select
                  className="inspector-select"
                  value={mergeSpeakerSourceId}
                  disabled={isAssigningSpeaker || !activeArtifact}
                  onChange={(event) =>
                    setMergeSpeakerSourceId(event.target.value)
                  }
                >
                  {knownSpeakers.map((speaker) => (
                    <option key={speaker.id} value={speaker.id}>
                      {speaker.label}
                    </option>
                  ))}
                </select>
                <span className="speaker-merge-arrow">
                  {t("inspector.mergeInto", "into")}
                </span>
                <select
                  className="inspector-select"
                  value={mergeSpeakerTargetId}
                  disabled={
                    isAssigningSpeaker ||
                    !activeArtifact ||
                    mergeTargetSpeakers.length === 0
                  }
                  onChange={(event) =>
                    setMergeSpeakerTargetId(event.target.value)
                  }
                >
                  {mergeTargetSpeakers.map((speaker) => (
                    <option key={speaker.id} value={speaker.id}>
                      {speaker.label}
                    </option>
                  ))}
                </select>
                <button
                  className="secondary-button speaker-assign-button"
                  disabled={isAssigningSpeaker || !canMergeSpeakers}
                  onClick={() => void onMergeSelectedSpeaker()}
                >
                  {isAssigningSpeaker
                    ? "..."
                    : t("inspector.mergeSpeakerAction", "Merge")}
                </button>
              </div>
              <small className="muted">
                {t(
                  "inspector.mergeSpeakerHint",
                  "All segments assigned to the first speaker will be reassigned to the second speaker.",
                )}
              </small>
            </div>
          ) : null}
          {showSpeakerManagement ? (
            <>
              <div className="speaker-edit-row">
                <input
                  ref={peopleSpeakerInputRef}
                  className="inspector-input"
                  placeholder={t("inspector.addSpeaker", "Add a speaker...")}
                  list="speaker-suggestions"
                  value={speakerDraft}
                  disabled={
                    !activeArtifact || selectedSegmentSourceIndex === null
                  }
                  onChange={(event) => setSpeakerDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void onAssignSpeakerToSelectedSegment();
                    }
                  }}
                />
                <button
                  className="secondary-button speaker-assign-button"
                  disabled={
                    isAssigningSpeaker ||
                    !activeArtifact ||
                    selectedSegmentSourceIndex === null ||
                    !speakerDraft.trim()
                  }
                  onClick={() => void onAssignSpeakerToSelectedSegment()}
                >
                  {isAssigningSpeaker ? "..." : t("inspector.assign", "Assign")}
                </button>
                <button
                  className="secondary-button speaker-assign-button"
                  disabled={isAssigningSpeaker || !canRenameSelectedSpeaker}
                  onClick={() => void onRenameSelectedSpeaker()}
                >
                  {isAssigningSpeaker
                    ? "..."
                    : t("inspector.renameSpeaker", "Rename")}
                </button>
              </div>
              <label className="toggle-row compact">
                <span>{t("inspector.propagate")}</span>
                <input
                  type="checkbox"
                  checked={propagateSpeakerAssignment}
                  onChange={(event) =>
                    setPropagateSpeakerAssignment(event.target.checked)
                  }
                />
              </label>
              {knownSpeakerLabels.length > 0 ? (
                <datalist id="speaker-suggestions">
                  {knownSpeakerLabels.map((speaker) => (
                    <option key={speaker} value={speaker} />
                  ))}
                </datalist>
              ) : null}
              <small className="muted">{peopleSummaryText}</small>
            </>
          ) : (
            <small className="muted">
              {t(
                "inspector.manageSpeakersInSegments",
                "Open Segments to manage speakers.",
              )}
            </small>
          )}
        </div>

        <div className="inspector-block">
          <h4>{t("inspector.options")}</h4>
          <label className="toggle-row">
            <span>{t("inspector.fontSize")}</span>
            <select
              className="inspector-select"
              value={fontSize}
              onChange={(event) => setFontSize(Number(event.target.value))}
            >
              <option value={14}>14</option>
              <option value={16}>16</option>
              <option value={18}>18</option>
              <option value={20}>20</option>
              <option value={22}>22</option>
            </select>
          </label>
        </div>
      </div>
    );
  }

  function renderSummaryInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <label>
          {t("summary.aiService", "AI Service")}
          <select
            className="inspector-select"
            value={activeAiServiceSelectValue}
            onChange={(event) => {
              void onSelectAiService(event.target.value);
            }}
          >
            {aiServiceSelectOptions.map((option) => (
              <option
                key={option.value}
                value={option.value}
                disabled={option.disabled}
              >
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <button
          className="secondary-button"
          onClick={() =>
            void navigator.clipboard.writeText(
              draftSummary || visibleTranscript,
            )
          }
        >
          {t("summary.copy", "Copy")}
        </button>
        <button
          className="secondary-button"
          onClick={() => void onGenerateSummary()}
          disabled={
            isGeneratingSummary || !activeArtifact || !aiFeaturesAvailable
          }
          title={!aiFeaturesAvailable ? aiUnavailableReason : undefined}
        >
          {isGeneratingSummary
            ? t("detail.summarizing", "Summarizing...")
            : t("summary.summarize", "Summarize")}
        </button>
        <button
          className="secondary-button"
          onClick={() => setDraftSummary("")}
        >
          {t("summary.clear")}
        </button>
        {!aiFeaturesAvailable ? (
          <p className="muted">{aiUnavailableReason}</p>
        ) : null}

        <label className="toggle-row">
          <span>{t("summary.includeTimestamps")}</span>
          <input
            type="checkbox"
            checked={summaryIncludeTimestamps}
            onChange={(event) =>
              setSummaryIncludeTimestamps(event.target.checked)
            }
          />
        </label>
        <label className="toggle-row">
          <span>{t("summary.includeSpeakers")}</span>
          <input
            type="checkbox"
            checked={summaryIncludeSpeakers}
            onChange={(event) =>
              setSummaryIncludeSpeakers(event.target.checked)
            }
          />
        </label>
        <label className="toggle-row">
          <span>{t("summary.autostartSummary")}</span>
          <input
            type="checkbox"
            checked={summaryAutostart}
            onChange={(event) => setSummaryAutostart(event.target.checked)}
          />
        </label>

        <div className="inspector-block">
          <h4>{t("inspector.options")}</h4>
          <label className="toggle-row">
            <span>{t("summary.sections")}</span>
            <input
              type="checkbox"
              checked={summarySections}
              onChange={(event) => setSummarySections(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>{t("summary.bulletPoints")}</span>
            <input
              type="checkbox"
              checked={summaryBulletPoints}
              onChange={(event) => setSummaryBulletPoints(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>{t("summary.actionItems")}</span>
            <input
              type="checkbox"
              checked={summaryActionItems}
              onChange={(event) => setSummaryActionItems(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>{t("summary.keyPointsOnly")}</span>
            <input
              type="checkbox"
              checked={summaryKeyPointsOnly}
              onChange={(event) =>
                setSummaryKeyPointsOnly(event.target.checked)
              }
            />
          </label>
        </div>

        <label>
          {t("summary.language", "Language")}
          <select
            className="inspector-select"
            value={summaryLanguage}
            onChange={(event) =>
              setSummaryLanguage(event.target.value as LanguageCode)
            }
          >
            {languageOptions
              .filter((option) => option.value !== "auto")
              .map((option) => (
                <option key={option.value} value={option.value}>
                  {t(`lang.${option.value}`, option.label)}
                </option>
              ))}
          </select>
        </label>

        <label>
          {t("summary.customPrompt", "Custom summary prompt")}
          <textarea
            className="inspector-prompt"
            value={summaryCustomPrompt}
            onChange={(event) => setSummaryCustomPrompt(event.target.value)}
            placeholder={t(
              "summary.customPromptPlaceholder",
              "Optional: override summary instructions...",
            )}
          />
        </label>
      </div>
    );
  }

  function renderEmotionInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <label>
          {t("summary.aiService", "AI Service")}
          <select
            className="inspector-select"
            value={activeAiServiceSelectValue}
            onChange={(event) => {
              void onSelectAiService(event.target.value);
            }}
          >
            {aiServiceSelectOptions.map((option) => (
              <option
                key={option.value}
                value={option.value}
                disabled={option.disabled}
              >
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <button
          className="secondary-button"
          onClick={() =>
            void navigator.clipboard.writeText(
              draftEmotionAnalysis?.narrative_markdown || visibleTranscript,
            )
          }
        >
          {t("summary.copy", "Copy")}
        </button>
        <button
          className="secondary-button"
          onClick={() => void onGenerateEmotionAnalysis()}
          disabled={
            isGeneratingEmotionAnalysis ||
            !activeArtifact ||
            !aiFeaturesAvailable
          }
          title={!aiFeaturesAvailable ? aiUnavailableReason : undefined}
        >
          {isGeneratingEmotionAnalysis
            ? t("emotion.generating", "Analyzing emotions...")
            : t("emotion.generate", "Analyze emotions")}
        </button>
        {!aiFeaturesAvailable ? (
          <p className="muted">
            {draftEmotionAnalysis
              ? t(
                  "emotion.cachedOnly",
                  "Cached emotion analysis is available, but regeneration needs an AI provider.",
                )
              : aiUnavailableReason}
          </p>
        ) : null}
        {emotionAnalysisGeneratedAt ? (
          <p className="muted">
            {t("emotion.generatedAt", "Last generated")}:{" "}
            {formatDate(emotionAnalysisGeneratedAt)}
          </p>
        ) : null}

        <label className="toggle-row">
          <span className="emotion-toggle-label">
            {t("summary.includeTimestamps", "Include timestamps")}
            <InlineInfoHint
              label={t("summary.includeTimestamps", "Include timestamps")}
              description={t(
                "emotion.includeTimestampsHelp",
                "Shows transcript timestamps in the analysis so you can connect each emotional moment back to the recording.",
              )}
              side="left"
            />
          </span>
          <input
            type="checkbox"
            checked={emotionIncludeTimestamps}
            onChange={(event) =>
              setEmotionIncludeTimestamps(event.target.checked)
            }
          />
        </label>
        <label className="toggle-row">
          <span className="emotion-toggle-label">
            {t("summary.includeSpeakers", "Include speakers")}
            <InlineInfoHint
              label={t("summary.includeSpeakers", "Include speakers")}
              description={t(
                "emotion.includeSpeakersHelp",
                "Shows speaker labels inside the analysis when the transcript has diarization or speaker metadata available.",
              )}
              side="left"
            />
          </span>
          <input
            type="checkbox"
            checked={emotionIncludeSpeakers}
            onChange={(event) =>
              setEmotionIncludeSpeakers(event.target.checked)
            }
          />
        </label>
        <label className="toggle-row">
          <span className="emotion-toggle-label">
            {t("emotion.speakerDynamics", "Speaker dynamics")}
            <InlineInfoHint
              label={t("emotion.speakerDynamics", "Speaker dynamics")}
              description={t(
                "emotion.speakerDynamicsHelp",
                "Adds a cross-speaker reading to the overview, highlighting how different speakers contribute tension, caution, confidence, or other recurring tones across the conversation.",
              )}
              side="left"
            />
          </span>
          <input
            type="checkbox"
            checked={speakerDynamicsAvailable && emotionSpeakerDynamics}
            disabled={!speakerDynamicsAvailable}
            onChange={(event) =>
              setEmotionSpeakerDynamics(event.target.checked)
            }
          />
        </label>

        <label>
          {t("summary.language", "Language")}
          <select
            className="inspector-select"
            value={emotionLanguage}
            onChange={(event) =>
              setEmotionLanguage(event.target.value as LanguageCode)
            }
          >
            {languageOptions
              .filter((option) => option.value !== "auto")
              .map((option) => (
                <option key={option.value} value={option.value}>
                  {t(`lang.${option.value}`, option.label)}
                </option>
              ))}
          </select>
        </label>
      </div>
    );
  }

  function renderChatInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <h4>{t("chat.prompts")}</h4>
        <p className="muted">
          {t(
            "detail.chatPromptsHint",
            "Quick prompts are available above the composer.",
          )}
        </p>
        <button
          className="secondary-button"
          onClick={() => {
            void onOpenStandaloneSettingsWindow("prompts");
          }}
        >
          {t("chat.managePrompts", "Manage Prompts")}
        </button>

        <div className="inspector-block">
          <h4>{t("inspector.options")}</h4>
          <label className="toggle-row">
            <span>{t("summary.includeTimestamps")}</span>
            <input
              type="checkbox"
              checked={chatIncludeTimestamps}
              onChange={(event) =>
                setChatIncludeTimestamps(event.target.checked)
              }
            />
          </label>
          <label className="toggle-row">
            <span>{t("summary.includeSpeakers")}</span>
            <input
              type="checkbox"
              checked={chatIncludeSpeakers}
              onChange={(event) => setChatIncludeSpeakers(event.target.checked)}
            />
          </label>
        </div>
        {!aiFeaturesAvailable ? (
          <p className="muted">{aiUnavailableReason}</p>
        ) : null}
      </div>
    );
  }

  function renderMetadataInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <h4>{t("metadata.modelLanguage", "Model & Language")}</h4>

        <div className="property-grid">
          <label>{t("metadata.model")}</label>
          <select
            value={settings?.transcription.model ?? "base"}
            onChange={(event) =>
              void onChangeModel(event.target.value as SpeechModel)
            }
          >
            {modelOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {formatSpeechModelLabel(option.value, option.label)}
              </option>
            ))}
          </select>

          <label>{t("metadata.language")}</label>
          <select
            value={settings?.transcription.language ?? "auto"}
            onChange={(event) =>
              void onChangeLanguage(event.target.value as LanguageCode)
            }
          >
            {languageOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {t(`lang.${option.value}`, option.label)}
              </option>
            ))}
          </select>
        </div>

        {activeArtifact ? (
          <>
            <div className="property-line">
              <span>{t("metadata.kind")}</span>
              <strong>{formatArtifactKindLabel(activeArtifact.kind)}</strong>
            </div>
            <div className="property-line">
              <span>{t("metadata.audioDuration")}</span>
              <strong>{formatShortDuration(transcriptSeconds)}</strong>
            </div>
            <div className="property-line">
              <span>{t("metadata.created")}</span>
              <strong>{formatDate(activeArtifact.created_at)}</strong>
            </div>
            <div className="property-line">
              <span>{t("metadata.updated")}</span>
              <strong>{formatDate(activeArtifact.updated_at)}</strong>
            </div>
            <div className="property-line">
              <span>{t("metadata.characters")}</span>
              <strong>{visibleTranscript.length}</strong>
            </div>
            <div className="property-line">
              <span>{t("metadata.words")}</span>
              <strong>{transcriptWordCount}</strong>
            </div>
          </>
        ) : null}

        <button
          className="primary-button"
          onClick={() => void onSaveArtifact()}
          disabled={isSavingArtifact}
        >
          {isSavingArtifact
            ? t("metadata.saving", "Saving...")
            : t("metadata.save", "Save")}
        </button>
      </div>
    );
  }

  function renderInspector(): JSX.Element {
    if (isRealtimeDetailActive) {
      return (
        <div className="inspector-body">
          <button
            className="secondary-button"
            onClick={() =>
              void navigator.clipboard.writeText(realtimeTranscriptText)
            }
            disabled={!realtimeTranscriptText.trim()}
          >
            {t("inspector.copy", "Copy")}
          </button>

          <div className="inspector-block">
            <h4>{t("realtime.status")}</h4>
            <div className="property-line">
              <span>{t("realtime.status")}</span>
              <strong>
                {realtimeState === "idle"
                  ? t("realtime.idle", "Realtime idle")
                  : realtimeMessage}
              </strong>
            </div>
            <div className="property-line">
              <span>{t("inspector.duration")}</span>
              <strong>{formatShortDuration(realtimeElapsedSeconds)}</strong>
            </div>
          </div>

          <div className="inspector-block">
            <h4>{t("inspector.title")}</h4>
            <input
              className="inspector-input"
              value={draftTitle}
              onChange={(event) => setDraftTitle(event.target.value)}
            />
          </div>

          <div className="inspector-block">
            <h4>{t("inspector.audio")}</h4>
            <div className="property-line">
              <span>{t("inspector.file")}</span>
              <strong className="truncate-value">
                {t("realtime.liveMicrophone", "Live microphone")}
              </strong>
            </div>
            <div className="property-line">
              <span>{t("inspector.format")}</span>
              <strong>
                {t("realtime.recordingPending", "Recording in progress")}
              </strong>
            </div>
            <div className="property-line">
              <span>{t("detail.transcript", "Transcript")}</span>
              <strong>
                {realtimeHasAnyText
                  ? t("realtime.transcriptUpdating", "Updating live")
                  : t("realtime.waitingForSpeech", "Waiting for speech")}
              </strong>
            </div>
          </div>
        </div>
      );
    }

    if (!activeArtifact) {
      if (focusedJobId) {
        return (
          <div className="inspector-body">
            <button
              className="secondary-button"
              onClick={() =>
                void navigator.clipboard.writeText(
                  stripAnsi(activeJobPreviewText),
                )
              }
              disabled={!activeJobPreviewText}
            >
              {t("inspector.copy", "Copy")}
            </button>
            <div className="inspector-block">
              <h4>{t("inspector.transcribingTitle")}</h4>
              <p className="muted">
                {progress?.message ??
                  t(
                    "inspector.whisperRunning",
                    "Running Whisper transcription...",
                  )}
              </p>
            </div>
          </div>
        );
      }
      return (
        <div className="inspector-body muted">
          {t("inspector.noTranscript")}
        </div>
      );
    }

    if (inspectorMode === "info") return renderMetadataInspector();
    if (detailMode === "emotion") return renderEmotionInspector();
    if (detailMode === "summary") return renderSummaryInspector();
    if (detailMode === "chat") return renderChatInspector();
    return renderDefaultInspector();
  }

  function renderDetail(): JSX.Element {
    const isTrimRetranscriptionStarting =
      isStarting && Boolean(effectiveTrimmedAudioDraft);

    return (
      <div
        ref={detailLayoutRef}
        className={
          effectiveRightSidebarOpen
            ? "detail-layout"
            : "detail-layout right-collapsed"
        }
        style={detailLayoutStyle}
      >
        <section className="detail-main" ref={detailMainRef}>
          <DetailToolbar
            leftSidebarOpen={leftSidebarOpen}
            rightSidebarOpen={effectiveRightSidebarOpen}
            rightSidebarForcedCollapsed={rightSidebarForcedCollapsed}
            detailMode={detailMode}
            title={
              isRealtimeDetailActive
                ? draftTitle.trim() ||
                  effectiveDetailContext?.title ||
                  t("topbar.live", "Live")
                : (effectiveDetailContext?.title ??
                  activeJobTitle ??
                  t("detail.transcribing", "Transcribing"))
            }
            hasArtifact={Boolean(activeArtifact)}
            hasActiveJob={Boolean(focusedJobId)}
            transcriptionProgress={displayedTranscriptionPercentage}
            onToggleSidebar={() => setLeftSidebarOpen((open) => !open)}
            onBack={() => {
              if (!activeArtifact && focusedJobId) {
                setFocusedJobId(null);
                setActiveDetailContext(null);
              }
              setSection("history");
            }}
            onRenameTitle={
              activeArtifact
                ? () => onRenameArtifact(activeArtifact)
                : undefined
            }
            onSelectMode={(mode) => {
              if (mode === "transcript") {
                if (detailMode !== "segments") {
                  setDetailMode("transcript");
                }
              } else {
                setDetailMode(mode);
              }
              setInspectorMode("details");
            }}
            onOpenExport={() => setShowExportSheet(true)}
            onShowDetailsPanel={() => setRightSidebarOpen(true)}
            onHideDetailsPanel={() => setRightSidebarOpen(false)}
            onCancel={() => void onCancel()}
            isImprovingText={isImprovingText}
            onImproveText={onImproveText}
            chatDisabled={!aiFeaturesAvailable}
            optimizeDisabled={!aiFeaturesAvailable}
            optimizeDisabledTitle={aiUnavailableReason}
            showRetranscribe={Boolean(
              effectiveTrimmedAudioDraft && !activeJobId,
            )}
            isStartingTrimmedAudioRetranscription={
              isTrimRetranscriptionStarting
            }
            onRetranscribeTrimmedAudio={() => {
              if (effectiveTrimmedAudioDraft) {
                void onStartTranscription(effectiveTrimmedAudioDraft.path, {
                  parentId: effectiveTrimmedAudioDraft.parentArtifactId,
                  title: effectiveTrimmedAudioDraft.title,
                });
              }
            }}
            realtimeControls={
              isRealtimeDetailActive
                ? {
                    state: realtimeState,
                    isStopping: isStoppingRealtime,
                    onPause: () => void onPauseRealtime(),
                    onResume: () => void onResumeRealtime(),
                    onStop: () => void onStopRealtime(true),
                  }
                : null
            }
          />

          <div
            className={
              detailMode === "chat"
                ? "detail-body detail-body--chat"
                : "detail-body"
            }
          >
            {renderDetailMain()}
          </div>

          {segmentContextMenu ? (
            <div
              className="segment-context-menu"
              role="menu"
              style={{ left: segmentContextMenu.x, top: segmentContextMenu.y }}
              onContextMenu={(event) => event.preventDefault()}
            >
              <button
                type="button"
                className="segment-context-item"
                role="menuitem"
                onClick={onAddSpeakerFromContextMenu}
              >
                {t("inspector.addSpeaker", "Add a speaker...")}
              </button>
              <div className="segment-context-separator" />
              <p className="segment-context-title">{t("inspector.assign")}</p>
              {knownSpeakerLabels.length > 0 ? (
                knownSpeakerLabels.map((speakerLabel) => (
                  <button
                    key={speakerLabel}
                    type="button"
                    className="segment-context-item"
                    role="menuitem"
                    disabled={isAssigningSpeaker}
                    onClick={() =>
                      onAssignKnownSpeakerFromContextMenu(speakerLabel)
                    }
                  >
                    {speakerLabel}
                  </button>
                ))
              ) : (
                <p className="segment-context-empty">
                  {t("inspector.unknown")}
                </p>
              )}
              <div className="segment-context-separator" />
              <button
                type="button"
                className="segment-context-item danger"
                role="menuitem"
                disabled={
                  isAssigningSpeaker || !contextMenuSegment?.speakerLabel
                }
                onClick={onClearSpeakerFromContextMenu}
              >
                {t("inspector.clearSpeaker", "Clear Speaker")}
              </button>
            </div>
          ) : null}

          <div className="detail-audio-stack">
            <div className="detail-audio-player-group">
              {effectiveTrimmedAudioDraft ? (
                <div className="detail-audio-player-label">
                  {t("detail.trimmedAudio", "Trimmed audio")}
                </div>
              ) : null}
              {trimRetranscriptionError ? (
                <div className="detail-inline-error" role="alert">
                  <span>{trimRetranscriptionError}</span>
                  {shouldOfferLocalModelsCta(trimRetranscriptionError) ? (
                    <button
                      type="button"
                      className="secondary-button"
                      onClick={() =>
                        void onOpenStandaloneSettingsWindow("local_models")
                      }
                    >
                      {t("action.openLocalModels", "Open Local Models")}
                    </button>
                  ) : null}
                </div>
              ) : null}
              {isTrimRetranscriptionStarting && !trimRetranscriptionError ? (
                <div
                  className="detail-inline-status"
                  role="status"
                  aria-live="polite"
                >
                  <span>
                    {t(
                      "detail.preparingTrimmedRetranscription",
                      "Preparing trimmed audio transcription...",
                    )}
                  </span>
                </div>
              ) : null}
              {isRealtimeDetailActive ? (
                <LiveMicrophoneWaveform
                  ariaLabel={t(
                    "realtime.waveformAriaLabel",
                    "Live microphone waveform",
                  )}
                  mode={realtimeState}
                  previewState={realtimePreviewState}
                  levels={realtimeInputLevels}
                  elapsedSeconds={realtimeElapsedSeconds}
                  runningLabel={t("realtime.waveformRunning", "Mic live")}
                  pausedLabel={t("realtime.waveformPaused", "Preview paused")}
                  idleStatusLabel={t("realtime.waveformIdleShort", "Mic idle")}
                  idleLabel={t(
                    "realtime.waveformIdle",
                    "Start live mode to preview microphone activity.",
                  )}
                  connectingLabel={t(
                    "realtime.waveformConnecting",
                    "Connecting to the microphone...",
                  )}
                  blockedLabel={t(
                    "realtime.waveformBlocked",
                    "Microphone preview is blocked. Check microphone permissions.",
                  )}
                  unavailableLabel={t(
                    "realtime.waveformUnavailable",
                    "Microphone preview is unavailable on this device.",
                  )}
                />
              ) : (
                <AudioPlayer
                  artifactId={detailAudioArtifactId}
                  inputPath={detailAudioInputPath}
                  sourceLabel={
                    activeArtifact?.source_label ??
                    effectiveDetailContext?.sourceArtifact?.source_label ??
                    null
                  }
                  trimEnabled
                  onMetadataLoaded={(metadata) => {
                    setAudioDurationSeconds(metadata.durationSeconds);
                  }}
                  onTrimRegionsChange={(regions) => {
                    setTrimRegions(regions);
                    setTrimRetranscriptionError(null);
                  }}
                  onTrimApplied={(trimmedAudio, regions) => {
                    const trimSourceArtifact =
                      activeArtifact ?? effectiveDetailContext?.sourceArtifact;
                    if (!trimSourceArtifact) {
                      return;
                    }
                    const sourceLabel =
                      trimSourceArtifact.title.trim() ||
                      trimSourceArtifact.source_label ||
                      t("inspector.unknown", "Unknown");
                    setTrimRetranscriptionError(null);
                    setTrimmedAudioDraft({
                      path: trimmedAudio.path,
                      durationSeconds: trimmedAudio.duration_seconds,
                      fileSizeBytes: trimmedAudio.file_size_bytes,
                      parentArtifactId: trimSourceArtifact.id,
                      title: buildTrimArtifactTitle(sourceLabel, regions),
                      regions: [...regions],
                    });
                  }}
                />
              )}
            </div>
          </div>
        </section>

        <aside
          className={`detail-inspector ${detailMode === "chat" ? "detail-inspector--chat " : ""}${effectiveRightSidebarOpen ? "" : "collapsed"}`}
        >
          <DetailInspectorHeader
            inspectorMode={inspectorMode}
            onInspectorModeChange={setInspectorMode}
            onHideDetailsPanel={() => setRightSidebarOpen(false)}
          />
          {renderInspector()}
          {effectiveRightSidebarOpen && !rightSidebarForcedCollapsed ? (
            <div
              className="sidebar-resize-handle sidebar-resize-handle-right"
              role="separator"
              aria-orientation="vertical"
              aria-label={t("detail.resizeInspector", "Resize details panel")}
              onMouseDown={(event) => onStartSidebarResize("right", event)}
            />
          ) : null}
        </aside>
      </div>
    );
  }

  function renderRealtime(): JSX.Element {
    const combinedText = realtimeFinalLines.join("\n");

    return (
      <div className="view-body">
        <div className="view-toolbar">
          <h2>{t("topbar.live")}</h2>
          <div className="toolbar-actions">
            <button
              className="realtime-toolbar-button realtime-toolbar-button--start"
              onClick={() => void onStartRealtime()}
              disabled={!canStartRealtime}
            >
              <span className="button-content">
                <Play size={14} />
                <span className="detail-action-label">
                  {t("realtime.start", "Start")}
                </span>
              </span>
            </button>
            <button
              className="realtime-toolbar-button realtime-toolbar-button--secondary"
              onClick={() => void onPauseRealtime()}
              disabled={realtimeState !== "running"}
            >
              <span className="button-content">
                <Pause size={14} />
                <span className="detail-action-label">
                  {t("realtime.pause", "Pause")}
                </span>
              </span>
            </button>
            <button
              className="realtime-toolbar-button realtime-toolbar-button--secondary"
              onClick={() => void onResumeRealtime()}
              disabled={realtimeState !== "paused"}
            >
              <span className="button-content">
                <Play size={14} />
                <span className="detail-action-label">
                  {t("realtime.resume", "Resume")}
                </span>
              </span>
            </button>
            <button
              className="realtime-toolbar-button realtime-toolbar-button--primary"
              onClick={() => void onStopRealtime(true)}
              disabled={realtimeState === "idle" || isStoppingRealtime}
            >
              <span className="button-content">
                <Square size={13} />
                <span className="detail-action-label">
                  {t("realtime.stopAndSave", "Stop & Save")}
                </span>
              </span>
            </button>
          </div>
        </div>

        <section className="panel-card">
          <div className="panel-head">
            <strong>{t("realtime.status")}</strong>
            <span className={`status-chip ${realtimeState}`}>
              {realtimeState === "idle"
                ? t("realtime.idle", "Realtime idle")
                : realtimeMessage}
            </span>
          </div>

          <div className="live-view">
            <LiveMicrophoneWaveform
              ariaLabel={t(
                "realtime.waveformAriaLabel",
                "Live microphone waveform",
              )}
              mode={realtimeState}
              previewState={realtimePreviewState}
              levels={realtimeInputLevels}
              elapsedSeconds={realtimeElapsedSeconds}
              runningLabel={t("realtime.waveformRunning", "Mic live")}
              pausedLabel={t("realtime.waveformPaused", "Preview paused")}
              idleStatusLabel={t("realtime.waveformIdleShort", "Mic idle")}
              idleLabel={t(
                "realtime.waveformIdle",
                "Start live mode to preview microphone activity.",
              )}
              connectingLabel={t(
                "realtime.waveformConnecting",
                "Connecting to the microphone...",
              )}
              blockedLabel={t(
                "realtime.waveformBlocked",
                "Microphone preview is blocked. Check microphone permissions.",
              )}
              unavailableLabel={t(
                "realtime.waveformUnavailable",
                "Microphone preview is unavailable on this device.",
              )}
            />

            <div className="live-transcript-panel">
              <div className="live-transcript-head">
                <strong>{t("detail.transcript", "Transcript")}</strong>
              </div>

              <div className="live-transcript-copy">
                {combinedText || realtimePreviewText ? (
                  <>
                    {combinedText ? <pre>{combinedText}</pre> : null}
                    {realtimePreviewText ? (
                      <p className="preview-line">{realtimePreviewText}</p>
                    ) : null}
                  </>
                ) : (
                  <div className="center-empty compact">
                    <h3>{t("realtime.noTranscript")}</h3>
                    <p>{t("realtime.noTranscriptDesc")}</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        </section>
      </div>
    );
  }

  const showGlobalUpdateBanner = shouldShowUpdateBanner(
    updateInfo,
    installingUpdate,
    checkingUpdates,
    dismissedUpdateVersion,
  );

  function renderPostUpdateValidationBanner(): JSX.Element | null {
    if (!warmStartEligible) {
      return null;
    }
    if (!postUpdateValidationError && !postUpdateValidationVisible) {
      return null;
    }

    const bannerTitle = postUpdateValidationError
      ? t(
          "startup.postUpdate.bannerErrorTitle",
          "Post-update checks need attention",
        )
      : t(
          "startup.postUpdate.bannerTitle",
          "Finishing post-update checks",
        );
    const bannerMessage = postUpdateValidationError
      ? postUpdateValidationError
      : t(
          "startup.postUpdate.bannerBody",
          "Sbobino is validating the updated local runtime in the background.",
        );

    return (
      <div
        className={`app-update-banner ${postUpdateValidationError ? "is-available" : "is-progress"}`}
      >
        <div className="app-update-banner-copy">
          <strong>{bannerTitle}</strong>
          <span>{bannerMessage}</span>
        </div>
        <div className="app-update-banner-actions">
          {postUpdateValidationError ? (
            <button
              className="primary-button"
              onClick={() => {
                setPostUpdateValidationError(null);
                setPostUpdateValidationVisible(false);
                setPostUpdateValidationAttempt((value) => value + 1);
              }}
              disabled={postUpdateValidationRunning}
            >
              {postUpdateValidationRunning
                ? t("startup.postUpdate.retrying", "Retrying...")
                : t("startup.postUpdate.retry", "Retry checks")}
            </button>
          ) : null}
          {postUpdateValidationError &&
          shouldOfferLocalModelsCta(postUpdateValidationError) ? (
            <button
              className="secondary-button"
              onClick={() => void onOpenStandaloneSettingsWindow("local_models")}
            >
              {t("action.openLocalModels", "Open Local Models")}
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  function renderGlobalUpdateBanner(): JSX.Element | null {
    if (!showGlobalUpdateBanner) {
      return null;
    }

    const latestVersion = updateInfo?.latest_version ?? "";
    const bannerTitle = checkingUpdates
      ? t("updates.banner.checkingTitle", "Checking for updates")
      : installingUpdate
        ? t("updates.banner.installingTitle", "Installing update")
        : updateInfo?.has_update
          ? t("updates.banner.availableTitle", "Update {version} available", {
              version: latestVersion,
            })
          : t("updates.banner.readyTitle", "Updater active");
    const bannerMessage = updateStatusMessage ??
      (checkingUpdates
        ? t(
            "updates.banner.checkingBody",
            "Sbobino is checking for a newer version in the background.",
          )
        : installingUpdate
          ? t(
              "updates.banner.installingBody",
              "The update is being downloaded and applied automatically.",
            )
          : updateInfo?.has_update
            ? t(
                "updates.banner.availableBody",
                "Install version {version} now or download it manually.",
                { version: latestVersion },
              )
            : "");
    const canInstallFromBanner = Boolean(
      updateInfo?.has_update &&
        nativeUpdate &&
        !installingUpdate &&
        !checkingUpdates,
    );
    const canDismissBanner = Boolean(updateInfo?.has_update && !installingUpdate);

    const onBannerClick = (event: ReactMouseEvent<HTMLDivElement>) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest("button, a")) {
        return;
      }
      if (!canInstallFromBanner) {
        return;
      }
      void onInstallUpdate();
    };

    const onBannerKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (!canInstallFromBanner) {
        return;
      }
      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }
      event.preventDefault();
      void onInstallUpdate();
    };

    return (
      <div
        className={`app-update-banner ${installingUpdate ? "is-progress" : "is-available"} ${canInstallFromBanner ? "is-clickable" : ""}`}
        onClick={onBannerClick}
        onKeyDown={onBannerKeyDown}
        role={canInstallFromBanner ? "button" : undefined}
        tabIndex={canInstallFromBanner ? 0 : undefined}
      >
        <div className="app-update-banner-copy">
          <strong>{bannerTitle}</strong>
          <span>{bannerMessage}</span>
        </div>
        <div className="app-update-banner-actions">
          {updateInfo?.has_update && nativeUpdate ? (
            <button
              className="primary-button"
              onClick={() => void onInstallUpdate()}
              disabled={installingUpdate || checkingUpdates}
            >
              {installingUpdate
                ? t("updates.banner.installingAction", "Installing...")
                : t("settings.general.downloadAndInstall", "Download & Install")}
            </button>
          ) : null}
          {updateInfo?.has_update && updateInfo.download_url ? (
            <a
              className="secondary-button"
              href={updateInfo.download_url}
              target="_blank"
              rel="noreferrer"
            >
              {t("settings.general.manualDownload", "Manual Download")}
            </a>
          ) : null}
          {canDismissBanner ? (
            <button
              className="update-banner-close"
              onClick={dismissUpdateBanner}
              title={t("action.dismiss", "Dismiss")}
            >
              <X size={14} />
            </button>
          ) : null}
        </div>
      </div>
    );
  }

  function renderSettingsGeneral(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>{t("settings.general.title", "General")}</h3>
            <p>{t("settings.general.desc")}</p>
          </header>

          <div className="settings-row">
            <div>
              <strong>
                {t("settings.general.autoUpdate", "Enable auto update checks")}
              </strong>
              <small>{t("settings.general.autoUpdateDesc")}</small>
            </div>
            <input
              type="checkbox"
              checked={settings.general.auto_update_enabled}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  general: {
                    ...current.general,
                    auto_update_enabled: event.target.checked,
                  },
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>
                {t("settings.general.appearanceMode", "Appearance")}
              </strong>
              <small>
                {t(
                  "settings.general.appearanceDesc",
                  "Choose app theme behavior.",
                )}
              </small>
            </div>
            <select
              value={settings.general.appearance_mode ?? "system"}
              onChange={(event) => {
                const value = event.target.value as AppearanceMode;
                void patchSettings((current) => ({
                  ...current,
                  general: {
                    ...current.general,
                    appearance_mode: value,
                  },
                }));
              }}
            >
              <option value="system">
                {t("settings.general.system", "System")}
              </option>
              <option value="light">
                {t("settings.general.light", "Light")}
              </option>
              <option value="dark">{t("settings.general.dark", "Dark")}</option>
            </select>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>
                {t("settings.general.appLanguage", "App Language")}
              </strong>
              <small>
                {t(
                  "settings.general.appLanguageDesc",
                  "Choose the application language.",
                )}
              </small>
            </div>
            <select
              value={settings.general.app_language ?? "en"}
              onChange={(event) => {
                const value = event.target.value as AppLanguage;
                changeLanguage(value);
                void patchSettings((current) => ({
                  ...current,
                  general: {
                    ...current.general,
                    app_language: value,
                  },
                }));
              }}
            >
              {supportedAppLanguages.map((language) => (
                <option key={language} value={language}>
                  {t(`lang.${language}`, language)}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-actions-row">
            <button
              className="secondary-button"
              onClick={() => void onRefreshUpdates()}
              disabled={checkingUpdates}
            >
              {checkingUpdates
                ? t("settings.general.checking", "Checking...")
                : t("settings.general.checkUpdates", "Check Updates")}
            </button>
            {updateInfo ? (
              <small>
                {updateInfo.has_update
                  ? updateSource === "native"
                    ? t(
                        "settings.general.updateAvailableInApp",
                        "Update {version} available (in-app install)",
                        {
                          version: updateInfo.latest_version ?? "",
                        },
                      )
                    : t(
                        "settings.general.updateAvailable",
                        "Update {version} available",
                        {
                          version: updateInfo.latest_version ?? "",
                        },
                      )
                  : t("settings.general.upToDate", "Up to date ({version})", {
                      version: updateInfo.current_version,
                    })}
              </small>
            ) : null}
          </div>
          {updateInfo?.has_update && nativeUpdate ? (
            <button
              className="cta-link-button"
              onClick={() => void onInstallUpdate()}
              disabled={installingUpdate}
            >
              {installingUpdate
                ? t("settings.general.installing", "Installing{suffix}", {
                    suffix:
                      updateDownloadPercent !== null
                        ? ` (${updateDownloadPercent}%)`
                        : "...",
                  })
                : t(
                    "settings.general.downloadAndInstall",
                    "Download & Install",
                  )}
            </button>
          ) : null}
          {updateInfo?.has_update && updateInfo.download_url ? (
            <a
              className="cta-link-button"
              href={updateInfo.download_url}
              target="_blank"
              rel="noreferrer"
            >
              {nativeUpdate
                ? t("settings.general.manualDownload", "Manual Download")
                : t("settings.general.downloadUpdate", "Download Update")}
            </a>
          ) : null}
          {updateStatusMessage ? <small>{updateStatusMessage}</small> : null}
        </section>
      </div>
    );
  }

  function renderSettingsTranscription(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    const speakerDiarization =
      settings.transcription.speaker_diarization ??
      getDefaultSpeakerDiarizationSettings();
    const pyannoteHealth = runtimeHealth?.pyannote ?? null;

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>
              {t("settings.transcription.title", "Transcription Defaults")}
            </h3>
            <p>
              {t(
                "settings.transcription.desc",
                "Used for every new transcription job.",
              )}
            </p>
          </header>

          <div className="settings-row settings-row-block">
            <div>
              <strong>
                {t("settings.transcription.engine", "Transcription engine")}
              </strong>
              <small>
                {t(
                  "settings.transcription.engineDesc",
                  "This app uses Whisper.cpp for local transcription.",
                )}
              </small>
            </div>
            <select
              value={settings.transcription.engine}
              disabled={transcriptionEngineOptions.length <= 1}
              onChange={(event) => {
                void onChangeTranscriptionEngine(
                  event.target.value as TranscriptionEngine,
                );
              }}
            >
              {transcriptionEngineOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>
                {t("settings.transcription.model", "Default model")}
              </strong>
            </div>
            <select
              value={settings.transcription.model}
              onChange={(event) => {
                void onChangeModel(event.target.value as SpeechModel);
              }}
            >
              {modelOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {formatSpeechModelLabel(option.value, option.label)}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>
                {t("settings.transcription.language", "Default language")}
              </strong>
            </div>
            <select
              value={settings.transcription.language}
              onChange={(event) => {
                void onChangeLanguage(event.target.value as LanguageCode);
              }}
            >
              {languageOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(`lang.${option.value}`, option.label)}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row">
            <div>
              <strong>
                {t(
                  "settings.transcription.speakerDiarization",
                  "Enable speaker diarization",
                )}
              </strong>
              <small>
                {t(
                  "settings.transcription.speakerDiarizationDesc",
                  "After transcription completes, the app will run its managed offline pyannote diarization runtime and assign speakers into the timeline when the pyannote assets are installed from Local Models.",
                )}
              </small>
              {pyannoteHealth ? (
                <small className="muted">
                  {formatPyannoteHealthMessage(pyannoteHealth)}
                </small>
              ) : null}
            </div>
            <div className="settings-toggle-stack">
              {pyannoteHealth ? (
                <span
                  className={
                    pyannoteHealth.ready ? "kind-chip" : "missing-chip"
                  }
                >
                  {pyannoteHealth.ready
                    ? t("status.ready", "Ready")
                    : t("status.setupRequired", "Setup required")}
                </span>
              ) : (
                <span
                  className="settings-status-chip-placeholder"
                  aria-hidden="true"
                />
              )}
              <button
                type="button"
                className={`settings-switch ${speakerDiarization.enabled ? "is-on" : "is-off"}`}
                role="switch"
                aria-checked={speakerDiarization.enabled}
                aria-label={t(
                  "settings.transcription.speakerDiarization",
                  "Enable speaker diarization",
                )}
                title={t(
                  "settings.transcription.speakerDiarization",
                  "Enable speaker diarization",
                )}
                onClick={() => {
                  void onPatchSpeakerDiarizationSettings((current) => ({
                    ...current,
                    enabled: !current.enabled,
                  }));
                }}
              >
                <span className="settings-switch-track" aria-hidden="true">
                  <span className="settings-switch-thumb" />
                </span>
              </button>
            </div>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>
                {t("settings.transcription.pyannoteDevice", "Pyannote device")}
              </strong>
              <small>
                {t(
                  "settings.transcription.pyannoteDeviceDesc",
                  "Use CPU by default for best Intel/Apple Silicon compatibility. `auto` will try MPS when available in the managed local runtime.",
                )}
              </small>
            </div>
            <select
              value={speakerDiarization.device}
              onChange={(event) => {
                void onPatchSpeakerDiarizationSettings((current) => ({
                  ...current,
                  device: event.target
                    .value as SpeakerDiarizationSettings["device"],
                }));
              }}
            >
              <option value="cpu">
                {t("settings.transcription.deviceCpu", "CPU")}
              </option>
              <option value="auto">{t("lang.auto", "Auto Detect")}</option>
              <option value="mps">
                {t("settings.transcription.deviceMps", "MPS")}
              </option>
            </select>
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsWhisperCpp(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    const whisperOptions =
      settings.transcription.whisper_options ??
      getDefaultWhisperOptions(platformIsAppleSilicon);

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>{t("settings.whisper.title", "Whisper C++")}</h3>
            <p>
              {t(
                "settings.whisper.desc",
                "Decoding controls used by `whisper-cli` (whisper.cpp).",
              )}
            </p>
          </header>

          <div className="settings-row">
            <div>
              <strong>
                {t(
                  "settings.whisper.translateToEnglish",
                  "Translate transcript to English",
                )}
              </strong>
              <small>
                {t(
                  "settings.whisper.translateDesc",
                  "Equivalent to `--translate`.",
                )}
              </small>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.translate_to_english}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  translate_to_english: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>
                {t("settings.whisper.noContext", "No context between windows")}
              </strong>
              <small>
                {t(
                  "settings.whisper.noContextDesc",
                  "Equivalent to `--max-context 0`.",
                )}
              </small>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.no_context}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  no_context: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>
                {t("settings.whisper.splitOnWord", "Split on word")}
              </strong>
              <small>
                {t(
                  "settings.whisper.splitOnWordDesc",
                  "Use word boundaries when producing segments (`--split-on-word`).",
                )}
              </small>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.split_on_word}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  split_on_word: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>
                {t(
                  "settings.whisper.tinydiarize",
                  "Speaker diarization (tinydiarize)",
                )}
              </strong>
              <small>
                {t(
                  "settings.whisper.tinydiarizeDesc",
                  "Enable whisper.cpp tiny diarization (`-tdrz`) to infer speaker turns.",
                )}
              </small>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.tinydiarize}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  tinydiarize: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>
                {t("settings.whisper.stereodiarize", "Stereo diarization")}
              </strong>
              <small>
                {t(
                  "settings.whisper.stereodiarizeDesc",
                  "Enable whisper.cpp stereo diarization (`-di`) for stereo channel separation.",
                )}
              </small>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.diarize}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  diarize: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.threads", "Threads")}</strong>
              <small>`--threads`</small>
            </div>
            <input
              type="number"
              min={1}
              max={32}
              value={whisperOptions.threads}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  threads: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.processors", "Processors")}</strong>
              <small>`--processors`</small>
            </div>
            <input
              type="number"
              min={1}
              max={16}
              value={whisperOptions.processors}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  processors: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.beamSize", "Beam size")}</strong>
              <small>
                {t(
                  "settings.whisper.beamSizeDesc",
                  "`--beam-size` (when > 1, best-of is ignored).",
                )}
              </small>
            </div>
            <input
              type="number"
              min={1}
              max={20}
              value={whisperOptions.beam_size}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  beam_size: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.bestOf")}</strong>
              <small>
                {t(
                  "settings.whisper.bestOfDesc",
                  "`--best-of` (used when beam size is 1).",
                )}
              </small>
            </div>
            <input
              type="number"
              min={1}
              max={20}
              disabled={whisperOptions.beam_size > 1}
              value={whisperOptions.best_of}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  best_of: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.temperature")}</strong>
              <small>`--temperature`</small>
            </div>
            <input
              type="number"
              min={0}
              max={1}
              step={0.05}
              value={whisperOptions.temperature}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  temperature: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.entropyThreshold")}</strong>
              <small>`--entropy-thold`</small>
            </div>
            <input
              type="number"
              min={0}
              max={10}
              step={0.1}
              value={whisperOptions.entropy_threshold}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  entropy_threshold: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.logprobThreshold")}</strong>
              <small>`--logprob-thold`</small>
            </div>
            <input
              type="number"
              min={-10}
              max={0}
              step={0.1}
              value={whisperOptions.logprob_threshold}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  logprob_threshold: Number(event.target.value),
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisper.wordThreshold")}</strong>
              <small>`--word-thold`</small>
            </div>
            <input
              type="number"
              min={0}
              max={1}
              step={0.01}
              value={whisperOptions.word_threshold}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  word_threshold: Number(event.target.value),
                }));
              }}
            />
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsWhisperKit(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    const whisperOptions =
      settings.transcription.whisper_options ??
      getDefaultWhisperOptions(platformIsAppleSilicon);

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>{t("settings.whisperkit.title")}</h3>
            <p>{t("settings.whisperkit.desc")}</p>
          </header>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisperkit.chunkingStrategy")}</strong>
            </div>
            <select
              value={whisperOptions.chunking_strategy}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  chunking_strategy: event.target
                    .value as WhisperOptions["chunking_strategy"],
                }));
              }}
            >
              {chunkingOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {t(
                    `settings.whisperkit.chunking.${option.value}`,
                    option.label,
                  )}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisperkit.concurrentWorkers")}</strong>
              <small>{t("settings.whisperkit.concurrentWorkersDesc")}</small>
            </div>
            <input
              type="number"
              min={1}
              max={16}
              value={whisperOptions.concurrent_worker_count}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  concurrent_worker_count: Number(event.target.value),
                }));
              }}
            />
          </div>

          {platformIsAppleSilicon && (
            <div className="settings-row settings-row-block">
              <div>
                <strong>{t("settings.whisperkit.audioEncoderUnits")}</strong>
                <small>{t("settings.whisperkit.audioEncoderUnitsDesc")}</small>
              </div>
              <select
                value={whisperOptions.audio_encoder_compute_units}
                onChange={(event) => {
                  void onPatchWhisperOptions((current) => ({
                    ...current,
                    audio_encoder_compute_units: event.target
                      .value as WhisperOptions["audio_encoder_compute_units"],
                  }));
                }}
              >
                {computeUnitOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {t(
                      `settings.whisperkit.compute.${option.value}`,
                      option.label,
                    )}
                  </option>
                ))}
              </select>
            </div>
          )}

          {platformIsAppleSilicon && (
            <div className="settings-row settings-row-block">
              <div>
                <strong>{t("settings.whisperkit.textDecoderUnits")}</strong>
                <small>{t("settings.whisperkit.textDecoderUnitsDesc")}</small>
              </div>
              <select
                value={whisperOptions.text_decoder_compute_units}
                onChange={(event) => {
                  void onPatchWhisperOptions((current) => ({
                    ...current,
                    text_decoder_compute_units: event.target
                      .value as WhisperOptions["text_decoder_compute_units"],
                  }));
                }}
              >
                {computeUnitOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {t(
                      `settings.whisperkit.compute.${option.value}`,
                      option.label,
                    )}
                  </option>
                ))}
              </select>
            </div>
          )}

          <div className="settings-row">
            <div>
              <strong>{t("settings.whisperkit.usePrefillPrompt")}</strong>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.use_prefill_prompt}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  use_prefill_prompt: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>{t("settings.whisperkit.usePrefillCache")}</strong>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.use_prefill_cache}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  use_prefill_cache: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>{t("settings.whisperkit.withoutTimestamps")}</strong>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.without_timestamps}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  without_timestamps: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row">
            <div>
              <strong>{t("settings.whisperkit.wordTimestamps")}</strong>
            </div>
            <input
              type="checkbox"
              checked={whisperOptions.word_timestamps}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  word_timestamps: event.target.checked,
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>{t("settings.whisperkit.promptOverride")}</strong>
              <small>{t("settings.whisperkit.promptOverrideDesc")}</small>
            </div>
            <textarea
              className="settings-textarea small"
              value={whisperOptions.prompt ?? ""}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  prompt: event.target.value,
                }));
              }}
              placeholder={t(
                "settings.whisperkit.promptPlaceholder",
                "Optional decoder prompt",
              )}
            />
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsLocalModels(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }
    const pyannoteHealth = runtimeHealth?.pyannote ?? provisioning.pyannote;
    const runtimeBusy =
      provisioning.running &&
      provisioning.progress?.asset_kind === "speech_runtime";
    const pyannoteBusy =
      provisioning.running &&
      (provisioning.progress?.asset_kind === "pyannote_runtime" ||
        provisioning.progress?.asset_kind === "pyannote_model");

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <div className="settings-card-head">
            <h3>{t("settings.localModels.title")}</h3>
            <button
              className="secondary-button"
              onClick={() => void refreshProvisioningModels()}
            >
              {t("action.refresh", "Refresh")}
            </button>
          </div>

          <p className="muted">
            {t(
              "settings.localModels.modelsDownloaded",
              "Models are downloaded in background and used by local transcription.",
            )}
          </p>
          <p className="muted">
            {t("settings.localModels.directory", "Directory:")}{" "}
            <code>
              {provisioning.modelsDir || settings.transcription.models_dir}
            </code>
          </p>

          {runtimeHealth ? (
            <div className="settings-health-block">
              <h4>{t("settings.localModels.runtimeHealth")}</h4>
              <div className="settings-health-rows">
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.localModels.platform")}
                  </span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.host_os}</code>
                    <code>{runtimeHealth.host_arch}</code>
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.localModels.enginePolicy")}
                  </span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.preferred_engine}</code>
                    {runtimeHealth.configured_engine ===
                    runtimeHealth.preferred_engine ? (
                      <span className="kind-chip">
                        {t("settings.localModels.aligned")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t("settings.localModels.willAutoFix")}
                      </span>
                    )}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.localModels.runtimeSource", "Runtime source")}
                  </span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.runtime_source || "unknown"}</code>
                    {runtimeHealth.managed_runtime_required ? (
                      <span className="kind-chip">
                        {t("settings.localModels.managedOnly", "Managed only")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t(
                          "settings.localModels.devFallbacksAllowed",
                          "Fallbacks allowed",
                        )}
                      </span>
                    )}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.advanced.ffmpegPath", "FFmpeg path")}
                  </span>
                  <span className="settings-health-value-inline settings-health-value-stack">
                    <code className="settings-health-value">
                      {runtimeHealth.ffmpeg_resolved ||
                        runtimeHealth.ffmpeg_path}
                    </code>
                    {runtimeHealth.ffmpeg_available ? (
                      <span className="kind-chip">
                        {t("settings.localModels.runnable")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t("settings.localModels.unavailable")}
                      </span>
                    )}
                    {!runtimeHealth.ffmpeg_available &&
                    runtimeHealth.managed_runtime?.ffmpeg.failure_message ? (
                      <span className="settings-health-detail">
                        {runtimeHealth.managed_runtime.ffmpeg.failure_message}
                      </span>
                    ) : null}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.localModels.whisperCli")}
                  </span>
                  <span className="settings-health-value-inline settings-health-value-stack">
                    <code className="settings-health-value">
                      {runtimeHealth.whisper_cli_resolved ||
                        runtimeHealth.whisper_cli_path}
                    </code>
                    {runtimeHealth.whisper_cli_available ? (
                      <span className="kind-chip">
                        {t("settings.localModels.runnable")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t("settings.localModels.unavailable")}
                      </span>
                    )}
                    {!runtimeHealth.whisper_cli_available &&
                    runtimeHealth.managed_runtime?.whisper_cli
                      .failure_message ? (
                      <span className="settings-health-detail">
                        {
                          runtimeHealth.managed_runtime.whisper_cli
                            .failure_message
                        }
                      </span>
                    ) : null}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.localModels.whisperStream")}
                  </span>
                  <span className="settings-health-value-inline settings-health-value-stack">
                    <code className="settings-health-value">
                      {runtimeHealth.whisper_stream_resolved ||
                        runtimeHealth.whisper_stream_path}
                    </code>
                    {runtimeHealth.whisper_stream_available ? (
                      <span className="kind-chip">
                        {t("settings.localModels.runnable")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t("settings.localModels.unavailable")}
                      </span>
                    )}
                    {!runtimeHealth.whisper_stream_available &&
                    runtimeHealth.managed_runtime?.whisper_stream
                      .failure_message ? (
                      <span className="settings-health-detail">
                        {
                          runtimeHealth.managed_runtime.whisper_stream
                            .failure_message
                        }
                      </span>
                    ) : null}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.localModels.activeModel")}
                  </span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.model_filename}</code>
                    {runtimeHealth.model_present ? (
                      <span className="kind-chip">
                        {t("settings.localModels.installed")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t("settings.localModels.missing")}
                      </span>
                    )}
                  </span>
                </div>
                {platformIsAppleSilicon && (
                  <div className="settings-health-row">
                    <span className="settings-health-label">
                      {t("settings.localModels.coremlEncoder")}
                    </span>
                    {runtimeHealth.coreml_encoder_present ? (
                      <span className="kind-chip">
                        {t("settings.localModels.installed")}
                      </span>
                    ) : (
                      <span className="missing-chip">
                        {t("settings.localModels.missing")}
                      </span>
                    )}
                  </div>
                )}
              </div>
            </div>
          ) : null}

          <div className="model-list compact-list">
            {modelCatalog.map((model) => (
              <div key={model.key} className="model-row">
                <div className="model-row-main">
                  <strong>
                    {formatSpeechModelLabel(model.key, model.label)}
                  </strong>
                  <small>{model.model_file}</small>
                </div>
                <div className="model-row-actions">
                  <span
                    className={model.installed ? "kind-chip" : "missing-chip"}
                  >
                    {model.installed
                      ? t("status.installed", "Installed")
                      : t("status.missing", "Missing")}
                  </span>
                  {platformIsAppleSilicon && (
                    <span
                      className={
                        model.coreml_installed ? "kind-chip" : "missing-chip"
                      }
                    >
                      {model.coreml_installed
                        ? t("settings.localModels.coremlReady", "CoreML Ready")
                        : t(
                            "settings.localModels.coremlMissing",
                            "CoreML Missing",
                          )}
                    </span>
                  )}
                  <button
                    className="secondary-button"
                    disabled={
                      provisioning.running ||
                      (model.installed &&
                        (!platformIsAppleSilicon || model.coreml_installed))
                    }
                    onClick={() => void onDownloadModel(model.key)}
                  >
                    {model.installed &&
                    (!platformIsAppleSilicon || model.coreml_installed)
                      ? t("status.installed", "Installed")
                      : t("settings.localModels.download", "Download")}
                  </button>
                </div>
              </div>
            ))}
          </div>

          <div className="notice-actions">
            <button
              className="secondary-button"
              onClick={() => void onInstallRuntime(false)}
              disabled={provisioning.running || runtimeToolchainReady}
            >
              {t(
                "settings.localModels.installRuntime",
                "Install Local Runtime",
              )}
            </button>
            <button
              className="secondary-button"
              onClick={() => void onInstallRuntime(true)}
              disabled={provisioning.running}
            >
              {t("settings.localModels.repairRuntime", "Repair Runtime")}
            </button>
            <button
              className="primary-button"
              onClick={() => void onProvisionModels()}
              disabled={provisioning.running}
            >
              {t(
                "settings.localModels.downloadMissing",
                "Download Missing Models",
              )}
            </button>
            <button
              className="secondary-button"
              onClick={() => void onCancelProvisioning()}
              disabled={!provisioning.running}
            >
              {t("action.cancel", "Cancel")}
            </button>
          </div>

          {provisioning.progress ? (
            <div className="inline-progress">
              <div style={{ width: `${provisioning.progress.percentage}%` }} />
            </div>
          ) : null}
          {(runtimeBusy ||
            provisioning.progress?.asset_kind !== "speech_runtime") &&
          provisioning.statusMessage ? (
            <small className="muted">{provisioning.statusMessage}</small>
          ) : null}
        </section>

        <section className="settings-panel">
          <div className="settings-card-head">
            <h3>{t("settings.pyannote.title", "Speaker Diarization")}</h3>
            <button
              className="secondary-button"
              onClick={() => void refreshRuntimeHealth()}
            >
              {t("action.refresh", "Refresh")}
            </button>
          </div>

          <p className="muted">
            {t(
              "settings.pyannote.desc",
              "Install the managed offline pyannote runtime once, then file transcription can assign speakers fully offline.",
            )}
          </p>
          <p className="muted">
            {t("settings.localModels.directory", "Directory:")}{" "}
            <code>
              {provisioning.modelsDir
                ? `${provisioning.modelsDir}/../runtime/pyannote`
                : "runtime/pyannote"}
            </code>
          </p>

          {pyannoteHealth ? (
            <div className="settings-health-block">
              <div className="settings-health-rows">
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.pyannote.arch", "Architecture")}
                  </span>
                  <span className="settings-health-value-inline">
                    <code>{pyannoteHealth.arch}</code>
                    <span
                      className={
                        pyannoteHealth.ready ? "kind-chip" : "missing-chip"
                      }
                    >
                      {pyannoteHealth.ready
                        ? t("settings.pyannote.ready", "Ready")
                        : t("settings.pyannote.missing", "Missing")}
                    </span>
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.pyannote.runtime", "Runtime")}
                  </span>
                  <span
                    className={
                      pyannoteHealth.runtime_installed
                        ? "kind-chip"
                        : "missing-chip"
                    }
                  >
                    {pyannoteHealth.runtime_installed
                      ? t("status.installed", "Installed")
                      : t("status.missing", "Missing")}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.pyannote.model", "Model")}
                  </span>
                  <span
                    className={
                      pyannoteHealth.model_installed
                        ? "kind-chip"
                        : "missing-chip"
                    }
                  >
                    {pyannoteHealth.model_installed
                      ? t("status.installed", "Installed")
                      : t("status.missing", "Missing")}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.pyannote.device", "Configured device")}
                  </span>
                  <code>{pyannoteHealth.device || "cpu"}</code>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">
                    {t("settings.pyannote.source", "Source")}
                  </span>
                  <code>{pyannoteHealth.source}</code>
                </div>
              </div>
              <small className="muted">
                {formatPyannoteHealthMessage(pyannoteHealth)}
              </small>
            </div>
          ) : null}

          <div className="notice-actions">
            <button
              className="primary-button"
              onClick={() => void onInstallPyannote(false)}
              disabled={provisioning.running || Boolean(pyannoteHealth?.ready)}
            >
              {t("settings.pyannote.install", "Install Pyannote")}
            </button>
            <button
              className="secondary-button"
              onClick={() => void onInstallPyannote(true)}
              disabled={provisioning.running}
            >
              {t("settings.pyannote.repair", "Repair")}
            </button>
            <button
              className="secondary-button"
              onClick={() => void onCancelProvisioning()}
              disabled={!provisioning.running}
            >
              {t("action.cancel", "Cancel")}
            </button>
          </div>

          {pyannoteBusy && provisioning.progress ? (
            <div className="inline-progress">
              <div style={{ width: `${provisioning.progress.percentage}%` }} />
            </div>
          ) : null}
          {pyannoteBusy && provisioning.statusMessage ? (
            <small className="muted">{provisioning.statusMessage}</small>
          ) : null}
        </section>
      </div>
    );
  }

  function renderSettingsAiServices(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    const foundationAvailable = platformIsAppleSilicon;
    const foundationEnabled = settings.ai.providers.foundation_apple.enabled;
    const foundationActive = settings.ai.active_provider === "foundation_apple";
    const geminiActive =
      settings.ai.active_provider === "gemini" &&
      Boolean(settings.ai.active_remote_service_id) &&
      settings.ai.remote_services.some(
        (service) =>
          service.id === settings.ai.active_remote_service_id &&
          service.kind === "google",
      );
    const geminiConfigured = Boolean(settings.ai.providers.gemini.has_api_key);
    const remoteServices = settings.ai.remote_services ?? [];
    const googleService = remoteServices.find(
      (service) => service.kind === "google",
    );
    const hasGoogleService = Boolean(googleService);
    const showGeminiService = hasGoogleService;
    const showGeminiConfig = aiServiceConfigOpen === googleService?.id;
    const configuredKinds = new Set(
      remoteServices.map((service) => service.kind),
    );

    const createRemoteServiceEntry = (
      kind: RemoteServiceKind,
    ): RemoteServiceConfig | null => {
      const catalog = serviceCatalog.find((item) => item.kind === kind);
      if (!catalog) return null;
      return {
        id: createRemoteServiceId(kind),
        kind,
        label: formatProviderLabel(kind),
        enabled: true,
        api_key: null,
        has_api_key:
          kind === "google" ? settings.ai.providers.gemini.has_api_key : false,
        model:
          kind === "google"
            ? settings.ai.providers.gemini.model
            : catalog.defaultModel,
        base_url: catalog.defaultBaseUrl,
      };
    };

    const activateFoundation = (): void => {
      void patchAiSettings((current) => ({
        ...current,
        active_provider: "foundation_apple",
        providers: {
          ...current.providers,
          foundation_apple: {
            ...current.providers.foundation_apple,
            enabled: true,
          },
        },
      }));
    };

    const activateGemini = (): void => {
      if (!googleService) {
        return;
      }

      void patchAiSettings((current) => ({
        ...current,
        active_provider: "gemini",
        active_remote_service_id: googleService.id,
      }));
    };

    const addRemoteService = (kind: RemoteServiceKind): void => {
      const entry = createRemoteServiceEntry(kind);
      if (!entry) return;
      if (configuredKinds.has(kind)) {
        const existing = remoteServices.find(
          (service) => service.kind === kind,
        );
        if (existing) {
          setAiServiceConfigOpen(existing.id);
        }
        return;
      }

      void patchAiSettings((current) => ({
        ...current,
        remote_services: [...(current.remote_services ?? []), entry],
      }));
      setAiServiceConfigOpen(entry.id);
    };

    const removeRemoteService = (id: string): void => {
      void patchAiSettings((current) => {
        const target = (current.remote_services ?? []).find(
          (service) => service.id === id,
        );
        const nextServices = (current.remote_services ?? []).filter(
          (service) => service.id !== id,
        );
        const removingActiveRemote = current.active_remote_service_id === id;
        const shouldDeactivateGemini =
          target?.kind === "google" && current.active_provider === "gemini";

        return {
          ...current,
          active_provider: shouldDeactivateGemini
            ? current.providers.foundation_apple.enabled &&
              platformIsAppleSilicon
              ? "foundation_apple"
              : "none"
            : current.active_provider,
          active_remote_service_id: removingActiveRemote
            ? null
            : current.active_remote_service_id,
          remote_services: nextServices,
        };
      });
      setRemoteServiceApiKeyDrafts((current) => {
        if (!(id in current)) {
          return current;
        }
        const next = { ...current };
        delete next[id];
        return next;
      });
      if (aiServiceConfigOpen === id) {
        setAiServiceConfigOpen(null);
      }
    };

    const patchRemoteService = (
      id: string,
      mutator: (service: RemoteServiceConfig) => RemoteServiceConfig,
    ): void => {
      void patchAiSettings((current) => ({
        ...current,
        remote_services: (current.remote_services ?? []).map((service) =>
          service.id === id ? mutator(service) : service,
        ),
      }));
    };

    return (
      <div className="settings-stack ai-services-stack">
        <section className="settings-panel ai-services-panel">
          <h3>{t("settings.ai.services")}</h3>

          <div className="ai-services-notice">
            <p>
              {t(
                "settings.ai.privacyNotice",
                "When you use remote AI services, your transcript will be sent to the service's servers and won't stay only on your Mac.",
              )}
            </p>
            <button
              className={
                aiServicesAcknowledged
                  ? "secondary-button ai-notice-button acknowledged"
                  : "secondary-button ai-notice-button"
              }
              onClick={() => setAiServicesAcknowledged(true)}
            >
              {aiServicesAcknowledged
                ? t("settings.ai.understood", "Understood")
                : t("settings.ai.iUnderstand", "I Understand")}
            </button>
          </div>

          <article
            className={
              foundationActive ? "ai-service-card active" : "ai-service-card"
            }
          >
            <div className="ai-service-row">
              <span className="ai-service-icon foundation"></span>
              <div className="ai-service-title">
                <strong>{t("settings.ai.foundationModel")}</strong>
                <small>
                  {foundationAvailable
                    ? t("settings.ai.apple", "Apple")
                    : t(
                        "settings.ai.requiresAppleSilicon",
                        "Requires Apple Silicon",
                      )}
                </small>
              </div>
              <div className="ai-service-actions">
                <label className="toggle-row compact">
                  <span>{t("settings.ai.enabled")}</span>
                  <input
                    type="checkbox"
                    checked={foundationEnabled}
                    disabled={!foundationAvailable}
                    onChange={(event) => {
                      const enabled = event.target.checked;
                      void patchAiSettings((current) => ({
                        ...current,
                        active_provider:
                          !enabled &&
                          current.active_provider === "foundation_apple"
                            ? current.providers.gemini.has_api_key
                              ? "gemini"
                              : "none"
                            : current.active_provider,
                        providers: {
                          ...current.providers,
                          foundation_apple: {
                            ...current.providers.foundation_apple,
                            enabled,
                          },
                        },
                      }));
                    }}
                  />
                </label>
                <button
                  className="secondary-button"
                  disabled={
                    !foundationAvailable ||
                    !foundationEnabled ||
                    foundationActive
                  }
                  onClick={activateFoundation}
                >
                  {foundationActive
                    ? t("settings.ai.active", "Active")
                    : t("settings.ai.use", "Use")}
                </button>
              </div>
            </div>
          </article>

          {showGeminiService ? (
            <article
              className={
                geminiActive ? "ai-service-card active" : "ai-service-card"
              }
            >
              <div className="ai-service-row">
                <span className="ai-service-icon gemini">
                  <Sparkles size={13} />
                </span>
                <div className="ai-service-title">
                  <strong>{settings.ai.providers.gemini.model}</strong>
                  <small>
                    {geminiConfigured
                      ? t("settings.ai.configuredToUse", "Configured to use")
                      : t("settings.ai.configureToUse", "Configure to use")}
                  </small>
                </div>
                <div className="ai-service-actions">
                  <button
                    className="secondary-button"
                    onClick={() =>
                      setAiServiceConfigOpen(
                        showGeminiConfig ? null : (googleService?.id ?? null),
                      )
                    }
                  >
                    {showGeminiConfig
                      ? t("settings.ai.done", "Done")
                      : t("settings.ai.configure", "Configure")}
                  </button>
                  <button
                    className="secondary-button"
                    disabled={!geminiConfigured || geminiActive}
                    onClick={activateGemini}
                  >
                    {geminiActive
                      ? t("settings.ai.active", "Active")
                      : t("settings.ai.use", "Use")}
                  </button>
                </div>
              </div>

              {showGeminiConfig ? (
                <div className="ai-service-config">
                  <label>
                    {t("settings.ai.apiKey", "API Key")}
                    <input
                      type="password"
                      placeholder={t(
                        "settings.ai.apiKeyPlaceholder",
                        "Enter API Key...",
                      )}
                      value={geminiApiKeyDraft}
                      onChange={(event) => {
                        const value = event.target.value.trim();
                        setGeminiApiKeyDraft(event.target.value);
                        void patchAiSettings((current) => ({
                          ...current,
                          providers: {
                            ...current.providers,
                            gemini: {
                              ...current.providers.gemini,
                              api_key:
                                value.length > 0 ? event.target.value : null,
                              has_api_key: value.length > 0,
                            },
                          },
                          remote_services: (current.remote_services ?? []).map(
                            (service) =>
                              service.kind === "google"
                                ? {
                                    ...service,
                                    api_key:
                                      value.length > 0
                                        ? event.target.value
                                        : null,
                                    has_api_key: value.length > 0,
                                  }
                                : service,
                          ),
                        }));
                      }}
                    />
                  </label>

                  <div className="ai-service-config-row">
                    <label>
                      {t("settings.ai.geminiModel", "Gemini Model")}
                      <select
                        value={settings.ai.providers.gemini.model}
                        onChange={(event) => {
                          void patchAiSettings((current) => ({
                            ...current,
                            providers: {
                              ...current.providers,
                              gemini: {
                                ...current.providers.gemini,
                                model: event.target.value,
                              },
                            },
                            remote_services: (
                              current.remote_services ?? []
                            ).map((service) =>
                              service.kind === "google"
                                ? {
                                    ...service,
                                    model: event.target.value,
                                  }
                                : service,
                            ),
                          }));
                        }}
                      >
                        {geminiModelChoices.map((model) => (
                          <option key={model} value={model}>
                            {model}
                          </option>
                        ))}
                      </select>
                    </label>
                    <div className="ai-service-config-links">
                      <button
                        className="secondary-button"
                        disabled={loadingGeminiModels}
                        onClick={() =>
                          setGeminiModelFetchNonce((value) => value + 1)
                        }
                      >
                        {loadingGeminiModels
                          ? t("settings.ai.loadingModels", "Loading models...")
                          : t("settings.ai.refreshModels", "Refresh models")}
                      </button>
                      <a
                        className="cta-link-button ai-service-link"
                        href="https://aistudio.google.com/apikey"
                        target="_blank"
                        rel="noreferrer"
                      >
                        {t(
                          "settings.ai.createGoogleKey",
                          "Create a Google API Key",
                        )}
                      </a>
                    </div>
                  </div>

                  <div className="ai-service-config-actions">
                    <button
                      className="secondary-button"
                      onClick={() => {
                        void patchAiSettings((current) => {
                          const nextProvider =
                            current.active_provider === "gemini"
                              ? current.providers.foundation_apple.enabled &&
                                platformIsAppleSilicon
                                ? "foundation_apple"
                                : "none"
                              : current.active_provider;

                          return {
                            ...current,
                            active_provider: nextProvider,
                            active_remote_service_id:
                              current.active_remote_service_id ===
                              googleService?.id
                                ? null
                                : current.active_remote_service_id,
                            providers: {
                              ...current.providers,
                              gemini: {
                                ...current.providers.gemini,
                                api_key: null,
                                has_api_key: false,
                              },
                            },
                            remote_services: (
                              current.remote_services ?? []
                            ).filter((service) => service.kind !== "google"),
                          };
                        });
                        setGeminiApiKeyDraft("");
                        setAiServiceConfigOpen(null);
                      }}
                    >
                      {t("action.delete", "Delete")}
                    </button>
                  </div>
                </div>
              ) : null}
            </article>
          ) : null}

          {remoteServices
            .filter((service) => service.kind !== "google")
            .map((service) => {
              const catalog = serviceCatalog.find(
                (item) => item.kind === service.kind,
              );
              const ServiceIcon = catalog?.icon ?? Settings2;
              const isOpen = aiServiceConfigOpen === service.id;
              const isActiveRemote =
                settings.ai.active_remote_service_id === service.id &&
                settings.ai.active_provider !== "foundation_apple";

              return (
                <article
                  key={service.id}
                  className={
                    isActiveRemote
                      ? "ai-service-card active"
                      : "ai-service-card"
                  }
                >
                  <div className="ai-service-row">
                    <span
                      className={`ai-service-icon ${catalog?.tone ?? "custom"}`}
                    >
                      <ServiceIcon size={13} />
                    </span>
                    <div className="ai-service-title">
                      <strong>
                        {formatRemoteServiceLabel(service, settings)}
                      </strong>
                      <small>
                        {service.enabled
                          ? t("settings.ai.configuredToUse", "Configured")
                          : t("settings.ai.disabled", "Disabled")}
                      </small>
                    </div>
                    <div className="ai-service-actions">
                      <label className="toggle-row compact">
                        <span>{t("settings.ai.enabled")}</span>
                        <input
                          type="checkbox"
                          checked={service.enabled}
                          onChange={(event) => {
                            const enabled = event.target.checked;
                            void patchAiSettings((current) => {
                              const nextServices = (
                                current.remote_services ?? []
                              ).map((entry) =>
                                entry.id === service.id
                                  ? {
                                      ...entry,
                                      enabled,
                                    }
                                  : entry,
                              );
                              const disablingActive =
                                !enabled &&
                                current.active_remote_service_id === service.id;
                              const nextProvider = disablingActive
                                ? current.providers.foundation_apple.enabled &&
                                  platformIsAppleSilicon
                                  ? "foundation_apple"
                                  : "none"
                                : current.active_provider;

                              return {
                                ...current,
                                active_provider: nextProvider,
                                active_remote_service_id: disablingActive
                                  ? null
                                  : current.active_remote_service_id,
                                remote_services: nextServices,
                              };
                            });
                          }}
                        />
                      </label>
                      <button
                        className="secondary-button"
                        onClick={() =>
                          setAiServiceConfigOpen(isOpen ? null : service.id)
                        }
                      >
                        {isOpen
                          ? t("settings.ai.done", "Done")
                          : t("settings.ai.configure", "Configure")}
                      </button>
                      <button
                        className="secondary-button"
                        disabled={!service.enabled || isActiveRemote}
                        onClick={() => {
                          void patchAiSettings((current) => ({
                            ...current,
                            active_provider:
                              service.kind === "google" ? "gemini" : "none",
                            active_remote_service_id: service.id,
                          }));
                        }}
                      >
                        {isActiveRemote
                          ? t("settings.ai.active", "Active")
                          : t("settings.ai.use", "Use")}
                      </button>
                    </div>
                  </div>

                  {isOpen ? (
                    <div className="ai-service-config">
                      <label>
                        {t("settings.ai.apiKey", "API Key")}
                        <input
                          type="password"
                          value={remoteServiceApiKeyDrafts[service.id] ?? ""}
                          onChange={(event) => {
                            const nextValue = event.target.value;
                            setRemoteServiceApiKeyDrafts((current) => ({
                              ...current,
                              [service.id]: nextValue,
                            }));
                            patchRemoteService(service.id, (current) => ({
                              ...current,
                              api_key:
                                nextValue.trim().length > 0 ? nextValue : null,
                              has_api_key: nextValue.trim().length > 0,
                            }));
                          }}
                        />
                      </label>
                      <label>
                        {t("settings.ai.model", "Model")}
                        <input
                          value={service.model ?? ""}
                          placeholder={t(
                            "settings.ai.modelPlaceholder",
                            "Optional model name",
                          )}
                          onChange={(event) =>
                            patchRemoteService(service.id, (current) => ({
                              ...current,
                              model:
                                event.target.value.trim().length > 0
                                  ? event.target.value
                                  : null,
                            }))
                          }
                        />
                      </label>
                      <label>
                        {t("settings.ai.baseUrl", "Base URL")}
                        <input
                          value={service.base_url ?? ""}
                          placeholder={t(
                            "settings.ai.baseUrlPlaceholder",
                            "Optional API endpoint",
                          )}
                          onChange={(event) =>
                            patchRemoteService(service.id, (current) => ({
                              ...current,
                              base_url:
                                event.target.value.trim().length > 0
                                  ? event.target.value
                                  : null,
                            }))
                          }
                        />
                      </label>
                      <div className="ai-service-config-actions">
                        <button
                          className="secondary-button"
                          onClick={() => removeRemoteService(service.id)}
                        >
                          {t("action.delete", "Delete")}
                        </button>
                      </div>
                    </div>
                  ) : null}
                </article>
              );
            })}

          <div className="ai-service-library">
            <strong>{t("settings.ai.addService")}</strong>
            <div className="ai-service-grid">
              {serviceCatalog.map((service) => {
                const ServiceIcon = service.icon;
                const isGoogle = service.kind === "google";
                const alreadyAdded = isGoogle
                  ? hasGoogleService
                  : configuredKinds.has(service.kind);
                return (
                  <button
                    key={service.kind}
                    className={`ai-service-chip available ${service.tone}`}
                    onClick={() => addRemoteService(service.kind)}
                  >
                    <span className="ai-service-chip-main">
                      <ServiceIcon size={13} />
                      <span>
                        {service.kind === "custom"
                          ? t("settings.ai.customService", "Custom")
                          : service.label}
                      </span>
                    </span>
                    {alreadyAdded ? <Check size={14} /> : <Plus size={14} />}
                  </button>
                );
              })}
            </div>
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsPrompts(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    return (
      <div className="settings-prompts-layout">
        <aside className="prompt-sidebar">
          <div className="settings-card-head">
            <h3>{t("settings.prompts.templates")}</h3>
            <button
              className="secondary-button"
              onClick={() => void onResetPrompts()}
            >
              {t("action.reset", "Reset")}
            </button>
          </div>

          <div className="prompt-template-list">
            {settings.prompts.templates.map((template) => (
              <button
                key={template.id}
                className={
                  template.id === activePromptId
                    ? "prompt-template-row active"
                    : "prompt-template-row"
                }
                onClick={() => setActivePromptId(template.id)}
              >
                <span>{template.name}</span>
                {template.builtin ? (
                  <small>{t("settings.prompts.builtIn")}</small>
                ) : (
                  <small>{t("settings.prompts.custom")}</small>
                )}
              </button>
            ))}
          </div>

          <div className="settings-card-block compact">
            <h4>{t("settings.prompts.bindings")}</h4>
            <label>
              {t("settings.prompts.optimize", "Optimize")}
              <select
                value={settings.prompts.bindings.optimize_prompt_id}
                onChange={(event) => {
                  const value = event.target.value;
                  void patchSettings((current) => ({
                    ...current,
                    prompts: {
                      ...current.prompts,
                      bindings: {
                        ...current.prompts.bindings,
                        optimize_prompt_id: value,
                      },
                    },
                  }));
                }}
              >
                {settings.prompts.templates.map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {t("settings.prompts.summary", "Summary")}
              <select
                value={settings.prompts.bindings.summary_prompt_id}
                onChange={(event) => {
                  const value = event.target.value;
                  void patchSettings((current) => ({
                    ...current,
                    prompts: {
                      ...current.prompts,
                      bindings: {
                        ...current.prompts.bindings,
                        summary_prompt_id: value,
                      },
                    },
                  }));
                }}
              >
                {settings.prompts.templates.map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {t("settings.prompts.faq", "FAQ")}
              <select
                value={settings.prompts.bindings.faq_prompt_id}
                onChange={(event) => {
                  const value = event.target.value;
                  void patchSettings((current) => ({
                    ...current,
                    prompts: {
                      ...current.prompts,
                      bindings: {
                        ...current.prompts.bindings,
                        faq_prompt_id: value,
                      },
                    },
                  }));
                }}
              >
                {settings.prompts.templates.map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {t("settings.prompts.emotion", "Emotion Analysis")}
              <select
                value={settings.prompts.bindings.emotion_prompt_id}
                onChange={(event) => {
                  const value = event.target.value;
                  void patchSettings((current) => ({
                    ...current,
                    prompts: {
                      ...current.prompts,
                      bindings: {
                        ...current.prompts.bindings,
                        emotion_prompt_id: value,
                      },
                    },
                  }));
                }}
              >
                {settings.prompts.templates.map((template) => (
                  <option key={template.id} value={template.id}>
                    {template.name}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </aside>

        <section className="settings-card-block prompt-editor-pane">
          {!promptDraft ? (
            <div className="settings-placeholder">
              {t("settings.prompts.selectTemplate")}
            </div>
          ) : (
            <>
              <div className="settings-card-head">
                <h3>{t("settings.prompts.editPrompt")}</h3>
                <button
                  className="primary-button"
                  onClick={() => void onSavePromptTemplate()}
                >
                  <Save size={14} />
                  {t("settings.prompts.savePrompt", "Save Prompt")}
                </button>
              </div>

              <label>
                {t("settings.prompts.promptTitle", "Title")}
                <input
                  value={promptDraft.name}
                  onChange={(event) =>
                    setPromptDraft((current) =>
                      current
                        ? { ...current, name: event.target.value }
                        : current,
                    )
                  }
                />
              </label>

              <label>
                {t("settings.prompts.promptBody", "Prompt Body")}
                <textarea
                  className="settings-textarea"
                  value={promptDraft.body}
                  onChange={(event) =>
                    setPromptDraft((current) =>
                      current
                        ? { ...current, body: event.target.value }
                        : current,
                    )
                  }
                />
              </label>

              <div className="prompt-test-row">
                <label>
                  {t("settings.prompts.testTask", "Test task")}
                  <select
                    value={promptBindingTask}
                    onChange={(event) =>
                      setPromptBindingTask(event.target.value as PromptTask)
                    }
                  >
                    {promptTaskOptions.map((task) => (
                      <option key={task.value} value={task.value}>
                        {t(`settings.prompts.task.${task.value}`, task.label)}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  className="secondary-button"
                  onClick={() => void onRunPromptTest()}
                  disabled={promptTest.running}
                >
                  {promptTest.running
                    ? t("settings.prompts.testing", "Testing...")
                    : t("settings.prompts.runTest", "Run Test")}
                </button>
              </div>

              <label>
                {t("settings.prompts.testInput", "Test input")}
                <textarea
                  className="settings-textarea small"
                  value={promptTest.input}
                  onChange={(event) =>
                    setPromptTest((current) => ({
                      ...current,
                      input: event.target.value,
                    }))
                  }
                />
              </label>

              <label>
                {t("settings.prompts.output", "Output")}
                <textarea
                  className="settings-textarea small"
                  value={promptTest.output}
                  readOnly
                />
              </label>
            </>
          )}
        </section>
      </div>
    );
  }

  function renderSettingsAdvanced(): JSX.Element {
    if (!settings) {
      return (
        <div className="settings-placeholder">{t("settings.unavailable")}</div>
      );
    }

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <h3>{t("settings.advanced.title")}</h3>

          <label>
            {t("settings.advanced.whisperCliPath", "Whisper CLI path")}
            <input
              value={settings.transcription.whisper_cli_path}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  transcription: {
                    ...current.transcription,
                    whisper_cli_path: event.target.value,
                  },
                }));
              }}
            />
          </label>

          <label>
            {t("settings.advanced.whisperStreamPath", "Whisper Stream path")}
            <input
              value={settings.transcription.whisperkit_cli_path}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  transcription: {
                    ...current.transcription,
                    whisperkit_cli_path: event.target.value,
                  },
                }));
              }}
            />
          </label>

          <label>
            {t("settings.advanced.ffmpegPath", "FFmpeg path")}
            <input
              value={settings.transcription.ffmpeg_path}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  transcription: {
                    ...current.transcription,
                    ffmpeg_path: event.target.value,
                  },
                }));
              }}
            />
          </label>

          <label>
            {t("settings.advanced.modelsDir", "Models directory")}
            <input
              value={settings.transcription.models_dir}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  transcription: {
                    ...current.transcription,
                    models_dir: event.target.value,
                  },
                }));
              }}
            />
          </label>

          <div className="notice-actions">
            <button
              className="secondary-button"
              onClick={() => void refreshSettingsFromDisk()}
            >
              {t("settings.advanced.reloadFromDisk", "Reload from disk")}
            </button>
          </div>
        </section>

        <section className="settings-panel">
          <h3>{t("settings.advanced.backupSection", "Backup & Restore")}</h3>
          <p className="settings-help-text">
            {t(
              "settings.advanced.backupHelp",
              "Create a password-protected portable backup of the app memory, then import it later on this or another device.",
            )}
          </p>
          <div className="notice-actions">
            <button
              className="secondary-button"
              onClick={() => void onExportAppBackup()}
              disabled={isRunningBackupAction}
            >
              {isRunningBackupAction
                ? t("settings.advanced.backupWorking", "Working...")
                : t("settings.advanced.exportBackup", "Export backup")}
            </button>
            <button
              className="secondary-button"
              onClick={() => void onImportAppBackup()}
              disabled={isRunningBackupAction}
            >
              {isRunningBackupAction
                ? t("settings.advanced.backupWorking", "Working...")
                : t("settings.advanced.importBackup", "Import backup")}
            </button>
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsPane(pane: SettingsPane): JSX.Element {
    if (pane === "transcription") return renderSettingsTranscription();
    if (pane === "whisper_cpp") return renderSettingsWhisperCpp();
    if (pane === "local_models") return renderSettingsLocalModels();
    if (pane === "advanced") return renderSettingsAdvanced();
    if (pane === "general") return renderSettingsGeneral();
    if (pane === "ai_services") return renderSettingsAiServices();
    if (pane === "prompts") return renderSettingsPrompts();
    return (
      <div className="settings-placeholder">{t("settings.unavailable")}</div>
    );
  }

  function renderSettings(): JSX.Element {
    const settingsPaneClassName = `settings-pane settings-pane--${settingsPane}`;

    return (
      <div className="settings-layout">
        <aside className="settings-sidebar">
          <label className="settings-search">
            <Search size={14} />
            <input
              placeholder={t("settings.searchPlaceholder")}
              value={settingsQuery}
              onChange={(event) => setSettingsQuery(event.target.value)}
            />
          </label>
          <div className="settings-nav-list">
            {visibleSettingsPaneGroups.map(({ group, panes }) => (
              <section key={group} className="settings-nav-group">
                <p className="settings-nav-group-title">
                  {group === "General"
                    ? t("nav.general", "General")
                    : group === "Transcription"
                      ? t("nav.transcription", "Transcription")
                      : t("nav.aiServices", "AI Services")}
                </p>
                <div className="settings-nav-group-items">
                  {panes.map((pane) => {
                    const PaneIcon = pane.icon;
                    return (
                      <button
                        key={pane.key}
                        className={
                          settingsPane === pane.key
                            ? "settings-nav-item active"
                            : "settings-nav-item"
                        }
                        onClick={() => setSettingsPane(pane.key)}
                      >
                        <span className="settings-nav-item-main">
                          <PaneIcon size={14} />
                          <span>{pane.label}</span>
                        </span>
                        <small>{pane.description}</small>
                      </button>
                    );
                  })}
                </div>
              </section>
            ))}
            {visibleSettingsPanes.length === 0 ? (
              <div className="settings-nav-empty">{t("settings.noMatch")}</div>
            ) : null}
          </div>
        </aside>

        <section className="settings-content">
          <div className={settingsPaneClassName}>
            {renderSettingsPane(settingsPane)}
          </div>
        </section>
      </div>
    );
  }

  function renderContent(): JSX.Element {
    if (section === "home") return renderHome();
    if (section === "queue") return renderQueue();
    if (section === "history") return renderHistory();
    if (section === "deleted_history") return renderDeletedHistory();
    if (section === "realtime") return renderRealtime();
    return renderDetail();
  }

  function renderStartupGate(): JSX.Element {
    const bootstrapMessage = provisioning.progress
      ? `${formatProvisioningAssetLabel(provisioning.progress)} (${provisioning.progress.current}/${provisioning.progress.total})`
      : provisioning.statusMessage;
    const startupStatusDetail = initialSetupStepDetail ?? bootstrapMessage;
    const blockingError =
      startupRequirementsError ??
      initialSetupError ??
      (!settings ? error : null);
    const loadingSettings = !settings && !blockingError;
    const requiresPrivacyAcceptance =
      Boolean(settings) && !privacyPolicyAccepted;
    const loadingDiagnostics =
      settings &&
      privacyPolicyAccepted &&
      !startupRequirementsLoaded &&
      !blockingError;
    const setupPending =
      settings &&
      privacyPolicyAccepted &&
      startupRequirementsLoaded &&
      !initialSetupReady;
    const startupKicker =
      loadingSettings || loadingDiagnostics
        ? t("startup.loading.kicker", "Starting up")
        : t("setup.firstLaunch.kicker", "First launch setup");
    const startupTitle = loadingSettings
      ? t("startup.loading.title", "Loading Sbobino")
      : loadingDiagnostics
        ? t("startup.loading.runtimeTitle", "Sbobino is checking your Mac")
        : t("setup.firstLaunch.title", "Sbobino is preparing your Mac");
    const startupIntro = loadingSettings
      ? t(
          "setup.firstLaunch.loadingSettingsDesc",
          "Preparing your local workspace and saved settings.",
        )
      : requiresPrivacyAcceptance
        ? t(
            "setup.firstLaunch.privacyIntro",
            "Review and accept the privacy terms before using the app.",
          )
        : loadingDiagnostics
          ? t(
              "startup.loading.runtimeIntro",
              "Checking local runtime and required components before opening the app.",
            )
          : t(
              "setup.firstLaunch.runtimeIntro",
              "Sbobino is finishing the local setup required to run completely on your Mac.",
            );

    return (
      <main className="startup-shell">
        <section className="startup-card">
          <div className="startup-card-head">
            <span className="startup-kicker">{startupKicker}</span>
            <h1>{startupTitle}</h1>
            <p>{startupIntro}</p>
          </div>

          {requiresPrivacyAcceptance && settings ? (
            <>
              <div className="startup-policy-panel">
                <div className="startup-policy-version">
                  {t(
                    "setup.firstLaunch.policyVersion",
                    "Policy version {version}",
                    {
                      version: PRIVACY_POLICY_VERSION,
                    },
                  )}
                </div>
                <ul className="startup-policy-list">
                  {PRIVACY_POLICY_SUMMARY.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </div>
              <div className="startup-actions">
                <button
                  className="primary-button"
                  onClick={() => void acceptPrivacyPolicy()}
                  disabled={acceptingPrivacyPolicy}
                >
                  {acceptingPrivacyPolicy
                    ? t("setup.firstLaunch.accepting", "Saving...")
                    : t("setup.firstLaunch.accept", "Accept and continue")}
                </button>
              </div>
            </>
          ) : null}

          {loadingSettings ? (
            <div className="startup-status-card">
              <LoadingAnimation />
              <div>
                <strong>
                  {t("setup.firstLaunch.loadingSettings", "Loading Sbobino...")}
                </strong>
                <p>
                  {t(
                    "setup.firstLaunch.loadingSettingsDesc",
                    "Preparing your local workspace and saved settings.",
                  )}
                </p>
              </div>
            </div>
          ) : null}

          {loadingDiagnostics ? (
            <div className="startup-status-card">
              <LoadingAnimation />
              <div>
                <strong>
                  {t(
                    "setup.firstLaunch.inspecting",
                    "Inspecting local runtime...",
                  )}
                </strong>
                <p>
                  {t(
                    "setup.firstLaunch.inspectingDesc",
                    "Checking local tools and prerequisites.",
                  )}
                </p>
              </div>
            </div>
          ) : null}

          {setupPending ? (
            <div className="startup-status-card">
              <SetupMatrixIndicator
                progress={provisioning.progress?.percentage ?? 8}
                ariaLabel={t(
                  "setup.firstLaunch.progressIndicator",
                  "Local setup progress",
                )}
              />
              <div>
                <strong>
                  {initialSetupStepLabel ??
                    t(
                      "setup.firstLaunch.downloading",
                      "Downloading local models...",
                    )}
                </strong>
                <p>
                  {startupStatusDetail ||
                    t(
                      "setup.firstLaunch.downloadingDesc",
                      "This can take a few minutes the first time.",
                    )}
                </p>
              </div>
            </div>
          ) : null}

          {blockingError ? (
            <div className="startup-error-card">
              <strong>
                {t("setup.firstLaunch.setupError", "Setup could not finish")}
              </strong>
              <p>{blockingError}</p>
            </div>
          ) : null}

          {!requiresPrivacyAcceptance && (blockingError || setupPending) ? (
            <div className="startup-actions">
              <button
                className="primary-button"
                onClick={() => {
                  if (!startupRequirementsLoaded) {
                    void loadStartupRequirements().catch((error) => {
                      setStartupRequirementsLoaded(false);
                      setStartupRequirementsError(
                        formatUiError(
                          "error.startupRequirementsFailed",
                          "Could not prepare local runtime requirements",
                          error,
                        ),
                      );
                    });
                    return;
                  }
                  autoInitialSetupAttemptedRef.current = true;
                  void beginInitialSetup();
                }}
                disabled={initialSetupRunning}
              >
                {initialSetupRunning
                  ? t("setup.firstLaunch.retrying", "Working...")
                  : t("setup.firstLaunch.retry", "Retry setup")}
              </button>
              <button
                className="secondary-button"
                onClick={() =>
                  void onOpenStandaloneSettingsWindow("local_models")
                }
                disabled={initialSetupRunning}
              >
                {t("action.openLocalModels", "Open Local Models")}
              </button>
            </div>
          ) : null}
        </section>
      </main>
    );
  }

  if (standaloneSettingsWindow) {
    return (
      <main className="settings-window-shell">
        <section className="settings-window-frame">
          <header className="settings-window-header" data-tauri-drag-region />
          {renderGlobalUpdateBanner()}
          {renderPostUpdateValidationBanner()}
          {renderSettings()}
          {error ? (
            <div className="error-banner settings-window-error">
              <p>{error}</p>
              {shouldOfferLocalModelsCta(error) ? (
                <button
                  className="secondary-button error-action-button"
                  onClick={() =>
                    void onOpenStandaloneSettingsWindow("local_models")
                  }
                >
                  {t("action.openLocalModels", "Open Local Models")}
                </button>
              ) : null}
              <button
                className="error-close"
                onClick={() => setError(null)}
                title={t("action.dismiss", "Dismiss")}
              >
                <X size={14} />
              </button>
            </div>
          ) : null}
        </section>

        <ModelManagerSheet
          open={showModelManager}
          modelsDir={provisioning.modelsDir}
          models={modelCatalog}
          running={provisioning.running}
          progress={provisioning.progress}
          statusMessage={provisioning.statusMessage}
          onDownloadModel={onDownloadModel}
          onDownloadAll={onProvisionModels}
          onRefresh={refreshProvisioningModels}
          onCancel={onCancelProvisioning}
          onClose={() => setShowModelManager(false)}
        />
      </main>
    );
  }

  const shouldBlockMainUi = shouldBlockMainUiDuringStartup({
    hasSettings: Boolean(settings),
    privacyAccepted: privacyPolicyAccepted,
    warmStartEligible,
    startupRequirementsLoaded,
    initialSetupReady,
  });

  if (shouldBlockMainUi) {
    return renderStartupGate();
  }

  return (
    <main className="app-shell">
      <section
        ref={windowFrameRef}
        className={
          leftSidebarOpen ? "window-frame" : "window-frame left-collapsed"
        }
        style={windowFrameStyle}
      >
        <aside
          ref={leftSidebarRef}
          className={`left-sidebar ${leftSidebarOpen ? "" : "collapsed"}`}
        >
          <div
            className="sidebar-drag-cap"
            data-tauri-drag-region
            aria-hidden="true"
          />

          <div className="sidebar-section">
            <button
              className={
                section === "home" ? "sidebar-item active" : "sidebar-item"
              }
              onClick={() => setSection("home")}
            >
              <House size={16} />
              {t("sidebar.home", "Home")}
            </button>
            <button
              className={
                section === "queue" ? "sidebar-item active" : "sidebar-item"
              }
              onClick={() => setSection("queue")}
            >
              <ListChecks size={16} />
              {t("sidebar.queue", "Queue")}
            </button>
            <button
              className={
                section === "history" ? "sidebar-item active" : "sidebar-item"
              }
              onClick={() => setSection("history")}
            >
              <HistoryIcon size={16} />
              {t("sidebar.history", "History")}
            </button>
          </div>

          {openArtifacts.length > 0 ? (
            <div className="sidebar-section">
              <h4>{t("sidebar.open")}</h4>
              {openArtifacts.map((artifact) => (
                <div
                  key={artifact.id}
                  className={
                    activeArtifactId === artifact.id
                      ? "sidebar-item sidebar-open-row active"
                      : "sidebar-item sidebar-open-row"
                  }
                  title={artifact.title}
                  onClick={() => {
                    hydrateDetail(artifact);
                    setSection("detail");
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <div className="sidebar-open-item">
                    <FileAudio size={16} />
                    <span className="sidebar-item-label">{artifact.title}</span>
                  </div>
                  <button
                    className="sidebar-open-close"
                    onClick={(event) => {
                      event.stopPropagation();
                      setOpenArtifacts((prev) =>
                        prev.filter((a) => a.id !== artifact.id),
                      );
                      if (activeArtifactId === artifact.id) {
                        setActiveArtifactId(null);
                        setActiveDetailContext(null);
                        setSection("home");
                      }
                    }}
                    title={t(
                      "sidebar.closeTranscription",
                      "Close opened transcription",
                    )}
                    aria-label={t(
                      "sidebar.closeTranscription",
                      "Close opened transcription",
                    )}
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
            </div>
          ) : null}

          <div className="sidebar-section">
            <button
              className={
                section === "deleted_history"
                  ? "sidebar-item active"
                  : "sidebar-item"
              }
              onClick={() => setSection("deleted_history")}
            >
              <Trash2 size={16} />
              {t("sidebar.recentlyDeleted", "Recently Deleted")}
            </button>
          </div>

          <div className="sidebar-footer">
            <button
              className="sidebar-item"
              onClick={() => void onOpenStandaloneSettingsWindow("general")}
            >
              <Settings2 size={16} />
              {t("sidebar.settings", "Settings")}
            </button>
          </div>
          {leftSidebarOpen ? (
            <div
              className="sidebar-resize-handle sidebar-resize-handle-left"
              role="separator"
              aria-orientation="vertical"
              aria-label={t(
                "sidebar.resizeNavigation",
                "Resize navigation sidebar",
              )}
              onMouseDown={(event) => onStartSidebarResize("left", event)}
            />
          ) : null}
        </aside>

        <section ref={mainAreaRef} className="main-area">
          {section !== "detail" ? (
            <header className="main-topbar" data-tauri-drag-region>
              <div className="topbar-title" data-tauri-drag-region>
                <button
                  className={`icon-button sidebar-toggle-btn sidebar-toggle-left ${leftSidebarOpen ? "is-open" : ""}`}
                  onClick={() => setLeftSidebarOpen(!leftSidebarOpen)}
                  title={
                    leftSidebarOpen
                      ? t("topbar.hideSidebar", "Hide sidebar")
                      : t("topbar.showSidebar", "Show sidebar")
                  }
                >
                  <PanelLeftClose className="icon-close" size={16} />
                  <PanelLeftOpen className="icon-open" size={16} />
                </button>
                {section === "home" ? null : (
                  <h1 data-tauri-drag-region>
                    {section === "queue"
                      ? t("topbar.queue", "Queue")
                      : section === "realtime"
                        ? t("topbar.live", "Live")
                        : section === "deleted_history"
                          ? t("topbar.recentlyDeleted", "Recently Deleted")
                          : t("topbar.transcriptions", "Transcriptions")}
                  </h1>
                )}
                <div
                  className="topbar-drag-spacer"
                  data-tauri-drag-region
                  aria-hidden="true"
                />
              </div>

              {section === "home" ||
              section === "queue" ||
              section === "realtime" ? (
                <div className="topbar-controls">
                  <label className="select-chip">
                    <span className="chip-label">
                      <Mic size={12} />
                    </span>
                    <select
                      value={settings?.transcription.model ?? "base"}
                      onChange={(event) =>
                        void onChangeModel(event.target.value as SpeechModel)
                      }
                    >
                      {modelOptions.map((option) => (
                        <option key={option.value} value={option.value}>
                          {formatSpeechModelLabel(option.value, option.label)}
                        </option>
                      ))}
                    </select>
                  </label>

                  <label className="select-chip">
                    <span className="chip-label">
                      <Languages size={12} />
                    </span>
                    <select
                      value={settings?.transcription.language ?? "auto"}
                      onChange={(event) =>
                        void onChangeLanguage(
                          event.target.value as LanguageCode,
                        )
                      }
                    >
                      {languageOptions.map((option) => (
                        <option key={option.value} value={option.value}>
                          {t(`lang.${option.value}`, option.label)}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              ) : null}

              {section === "history" ? (
                <div className="topbar-controls history-topbar-controls">
                  <label className="history-filter-chip">
                    <ListFilter size={13} />
                    <select
                      value={historyKind}
                      onChange={(event) =>
                        setHistoryKind(
                          event.target.value as "all" | ArtifactKind,
                        )
                      }
                    >
                      <option value="all">{t("history.all")}</option>
                      <option value="file">{t("history.files")}</option>
                      <option value="realtime">{t("history.live")}</option>
                    </select>
                  </label>
                  <label className="search-chip history-top-search">
                    <Search size={14} />
                    <input
                      placeholder={t("history.searchPlaceholder")}
                      value={search}
                      onChange={(event) => setSearch(event.target.value)}
                    />
                  </label>
                </div>
              ) : null}

              {section === "deleted_history" ? (
                <div className="topbar-controls deleted-topbar-controls">
                  <button
                    className="secondary-button history-action-button"
                    onClick={() => void onEmptyTrash()}
                  >
                    <Trash2 size={14} />
                    {t("deleted.emptyTrash", "Empty Trash")}
                  </button>
                  <label className="search-chip history-top-search">
                    <Search size={14} />
                    <input
                      placeholder={t("history.searchPlaceholder")}
                      value={deletedSearch}
                      onChange={(event) => setDeletedSearch(event.target.value)}
                    />
                  </label>
                </div>
              ) : null}
            </header>
          ) : null}

          {renderGlobalUpdateBanner()}
          {renderPostUpdateValidationBanner()}
          <div className="main-content">{renderContent()}</div>

          {error ? (
            <div className="error-banner">
              <p>{error}</p>
              {shouldOfferLocalModelsCta(error) ? (
                <button
                  className="secondary-button error-action-button"
                  onClick={() =>
                    void onOpenStandaloneSettingsWindow("local_models")
                  }
                >
                  {t("action.openLocalModels", "Open Local Models")}
                </button>
              ) : null}
              <button
                className="error-close"
                onClick={() => setError(null)}
                title={t("action.dismiss", "Dismiss")}
              >
                <X size={14} />
              </button>
            </div>
          ) : null}
        </section>
      </section>

      {renameTarget ? (
        <div className="sheet-overlay" role="presentation">
          <section
            className="rename-sheet"
            role="dialog"
            aria-modal="true"
            aria-label={t("rename.title", "Rename transcription")}
          >
            <header className="rename-sheet-head">
              <h3>{t("rename.title")}</h3>
            </header>
            <input
              className="rename-sheet-input"
              value={renameDraft}
              onChange={(event) => setRenameDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void confirmRenameArtifact();
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  closeRenameDialog();
                }
              }}
              autoFocus
              placeholder={t("rename.placeholder", "Transcription title")}
            />
            <div className="rename-sheet-actions">
              <button
                className="secondary-button"
                onClick={closeRenameDialog}
                disabled={isRenamingArtifact}
              >
                {t("rename.cancel", "Cancel")}
              </button>
              <button
                className="primary-button"
                onClick={() => void confirmRenameArtifact()}
                disabled={isRenamingArtifact || renameDraft.trim().length === 0}
              >
                {isRenamingArtifact
                  ? t("rename.saving", "Saving...")
                  : t("rename.save", "Save")}
              </button>
            </div>
          </section>
        </div>
      ) : null}

      <ModelManagerSheet
        open={showModelManager}
        modelsDir={provisioning.modelsDir}
        models={modelCatalog}
        running={provisioning.running}
        progress={provisioning.progress}
        statusMessage={provisioning.statusMessage}
        onDownloadModel={onDownloadModel}
        onDownloadAll={onProvisionModels}
        onRefresh={refreshProvisioningModels}
        onCancel={onCancelProvisioning}
        onClose={() => setShowModelManager(false)}
      />

      <ExportSheet
        open={showExportSheet}
        transcriptText={exportPreviewText}
        segments={segmentsAlignedWithVisibleTranscript ? detailSegments : []}
        segmentsAlignedWithTranscript={segmentsAlignedWithVisibleTranscript}
        title={activeArtifact?.title ?? ""}
        summary={draftSummary}
        faqs={draftFaqs}
        onClose={() => setShowExportSheet(false)}
        onExport={onExport}
      />
    </main>
  );
}
