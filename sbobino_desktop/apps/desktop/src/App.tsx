import {
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { confirm as confirmDialog, open, save } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { check as checkAppUpdate, type Update as TauriUpdate } from "@tauri-apps/plugin-updater";
import {
  ArrowLeft,
  ArrowUp,
  AudioLines,
  Bot,
  Check,
  Cloud,
  Clock3,
  ChevronDown,
  Cpu,
  Database,
  FileAudio,
  FileText,
  Globe,
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
  Pencil,
  Radio,
  Save,
  Settings2,
  Sparkles,
  Plus,
  Search,
  Trash2,
  Upload,
  X,
  type LucideIcon,
} from "lucide-react";
import {
  cancelTranscription,
  chatArtifact,
  checkUpdates,
  deleteArtifacts,
  emptyDeletedArtifacts,
  ensureTranscriptionRuntime,
  exportArtifact,
  fetchTranscriptionStartPreflight,
  fetchRuntimeHealth,
  fetchSettingsSnapshot,
  getArtifact,
  hardDeleteArtifacts,
  listDeletedArtifacts,
  listGeminiModels,
  listRecentArtifacts,
  openSettingsWindow,
  pauseRealtime,
  provisioningCancel,
  provisioningDownloadModel,
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
  subscribeRealtimeSaved,
  subscribeRealtimeStatus,
  subscribeSettingsNavigate,
  subscribeSettingsUpdated,
  testPromptTemplate,
  updateArtifact,
  updateArtifactTimeline,
} from "./lib/tauri";
import { useAppStore } from "./state/useAppStore";
import type {
  AppearanceMode,
  AppSettings,
  ArtifactKind,
  JobProgress,
  LanguageCode,
  PromptTask,
  PromptTemplate,
  ProvisioningProgressEvent,
  ProvisioningModelCatalogEntry,
  RealtimeDelta,
  RemoteServiceConfig,
  RemoteServiceKind,
  RuntimeHealth,
  SpeechModel,
  TimelineV2,
  TranscriptionEngine,
  TranscriptArtifact,
  UpdateCheckResponse,
  WhisperOptions,
} from "./types";
import { AudioPlayer } from "./components/AudioPlayer";
import { ExportSheet, type ExportRequest } from "./components/ExportSheet";
import { ModelManagerSheet } from "./components/ModelManagerSheet";
import { LoadingAnimation } from "./components/LoadingAnimation";

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
type DetailMode = "transcript" | "segments" | "summary" | "chat";
type InspectorMode = "details" | "info";
type SettingsPane =
  | "general"
  | "transcription"
  | "whisper_cpp"
  | "local_models"
  | "ai_services"
  | "prompts"
  | "advanced";
type ChatMessage = { role: "user" | "assistant"; text: string };

type DetailSegment = {
  sourceIndex: number;
  time: string;
  line: string;
  speakerLabel: string | null;
  startSeconds: number | null;
  endSeconds: number | null;
};

type PromptTestState = {
  input: string;
  output: string;
  running: boolean;
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

function guessAppleSiliconFromUA(): boolean {
  const ua = (navigator.userAgent ?? "").toLowerCase();
  const platform = (navigator.platform ?? "").toLowerCase();
  const isMac = ua.includes("macintosh") || platform.includes("mac");

  const userAgentData = (navigator as Navigator & {
    userAgentData?: { architecture?: string; platform?: string };
  }).userAgentData;
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

const allTranscriptionEngineOptions: Array<{ value: TranscriptionEngine; label: string }> = [
  { value: "whisper_cpp", label: "Whisper C++" },
];

const chunkingOptions: Array<{ value: WhisperOptions["chunking_strategy"]; label: string }> = [
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
  tone: "google" | "openai" | "anthropic" | "azure" | "lmstudio" | "ollama" | "openrouter" | "xai" | "huggingface" | "custom";
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

const defaultPromptTestInput = "This is an example of some transcribed text.";
function getDefaultWhisperOptions(useAppleSiliconDefaults = guessAppleSiliconFromUA()): WhisperOptions {
  // On Intel Macs: use more CPU threads, greedy beam search, and CPU-only compute units
  // (no Neural Engine or CoreML acceleration available)
  const threads = useAppleSiliconDefaults
    ? 4
    : Math.max(4, Math.min(8, Math.floor((navigator.hardwareConcurrency ?? 8) / 2)));

  return {
    translate_to_english: false,
    no_context: false,
    split_on_word: false,
    tinydiarize: false,
    diarize: false,
    temperature: 0,
    temperature_increment_on_fallback: 0.2,
    temperature_fallback_count: 5,
    entropy_threshold: 2.5,
    logprob_threshold: -1,
    first_token_logprob_threshold: -1.5,
    no_speech_threshold: 0.6,
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
    audio_encoder_compute_units: useAppleSiliconDefaults ? "cpu_and_neural_engine" : "cpu_only",
    text_decoder_compute_units: useAppleSiliconDefaults ? "cpu_and_neural_engine" : "cpu_only",
  };
}

type SettingsPaneDefinition = {
  key: SettingsPane;
  label: string;
  description: string;
  group: "General" | "Transcription" | "AI";
  icon: LucideIcon;
};

const _settingsPaneDefinitions: SettingsPaneDefinition[] = [
  {
    key: "general",
    label: "General",
    description: "App updates and general behavior",
    group: "General",
    icon: House,
  },
  {
    key: "transcription",
    label: "Transcription",
    description: "Default engine, model and language",
    group: "Transcription",
    icon: Mic,
  },
  {
    key: "whisper_cpp",
    label: "Whisper C++",
    description: "whisper.cpp decoding controls",
    group: "Transcription",
    icon: Settings2,
  },
  {
    key: "local_models",
    label: "Local Models",
    description: "Model downloads and runtime health",
    group: "Transcription",
    icon: Upload,
  },
  {
    key: "advanced",
    label: "Advanced",
    description: "CLI and FFmpeg paths",
    group: "Transcription",
    icon: Settings2,
  },
  {
    key: "ai_services",
    label: "AI Services",
    description: "Provider configuration",
    group: "AI",
    icon: Sparkles,
  },
  {
    key: "prompts",
    label: "Prompts",
    description: "Template management",
    group: "AI",
    icon: MessageSquareText,
  },
];

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

async function withTimeout<T>(promise: Promise<T>, ms: number, timeoutMessage: string): Promise<T> {
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
  const startDate = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const dayMs = 24 * 60 * 60 * 1000;
  const diffDays = Math.round((startToday.getTime() - startDate.getTime()) / dayMs);
  if (diffDays === 0) return "oggi";
  if (diffDays === 1) return "ieri";
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

function parseTimelineV2Segments(timelineV2Json: string | null | undefined): DetailSegment[] {
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

      const startSeconds = parseFiniteSeconds((segment as { start_seconds?: unknown }).start_seconds);
      const endSeconds = parseFiniteSeconds((segment as { end_seconds?: unknown }).end_seconds);
      let firstWordStart: number | null = null;
      let lastWordEnd: number | null = null;
      const words = (segment as { words?: unknown }).words;
      if (Array.isArray(words)) {
        for (const word of words) {
          if (!word || typeof word !== "object") {
            continue;
          }
          const wordStart = parseFiniteSeconds((word as { start_seconds?: unknown }).start_seconds);
          if (firstWordStart === null && wordStart !== null) {
            firstWordStart = wordStart;
          }
          const wordEnd = parseFiniteSeconds((word as { end_seconds?: unknown }).end_seconds);
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
        parseNonEmptyText((segment as { speaker_label?: unknown }).speaker_label)
        ?? parseNonEmptyText((segment as { speaker_id?: unknown }).speaker_id);

      return [{
        sourceIndex,
        time: formatTimelineTimestamp(anchorSeconds),
        line: text,
        speakerLabel,
        startSeconds: resolvedStartSeconds,
        endSeconds: resolvedEndSeconds,
      }];
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
        .map((segment) => (segment && typeof segment === "object" ? { ...(segment as Record<string, unknown>) } : null))
        .filter((segment): segment is Record<string, unknown> => Boolean(segment)),
    };
  } catch {
    return null;
  }
}

function normalizeSpeakerId(label: string): string {
  const candidate = label
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return candidate.length > 0 ? candidate : "speaker";
}

function readSegmentSpeakerLabel(segment: Record<string, unknown>): string | null {
  const label = parseNonEmptyText(segment.speaker_label);
  if (label) {
    return label;
  }
  return parseNonEmptyText(segment.speaker_id);
}

function readSegmentStartSeconds(segment: Record<string, unknown>): number | null {
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
    const wordStart = parseFiniteSeconds((word as { start_seconds?: unknown }).start_seconds);
    if (wordStart !== null) {
      return wordStart;
    }
  }
  return null;
}

function readSegmentEndSeconds(segment: Record<string, unknown>): number | null {
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
    const wordEnd = parseFiniteSeconds((word as { end_seconds?: unknown }).end_seconds);
    if (wordEnd !== null) {
      return wordEnd;
    }
  }
  return null;
}

function pushOrReplaceQueueItem(items: JobProgress[], incoming: JobProgress): JobProgress[] {
  const existing = items.find((entry) => entry.job_id === incoming.job_id);
  if (!existing) {
    return [incoming, ...items];
  }
  return items.map((entry) => (entry.job_id === incoming.job_id ? incoming : entry));
}

function formatShortDuration(seconds: number): string {
  const mm = String(Math.floor(seconds / 60));
  const ss = String(seconds % 60).padStart(2, "0");
  return `${mm}:${ss}`;
}

function clampPercentage(value: number): number {
  if (!Number.isFinite(value)) return 0;
  if (value < 0) return 0;
  if (value > 100) return 100;
  return value;
}

function percentageFromJobProgress(progress: JobProgress | null | undefined): number {
  if (!progress) return 0;
  const currentSeconds = progress.current_seconds ?? null;
  const totalSeconds = progress.total_seconds ?? null;
  if (currentSeconds !== null && totalSeconds !== null && totalSeconds > 0) {
    return clampPercentage((currentSeconds / totalSeconds) * 100);
  }
  return clampPercentage(progress.percentage);
}

function formatJobStageLabel(stage: string): string {
  return stage
    .replace(/_/g, " ")
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

function formatProviderLabel(kind: RemoteServiceKind): string {
  const entry = serviceCatalog.find((item) => item.kind === kind);
  if (entry) return entry.label;
  return kind.replace(/_/g, " ").replace(/\b\w/g, (match) => match.toUpperCase());
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
    const direct = typeof record.message === "string" ? record.message.trim() : "";
    if (direct.length > 0) return direct;

    return (
      pickMessage(record.error, depth + 1)
      ?? pickMessage(record.cause, depth + 1)
      ?? pickMessage(record.details, depth + 1)
      ?? pickMessage(record.data, depth + 1)
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
    return pickCode(record.error, depth + 1)
      ?? pickCode(record.cause, depth + 1)
      ?? pickCode(record.details, depth + 1)
      ?? pickCode(record.data, depth + 1);
  };

  return pickCode(error);
}

function ProgressRing({ percentage, size = 18 }: { percentage: number; size?: number }): JSX.Element {
  const clamped = clampPercentage(percentage);
  const ringStyle = {
    width: `${size}px`,
    height: `${size}px`,
    backgroundImage: `conic-gradient(from -90deg, var(--accent) ${clamped}%, var(--line-soft) ${clamped}% 100%)`,
  } satisfies CSSProperties;

  return (
    <span className="progress-ring" style={ringStyle} aria-hidden>
      <span className="progress-ring-core" />
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

function createRemoteServiceId(kind: RemoteServiceKind): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `${kind}_${crypto.randomUUID()}`;
  }
  return `${kind}_${Date.now()}_${Math.random().toString(16).slice(2, 10)}`;
}

function normalizeSettings(settings: AppSettings): AppSettings {
  const normalizedWhisperOptions = sanitizeWhisperOptions({
    ...getDefaultWhisperOptions(),
    ...settings.transcription.whisper_options,
  });

  const normalized: AppSettings = {
    ...settings,
    general: {
      ...settings.general,
      appearance_mode: settings.general?.appearance_mode ?? "system",
    },
    transcription: {
      ...settings.transcription,
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
        },
      },
      remote_services: (settings.ai.remote_services ?? []).map((service) => ({
        ...service,
        label: service.label || formatProviderLabel(service.kind),
      })),
    },
    prompts: {
      ...settings.prompts,
      bindings: {
        ...settings.prompts.bindings,
      },
      templates: settings.prompts.templates.map((template) => ({ ...template })),
    },
  };

  normalized.model = normalized.transcription.model;
  normalized.language = normalized.transcription.language;
  normalized.transcription_engine = normalized.transcription.engine;
  normalized.ai_post_processing = normalized.transcription.enable_ai_post_processing;
  normalized.whisper_cli_path = normalized.transcription.whisper_cli_path;
  normalized.whisperkit_cli_path = normalized.transcription.whisperkit_cli_path;
  normalized.ffmpeg_path = normalized.transcription.ffmpeg_path;
  normalized.models_dir = normalized.transcription.models_dir;

  normalized.auto_update_enabled = normalized.general.auto_update_enabled;
  normalized.auto_update_repo = normalized.general.auto_update_repo;

  normalized.gemini_model = normalized.ai.providers.gemini.model;
  normalized.gemini_api_key = normalized.ai.providers.gemini.api_key;

  return normalized;
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
    temperature_increment_on_fallback: clamp(options.temperature_increment_on_fallback, 0, 2),
    temperature_fallback_count: Math.round(clamp(options.temperature_fallback_count, 0, 20)),
    entropy_threshold: clamp(options.entropy_threshold, 0, 10),
    logprob_threshold: clamp(options.logprob_threshold, -10, 0),
    first_token_logprob_threshold: clamp(options.first_token_logprob_threshold, -10, 0),
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
    concurrent_worker_count: Math.round(clamp(options.concurrent_worker_count, 1, 16)),
    chunking_strategy: options.chunking_strategy === "none" ? "none" : "vad",
    audio_encoder_compute_units: options.audio_encoder_compute_units,
    text_decoder_compute_units: options.text_decoder_compute_units,
  };
}

type DetailCenterModeControlProps = {
  detailMode: DetailMode;
  summaryDisabled: boolean;
  chatDisabled: boolean;
  onSelect: (mode: "transcript" | "summary" | "chat") => void;
};

function DetailCenterModeControl({
  detailMode,
  summaryDisabled,
  chatDisabled,
  onSelect,
}: DetailCenterModeControlProps): JSX.Element {
  return (
    <div className="segmented-control detail-mode-slider">
      <button
        className={detailMode === "transcript" || detailMode === "segments" ? "seg active" : "seg"}
        onClick={() => onSelect("transcript")}
        title="Transcription"
      >
        <FileText size={15} />
      </button>
      <button
        className={detailMode === "summary" ? "seg active" : "seg"}
        onClick={() => onSelect("summary")}
        title="AI Summary"
        disabled={summaryDisabled}
      >
        <Sparkles size={15} />
      </button>
      <button
        className={detailMode === "chat" ? "seg active" : "seg"}
        onClick={() => onSelect("chat")}
        title="AI Chat"
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
  detailMode: DetailMode;
  title: string;
  hasArtifact: boolean;
  hasActiveJob: boolean;
  transcriptionProgress: number;
  onToggleSidebar: () => void;
  onBack: () => void;
  onSelectMode: (mode: "transcript" | "summary" | "chat") => void;
  onOpenExport: () => void;
  onShowDetailsPanel: () => void;
  onHideDetailsPanel: () => void;
  onCancel: () => void;
  isImprovingText?: boolean;
  onImproveText?: () => void;
};

function DetailToolbar({
  leftSidebarOpen,
  rightSidebarOpen,
  detailMode,
  title,
  hasArtifact,
  hasActiveJob,
  transcriptionProgress,
  onToggleSidebar,
  onBack,
  onSelectMode,
  onOpenExport,
  onShowDetailsPanel,
  onHideDetailsPanel,
  onCancel,
  isImprovingText,
  onImproveText,
}: DetailToolbarProps): JSX.Element {
  return (
    <header
      className={`detail-toolbar ${!leftSidebarOpen ? "sidebar-closed" : ""}`}
      data-tauri-drag-region
    >
      <div className="detail-toolbar-left" data-tauri-drag-region>
        <button
          className={`icon-button sidebar-toggle-btn sidebar-toggle-left ${leftSidebarOpen ? "is-open" : ""}`}
          onClick={onToggleSidebar}
          title={leftSidebarOpen ? "Hide sidebar" : "Show sidebar"}
        >
          <PanelLeftClose className="icon-close" size={16} />
          <PanelLeftOpen className="icon-open" size={16} />
        </button>
        <button className="icon-button" onClick={onBack} title="Back to history">
          <ArrowLeft size={16} />
        </button>
        <strong className="detail-title" data-tauri-drag-region>{title}</strong>
      </div>

      <div style={{ display: "flex", alignItems: "center" }}>
        {detailMode === "transcript" && onImproveText && (
          <button
            className="optimize-hover-button"
            onClick={() => void onImproveText()}
            disabled={isImprovingText || !hasArtifact}
            title="Improve Text"
          >
            <div className="button-content">
              <Sparkles size={14} /> Optimize
            </div>
          </button>
        )}
        <DetailCenterModeControl
          detailMode={detailMode}
          summaryDisabled={!hasArtifact}
          chatDisabled={!hasArtifact}
          onSelect={onSelectMode}
        />
      </div>

      <div className="detail-toolbar-right">
        {hasArtifact ? (
          <button className="secondary-button export-toolbar-button" onClick={onOpenExport}>
            Export
            <ChevronDown size={14} />
          </button>
        ) : null}
        <button
          className={`icon-button sidebar-toggle-btn sidebar-toggle-right ${rightSidebarOpen ? "is-open" : ""}`}
          onClick={() => (rightSidebarOpen ? onHideDetailsPanel() : onShowDetailsPanel())}
          title={rightSidebarOpen ? "Hide details panel" : "Show details panel"}
        >
          <PanelRightClose className="icon-close" size={16} />
          <PanelRightOpen className="icon-open" size={16} />
        </button>
        {!hasArtifact && hasActiveJob ? (
          <button className="transcribing-cancel-pill" onClick={onCancel} title="Cancel transcription">
            <span className="pill-content default-content">
              <ProgressRing percentage={transcriptionProgress} size={16} />
              <span>Transcribing</span>
            </span>
            <span className="pill-content hover-content">
              <X size={16} />
              <span>Cancel transcription</span>
            </span>
          </button>
        ) : null}
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
  return (
    <header className="inspector-header">
      <div className="segmented-control inspector-view-toggle">
        <button
          className={inspectorMode === "details" ? "seg active" : "seg"}
          onClick={() => onInspectorModeChange("details")}
          title="Details controls"
          aria-label="Show details controls"
        >
          <List size={15} />
        </button>
        <button
          className={inspectorMode === "info" ? "seg active" : "seg"}
          onClick={() => onInspectorModeChange("info")}
          title="Transcript information"
          aria-label="Show transcript information"
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
  return (
    <div className="inspector-mode-grid">
      <button
        className={detailMode === "transcript" ? "mode-tile active" : "mode-tile"}
        onClick={() => onSelectMode("transcript")}
        title="Transcript"
      >
        <span className="mode-tile-icon">
          <FileText size={18} />
        </span>
        <span>Transcript</span>
      </button>
      <button
        className={detailMode === "segments" ? "mode-tile active" : "mode-tile"}
        onClick={() => onSelectMode("segments")}
        title="Segments"
      >
        <span className="mode-tile-icon">
          <List size={18} />
        </span>
        <span>Segments</span>
      </button>
    </div>
  );
}

type AppProps = {
  standaloneSettingsWindow?: boolean;
};

export function App({ standaloneSettingsWindow = false }: AppProps) {
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

  const preferredAppearanceMode = settings?.general?.appearance_mode ?? "system";

  // Apply user-selected appearance mode (system/light/dark) consistently for all windows.
  useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      const nextTheme =
        preferredAppearanceMode === "system"
          ? (mediaQuery.matches ? "dark" : "light")
          : preferredAppearanceMode;
      document.documentElement.dataset.theme = nextTheme;

      // Keep native macOS titlebar appearance aligned with the app theme.
      // This avoids light titlebar bleed when the app is forced to dark mode (and vice-versa).
      void (async () => {
        const hasTauriRuntime = Boolean(
          (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__,
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
  const [settingsPane, setSettingsPane] = useState<SettingsPane>("transcription");
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
  const [search, setSearch] = useState("");
  const [deletedSearch, setDeletedSearch] = useState("");
  const [historyKind, setHistoryKind] = useState<"all" | ArtifactKind>("all");
  const [deletedArtifacts, setDeletedArtifacts] = useState<TranscriptArtifact[]>([]);
  const [selectedArtifactIds, setSelectedArtifactIds] = useState<string[]>([]);

  const [isStarting, setIsStarting] = useState(false);
  const [isSavingArtifact, setIsSavingArtifact] = useState(false);

  const [openArtifacts, setOpenArtifacts] = useState<TranscriptArtifact[]>([]);
  const [activeArtifactId, setActiveArtifactId] = useState<string | null>(null);

  const activeArtifact = useMemo(
    () => openArtifacts.find((a) => a.id === activeArtifactId) || null,
    [openArtifacts, activeArtifactId]
  );
  
  const setActiveArtifact = (artifact: TranscriptArtifact | null) => {
    if (!artifact) {
      // Clear all
      setOpenArtifacts([]);
      setActiveArtifactId(null);
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
  const [draftSummary, setDraftSummary] = useState("");
  const [draftFaqs, setDraftFaqs] = useState("");
  const [showExportSheet, setShowExportSheet] = useState(false);
  const [renameTarget, setRenameTarget] = useState<TranscriptArtifact | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [isRenamingArtifact, setIsRenamingArtifact] = useState(false);

  const [chatInput, setChatInput] = useState("");
  const [chatHistory, setChatHistory] = useState<Array<{ role: "user" | "assistant"; text: string }>>([]);
  const [isAskingChat, setIsAskingChat] = useState(false);
  const [isImprovingText, setIsImprovingText] = useState(false);
  const [activeJobPreviewText, setActiveJobPreviewText] = useState("");
  const [activeJobTitle, setActiveJobTitle] = useState("");
  const [selectedSegmentSourceIndex, setSelectedSegmentSourceIndex] = useState<number | null>(null);
  const [speakerDraft, setSpeakerDraft] = useState("");
  const [isAssigningSpeaker, setIsAssigningSpeaker] = useState(false);
  const [propagateSpeakerAssignment, setPropagateSpeakerAssignment] = useState(false);
  const [segmentContextMenu, setSegmentContextMenu] = useState<{
    x: number;
    y: number;
    sourceIndex: number;
  } | null>(null);

  const [queueItems, setQueueItems] = useState<JobProgress[]>([]);
  const [modelCatalog, setModelCatalog] = useState<ProvisioningModelCatalogEntry[]>([]);
  const [runtimeHealth, setRuntimeHealth] = useState<RuntimeHealth | null>(null);
  const platformIsAppleSilicon = runtimeHealth?.is_apple_silicon ?? guessAppleSiliconFromUA();
  const transcriptionEngineOptions = useMemo(() => allTranscriptionEngineOptions, []);
  const settingsPaneDefinitions = useMemo(() => _settingsPaneDefinitions, []);

  const [realtimeState, setRealtimeState] = useState<"idle" | "running" | "paused">("idle");
  const [realtimeMessage, setRealtimeMessage] = useState("Realtime idle");
  const [realtimeFinalLines, setRealtimeFinalLines] = useState<string[]>([]);
  const [realtimePreview, setRealtimePreview] = useState("");
  const [isStoppingRealtime, setIsStoppingRealtime] = useState(false);

  const [provisioning, setProvisioning] = useState<{
    ready: boolean;
    modelsDir: string;
    missing: string[];
    running: boolean;
    progress: ProvisioningProgressEvent | null;
    statusMessage: string;
  }>({
    ready: true,
    modelsDir: "",
    missing: [],
    running: false,
    progress: null,
    statusMessage: "",
  });

  const [updateInfo, setUpdateInfo] = useState<UpdateCheckResponse | null>(null);
  const [nativeUpdate, setNativeUpdate] = useState<TauriUpdate | null>(null);
  const [updateSource, setUpdateSource] = useState<"native" | "github" | null>(null);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [updateDownloadPercent, setUpdateDownloadPercent] = useState<number | null>(null);
  const [updateStatusMessage, setUpdateStatusMessage] = useState<string | null>(null);
  const [aiServicesAcknowledged, setAiServicesAcknowledged] = useState(false);
  const [aiServiceConfigOpen, setAiServiceConfigOpen] = useState<string | null>(null);
  const [geminiModelChoices, setGeminiModelChoices] = useState<string[]>(fallbackGeminiModelOptions);
  const [loadingGeminiModels, setLoadingGeminiModels] = useState(false);
  const [geminiModelFetchNonce, setGeminiModelFetchNonce] = useState(0);

  const [activePromptId, setActivePromptId] = useState("");
  const [promptDraft, setPromptDraft] = useState<PromptTemplate | null>(null);
  const [promptBindingTask, setPromptBindingTask] = useState<PromptTask>("optimize");
  const [promptTest, setPromptTest] = useState<PromptTestState>({
    input: defaultPromptTestInput,
    output: "",
    running: false,
  });
  const [fontSize, setFontSize] = useState(18);
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const [groupSegmentsWithoutSpeakers, setGroupSegmentsWithoutSpeakers] = useState(true);
  const [summaryIncludeTimestamps, setSummaryIncludeTimestamps] = useState(true);
  const [summaryIncludeSpeakers, setSummaryIncludeSpeakers] = useState(false);
  const [summaryAutostart, setSummaryAutostart] = useState(false);
  const [summarySections, setSummarySections] = useState(true);
  const [summaryBulletPoints, setSummaryBulletPoints] = useState(false);
  const [summaryActionItems, setSummaryActionItems] = useState(true);
  const [summaryKeyPointsOnly, setSummaryKeyPointsOnly] = useState(true);
  const [summaryLanguage, setSummaryLanguage] = useState<LanguageCode>("en");
  const [summaryCustomPrompt, setSummaryCustomPrompt] = useState("");
  const [chatIncludeTimestamps, setChatIncludeTimestamps] = useState(true);
  const [chatIncludeSpeakers, setChatIncludeSpeakers] = useState(false);
  const [isGeneratingSummary, setIsGeneratingSummary] = useState(false);
  const [audioDurationSeconds, setAudioDurationSeconds] = useState(0);

  const activeJobIdRef = useRef<string | null>(activeJobId);
  const activeJobDeltaSequenceRef = useRef<number>(-1);
  const activeJobPreviewTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const detailMainRef = useRef<HTMLElement | null>(null);
  const peopleSpeakerInputRef = useRef<HTMLInputElement | null>(null);
  const failedJobMessagesRef = useRef<Map<string, string>>(new Map());
  const startupWatchdogRef = useRef<number | null>(null);
  const settingsSaveSequenceRef = useRef(0);
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
    if (!activeJobId) {
      return;
    }
    const textarea = activeJobPreviewTextareaRef.current;
    if (!textarea) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      textarea.scrollTop = textarea.scrollHeight;
    });
    return () => window.cancelAnimationFrame(frame);
  }, [activeJobId, activeJobPreviewText]);

  useEffect(() => {
    setSelectedSegmentSourceIndex(null);
    setSpeakerDraft("");
  }, [activeArtifactId]);

  useEffect(() => {
    window.localStorage.setItem("sbobino.layout.leftSidebarOpen", String(leftSidebarOpen));
  }, [leftSidebarOpen]);

  useEffect(() => {
    window.localStorage.setItem("sbobino.layout.rightSidebarOpen", String(rightSidebarOpen));
  }, [rightSidebarOpen]);

  useEffect(() => {
    if (standaloneSettingsWindow && settingsPane === "local_models") {
      void refreshRuntimeHealth();
    }
  }, [settingsPane, standaloneSettingsWindow]);

  useEffect(() => {
    if (section !== "deleted_history") {
      return;
    }

    void (async () => {
      try {
        const deletedArtifactsSnapshot = await listDeletedArtifacts({ limit: 200 });
        setDeletedArtifacts(deletedArtifactsSnapshot);
      } catch (deletedError) {
        setError(`Could not load Recently Deleted: ${formatAppError(deletedError)}`);
      }
    })();
  }, [section, setError]);

  useEffect(() => {
    let disposed = false;

    void (async () => {
      try {
        const [
          initialSettings,
          activeArtifactsSnapshot,
          deletedArtifactsSnapshot,
          provision,
          models,
        ] = await Promise.all([
          fetchSettingsSnapshot(),
          listRecentArtifacts(),
          listDeletedArtifacts({ limit: 200 }),
          provisioningStatus(),
          provisioningModels(),
        ]);

        if (disposed) return;

        setSettings(normalizeSettings(initialSettings));
        setArtifacts(activeArtifactsSnapshot);
        setDeletedArtifacts(deletedArtifactsSnapshot);
        setProvisioningState(provision);
        setModelCatalog(models);
        try {
          const initialRuntimeHealth = await fetchRuntimeHealth();
          if (!disposed) {
            setRuntimeHealth(initialRuntimeHealth);
          }

          if (
            !initialRuntimeHealth.whisper_cli_available ||
            !initialRuntimeHealth.whisper_stream_available
          ) {
            try {
              const ensured = await ensureTranscriptionRuntime();
              if (!disposed && ensured.did_setup) {
                const refreshed = await fetchRuntimeHealth();
                if (!disposed) {
                  setRuntimeHealth(refreshed);
                }
              }
            } catch {
              // non-blocking: runtime setup is retried on transcription start
            }
          }
        } catch {
          // keep app booting even if health probe fails
        }

        if (initialSettings.general.auto_update_enabled) {
          await refreshUpdates(true, () => disposed);
        }
      } catch (bootstrapError) {
        setError(`Bootstrap failed: ${formatAppError(bootstrapError)}`);
      }
    })();

    return () => {
      disposed = true;
    };
  }, [setArtifacts, setError, setSettings]);

  useEffect(() => {
    if (!standaloneSettingsWindow) {
      return;
    }

    const pane = new URLSearchParams(window.location.search).get("pane");
    if (!pane) {
      return;
    }

    const matchedPane = settingsPaneDefinitions.find((entry) => entry.key === pane);
    if (matchedPane) {
      setSettingsPane(matchedPane.key);
    }
  }, [settingsPaneDefinitions, standaloneSettingsWindow]);

  useEffect(() => {
    if (!settings) {
      return;
    }
    if (settings.ai.active_provider !== "none") {
      return;
    }
    const hasActiveRemote =
      settings.ai.active_remote_service_id !== null
      && settings.ai.remote_services.some(
        (service) =>
          service.id === settings.ai.active_remote_service_id
          && service.enabled,
      );
    if (hasActiveRemote) {
      return;
    }
    const hasGoogleService = (settings.ai.remote_services ?? []).some(
      (service) => service.kind === "google",
    );
    if (platformIsAppleSilicon && settings.ai.providers.foundation_apple.enabled) {
      void patchAiSettings((current) => ({
        ...current,
        active_provider: "foundation_apple",
      }));
      return;
    }
    if (hasGoogleService && settings.ai.providers.gemini.api_key?.trim()) {
      const googleServiceId =
        settings.ai.remote_services.find((service) => service.kind === "google")?.id ?? null;
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
    settings?.ai.providers.gemini.api_key,
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
      settings?.ai.remote_services?.find((service) => service.kind === "google")?.id ?? null;
    if (!googleServiceId || aiServiceConfigOpen !== googleServiceId) {
      return;
    }
    const apiKey = settings?.ai.providers.gemini.api_key?.trim();
    if (!apiKey) {
      setGeminiModelChoices(fallbackGeminiModelOptions);
      return;
    }

    let cancelled = false;
    void (async () => {
      setLoadingGeminiModels(true);
      try {
        const models = await listGeminiModels(apiKey);

        if (!cancelled && models.length > 0) {
          const current = settings?.ai.providers.gemini.model;
          const merged = Array.from(
            new Set([...(current ? [current] : []), ...models, ...fallbackGeminiModelOptions]),
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
    settings?.ai.providers.gemini.api_key,
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
        const matchedPane = settingsPaneDefinitions.find((entry) => entry.key === pane);
        if (matchedPane) {
          setSettingsPane(matchedPane.key);
        }
      });
    })();

    return () => {
      unlisten?.();
    };
  }, [settingsPaneDefinitions, standaloneSettingsWindow]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void (async () => {
      unlisten = await subscribeSettingsUpdated((incoming) => {
        setSettings(normalizeSettings(incoming));
      });
    })();

    return () => {
      unlisten?.();
    };
  }, [setSettings]);

  // Tauri native drag-and-drop: auto-load + transcribe dropped audio files
  useEffect(() => {
    const audioExtensions = ["wav", "mp3", "m4a", "flac", "ogg", "aac", "opus", "webm", "mp4", "mov", "mkv", "wma", "aiff"];
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
              setSelectedFile(audioFile);
              setSection("home");
              setError(null);
              // Defer briefly to let React flush states, then start transcription
              setTimeout(() => {
                void onStartTranscription(audioFile);
              }, 100);
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
  }, [setSelectedFile, setError]);

  useEffect(() => {
    let unmounted = false;
    let unsubProgress: (() => void) | undefined;
    let unsubCompleted: (() => void) | undefined;
    let unsubFailed: (() => void) | undefined;
    let unsubTranscriptionDelta: (() => void) | undefined;
    let unsubRealtimeDelta: (() => void) | undefined;
    let unsubRealtimeStatus: (() => void) | undefined;
    let unsubRealtimeSaved: (() => void) | undefined;
    let unsubProvisioningProgress: (() => void) | undefined;
    let unsubProvisioningStatus: (() => void) | undefined;

    void (async () => {
      const uProgress = await subscribeJobProgress((event) => {
        setQueueItems((previous) => pushOrReplaceQueueItem(previous, event));
        if (event.job_id === activeJobIdRef.current) {
          clearStartupWatchdog();
          setProgress(event);
          if (event.stage === "cancelled" || event.stage === "failed") {
            clearStartupWatchdog();
            clearActiveJob();
            activeJobIdRef.current = null;
            setActiveJobPreviewText("");
            setActiveJobTitle("");
            activeJobDeltaSequenceRef.current = -1;
            if (event.stage === "failed") {
              setError(event.message || "Transcription failed.");
              setSection("home");
            }
          }
        }
      });
      if (unmounted) { uProgress(); } else { unsubProgress = uProgress; }

      const uCompleted = await subscribeJobCompleted((artifact) => {
        failedJobMessagesRef.current.delete(artifact.job_id);
        prependArtifact(artifact);
        setQueueItems((previous) => previous.filter((entry) => entry.job_id !== artifact.job_id));
        setActiveJobPreviewText("");
        setActiveJobTitle("");
        activeJobDeltaSequenceRef.current = -1;

        if (artifact.job_id === activeJobIdRef.current) {
          clearStartupWatchdog();
          clearActiveJob();
          activeJobIdRef.current = null;
          hydrateDetail(artifact);
          setSection("detail");
          setError(null);
        }
      });
      if (unmounted) { uCompleted(); } else { unsubCompleted = uCompleted; }

      const uFailed = await subscribeJobFailed((payload) => {
        failedJobMessagesRef.current.set(payload.job_id, payload.message);
        setQueueItems((previous) =>
          previous.map((entry) =>
            entry.job_id === payload.job_id
              ? {
                  ...entry,
                  stage: "failed",
                  message: payload.message,
                  percentage: 100,
                }
              : entry,
          ),
        );

        if (payload.job_id === activeJobIdRef.current) {
          clearStartupWatchdog();
          failedJobMessagesRef.current.delete(payload.job_id);
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobPreviewText("");
          setActiveJobTitle("");
          activeJobDeltaSequenceRef.current = -1;
          setError(payload.message);
          setSection("home");
        }
      });
      if (unmounted) { uFailed(); } else { unsubFailed = uFailed; }

      const uTranscriptionDelta = await subscribeTranscriptionDelta((delta) => {
        if (delta.job_id !== activeJobIdRef.current) {
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

          const nextLine = next.trim();
          if (!previous) return nextLine;
          if (previous.endsWith(nextLine)) return previous;
          return `${previous}\n${nextLine}`;
        });
      });
      if (unmounted) { uTranscriptionDelta(); } else { unsubTranscriptionDelta = uTranscriptionDelta; }

      const uRealtimeDelta = await subscribeRealtimeDelta((delta: RealtimeDelta) => {
        if (delta.kind === "append_final") {
          setRealtimeFinalLines((previous) => [...previous, delta.text]);
          setRealtimePreview("");
        }

        if (delta.kind === "update_preview") {
          setRealtimePreview(delta.text);
        }
      });
      if (unmounted) { uRealtimeDelta(); } else { unsubRealtimeDelta = uRealtimeDelta; }

      const uRealtimeStatus = await subscribeRealtimeStatus((event) => {
        setRealtimeMessage(event.message);
        if (event.state === "running") {
          setRealtimeState("running");
        } else if (event.state === "paused") {
          setRealtimeState("paused");
        } else {
          setRealtimeState("idle");
        }
      });
      if (unmounted) { uRealtimeStatus(); } else { unsubRealtimeStatus = uRealtimeStatus; }

      const uRealtimeSaved = await subscribeRealtimeSaved((artifact) => {
        prependArtifact(artifact);
      });
      if (unmounted) { uRealtimeSaved(); } else { unsubRealtimeSaved = uRealtimeSaved; }

      const uProvisioningProgress = await subscribeProvisioningProgress((event) => {
        setProvisioning((previous) => ({
          ...previous,
          running: true,
          progress: event,
          statusMessage: `Downloading ${event.asset} (${event.current}/${event.total})`,
        }));
      });
      if (unmounted) { uProvisioningProgress(); } else { unsubProvisioningProgress = uProvisioningProgress; }

      const uProvisioningStatus = await subscribeProvisioningStatus((event) => {
        setProvisioning((previous) => ({
          ...previous,
          running: false,
          statusMessage: event.message,
          progress: event.state === "completed" ? previous.progress : null,
          ready: event.state === "completed" ? true : previous.ready,
        }));

        if (event.state === "completed") {
          void refreshProvisioningStatus();
          void refreshProvisioningModels();
        }
      });
      if (unmounted) { uProvisioningStatus(); } else { unsubProvisioningStatus = uProvisioningStatus; }

    })();

    return () => {
      unmounted = true;
      unsubProgress?.();
      unsubCompleted?.();
      unsubFailed?.();
      unsubTranscriptionDelta?.();
      unsubRealtimeDelta?.();
      unsubRealtimeStatus?.();
      unsubRealtimeSaved?.();
      unsubProvisioningProgress?.();
      unsubProvisioningStatus?.();
      clearStartupWatchdog();
    };
  }, [clearActiveJob, clearStartupWatchdog, prependArtifact, setError, setProgress]);

  useEffect(() => {
    if (!settings) return;

    const templates = settings.prompts.templates;
    if (templates.length === 0) {
      setActivePromptId("");
      setPromptDraft(null);
      return;
    }

    const selected = templates.find((item) => item.id === activePromptId) ?? templates[0];
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
        artifact.input_path?.toLowerCase().includes(needle) ||
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
        artifact.input_path?.toLowerCase().includes(needle) ||
        artifact.optimized_transcript?.toLowerCase().includes(needle) ||
        artifact.raw_transcript?.toLowerCase().includes(needle)
      );
    });
  }, [deletedArtifacts, deletedSearch]);

  const groupedHistoryArtifacts = useMemo(() => {
    const groups: Array<{ label: string; items: TranscriptArtifact[] }> = [];
    for (const artifact of filteredArtifacts) {
      const label = dayGroupLabel(artifact.updated_at);
      const existing = groups[groups.length - 1];
      if (!existing || existing.label !== label) {
        groups.push({ label, items: [artifact] });
      } else {
        existing.items.push(artifact);
      }
    }
    return groups;
  }, [filteredArtifacts]);

  const groupedRecentArtifacts = useMemo(() => {
    const groups: Array<{ label: string; items: TranscriptArtifact[] }> = [];
    for (const artifact of artifacts.slice(0, 6)) {
      const label = dayGroupLabel(artifact.updated_at);
      const existing = groups[groups.length - 1];
      if (!existing || existing.label !== label) {
        groups.push({ label, items: [artifact] });
      } else {
        existing.items.push(artifact);
      }
    }
    return groups;
  }, [artifacts]);

  const selectedArtifactIdSet = useMemo(
    () => new Set(selectedArtifactIds),
    [selectedArtifactIds],
  );

  const isSelectionMode = selectedArtifactIds.length > 0;

  const homeVisibleArtifactIds = useMemo(
    () => groupedRecentArtifacts.flatMap((group) => group.items.map((artifact) => artifact.id)),
    [groupedRecentArtifacts],
  );

  const historyVisibleArtifactIds = useMemo(
    () => groupedHistoryArtifacts.flatMap((group) => group.items.map((artifact) => artifact.id)),
    [groupedHistoryArtifacts],
  );

  const canStartFileTranscription = useMemo(() => {
    if (!settings || !selectedFile || isStarting || Boolean(activeJobId)) {
      return false;
    }

    const selectedModel = settings.transcription.model;
    const modelEntry = modelCatalog.find((entry) => entry.key === selectedModel);
    if (!modelEntry) {
      return true;
    }

    return modelEntry.installed;
  }, [activeJobId, isStarting, modelCatalog, selectedFile, settings]);

  const canStartRealtime = useMemo(
    () => Boolean(settings && provisioning.ready && realtimeState === "idle"),
    [provisioning.ready, realtimeState, settings],
  );

  useEffect(() => {
    if (section === "home" || section === "history") {
      return;
    }
    setSelectedArtifactIds([]);
  }, [section]);

  useEffect(() => {
    const artifactIds = new Set(artifacts.map((artifact) => artifact.id));
    setSelectedArtifactIds((previous) => previous.filter((id) => artifactIds.has(id)));
  }, [artifacts]);

  const detailSegments = useMemo(
    () => parseTimelineV2Segments(activeArtifact?.metadata?.timeline_v2),
    [activeArtifact?.metadata?.timeline_v2],
  );

  const selectedDetailSegment = useMemo(
    () =>
      selectedSegmentSourceIndex === null
        ? null
        : detailSegments.find((segment) => segment.sourceIndex === selectedSegmentSourceIndex) ?? null,
    [detailSegments, selectedSegmentSourceIndex],
  );

  const contextMenuSegment = useMemo(
    () =>
      segmentContextMenu === null
        ? null
        : detailSegments.find((segment) => segment.sourceIndex === segmentContextMenu.sourceIndex) ?? null,
    [detailSegments, segmentContextMenu],
  );

  const knownSpeakerLabels = useMemo(() => {
    return Array.from(
      new Set(
        detailSegments
          .map((segment) => segment.speakerLabel?.trim())
          .filter((label): label is string => Boolean(label && label.length > 0)),
      ),
    ).sort((left, right) => left.localeCompare(right, undefined, { sensitivity: "base" }));
  }, [detailSegments]);

  useEffect(() => {
    if (selectedSegmentSourceIndex === null) {
      return;
    }
    const exists = detailSegments.some((segment) => segment.sourceIndex === selectedSegmentSourceIndex);
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
    const selected = detailSegments.find((segment) => segment.sourceIndex === selectedSegmentSourceIndex);
    setSpeakerDraft(selected?.speakerLabel ?? "");
  }, [detailSegments, selectedSegmentSourceIndex]);

  useEffect(() => {
    if (segmentContextMenu === null) {
      return;
    }
    const exists = detailSegments.some((segment) => segment.sourceIndex === segmentContextMenu.sourceIndex);
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
      if (target instanceof Element && target.closest(".segment-context-menu")) {
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

  const detailAudioInputPath = useMemo(
    () =>
      activeArtifact && activeArtifact.kind === "file"
        ? activeArtifact.input_path
        : activeJobId
          ? selectedFile
          : null,
    [activeArtifact, activeJobId, selectedFile],
  );

  const detailAudioFileLabel = useMemo(
    () => (detailAudioInputPath ? fileLabel(detailAudioInputPath) : "Unknown"),
    [detailAudioInputPath],
  );

  const detailAudioFormat = useMemo(() => {
    if (!detailAudioInputPath) {
      return "Unknown";
    }
    const extension = detailAudioInputPath.split(".").pop();
    if (!extension) {
      return "Unknown";
    }
    return extension.toUpperCase();
  }, [detailAudioInputPath]);

  const transcriptSeconds = useMemo(() => {
    if (audioDurationSeconds > 0) {
      return Math.round(audioDurationSeconds);
    }
    const timelineDuration = detailSegments.reduce((maxSeconds, segment) => {
      const candidate = segment.endSeconds ?? segment.startSeconds ?? 0;
      return Math.max(maxSeconds, candidate);
    }, 0);
    if (timelineDuration > 0) {
      return Math.round(timelineDuration);
    }
    return 0;
  }, [audioDurationSeconds, detailSegments]);

  const transcriptWordCount = useMemo(
    () => draftTranscript.split(/\s+/).filter(Boolean).length,
    [draftTranscript],
  );

  const activeQueueJob = useMemo(
    () => (activeJobId ? queueItems.find((item) => item.job_id === activeJobId) ?? null : null),
    [activeJobId, queueItems],
  );

  const activeTranscriptionPercentage = useMemo(() => {
    return percentageFromJobProgress(activeQueueJob ?? progress);
  }, [activeQueueJob, progress]);

  const queueActiveItems = useMemo(
    () =>
      queueItems.filter((entry) => !["completed", "cancelled", "failed"].includes(entry.stage)),
    [queueItems],
  );

  const exportPreviewText = useMemo(() => {
    const transcript = draftTranscript.trim();
    if (transcript) {
      return transcript;
    }
    return activeArtifact?.raw_transcript?.trim() ?? "";
  }, [activeArtifact?.raw_transcript, draftTranscript]);

  const selectedPromptTemplate = useMemo(() => {
    if (!settings || !activePromptId) return null;
    return settings.prompts.templates.find((template) => template.id === activePromptId) ?? null;
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
    const groups = new Map<SettingsPaneDefinition["group"], SettingsPaneDefinition[]>();

    for (const pane of visibleSettingsPanes) {
      const existing = groups.get(pane.group);
      if (existing) {
        existing.push(pane);
      } else {
        groups.set(pane.group, [pane]);
      }
    }

    return Array.from(groups.entries()).map(([group, panes]) => ({ group, panes }));
  }, [visibleSettingsPanes]);

  const enabledRemoteServices = useMemo(() => {
    return (settings?.ai.remote_services ?? []).filter((service) => service.enabled);
  }, [settings?.ai.remote_services]);

  const activeAiServiceSelectValue = useMemo(() => {
    if (!settings) return AI_SERVICE_NONE;
    if (settings.ai.active_provider === "foundation_apple") {
      return AI_SERVICE_FOUNDATION;
    }

    const activeRemoteId = settings.ai.active_remote_service_id;
    if (activeRemoteId && enabledRemoteServices.some((service) => service.id === activeRemoteId)) {
      return `remote:${activeRemoteId}`;
    }

    if (settings.ai.active_provider === "gemini") {
      const googleService = enabledRemoteServices.find((service) => service.kind === "google");
      if (googleService) {
        return `remote:${googleService.id}`;
      }
    }

    return AI_SERVICE_NONE;
  }, [enabledRemoteServices, settings]);

  const aiServiceSelectOptions = useMemo(() => {
    const options: Array<{ value: string; label: string; disabled?: boolean }> = [
      { value: AI_SERVICE_NONE, label: "No AI provider" },
    ];

    if (platformIsAppleSilicon) {
      options.push({
        value: AI_SERVICE_FOUNDATION,
        label: "Foundation Model",
        disabled: !settings?.ai.providers.foundation_apple.enabled,
      });
    }

    for (const service of enabledRemoteServices) {
      const label = service.kind === "google"
        ? `Google (${service.model?.trim() || settings?.ai.providers.gemini.model || "Gemini"})`
        : service.label || formatProviderLabel(service.kind);

      options.push({
        value: `remote:${service.id}`,
        label,
      });
    }

    return options;
  }, [
    enabledRemoteServices,
    platformIsAppleSilicon,
    settings?.ai.providers.foundation_apple.enabled,
    settings?.ai.providers.gemini.model,
  ]);

  useEffect(() => {
    setAudioDurationSeconds(0);
  }, [detailAudioInputPath]);

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
  }): void {
    setProvisioning((previous) => ({
      ...previous,
      ready: status.ready,
      modelsDir: status.models_dir,
      missing: [...status.missing_models, ...status.missing_encoders],
      running: false,
      progress: null,
      statusMessage: status.ready
        ? "Local models are ready"
        : `${status.missing_models.length + status.missing_encoders.length} model assets missing`,
    }));
  }

  function hydrateDetail(artifact: TranscriptArtifact): void {
    setActiveArtifact(artifact);
    setDraftTitle(artifact.title);
    setDraftTranscript(artifact.optimized_transcript || artifact.raw_transcript);
    setDraftSummary(artifact.summary);
    setDraftFaqs(artifact.faqs);
    setChatInput("");
    setChatHistory([]);
    setDetailMode("transcript");
    setInspectorMode("details");
  }

  async function persistSettings(updated: AppSettings, previous: AppSettings | null): Promise<void> {
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
      setError(`Could not save settings: ${formatAppError(settingsError)}`);
    }
  }

  async function patchSettings(mutator: (current: AppSettings) => AppSettings): Promise<void> {
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
      setError(`Could not save settings: ${formatAppError(settingsError)}`);
    }
  }

  async function refreshSettingsFromDisk(): Promise<void> {
    try {
      const next = await fetchSettingsSnapshot();
      setSettings(normalizeSettings(next));
    } catch (error) {
      setError(`Could not reload settings: ${formatAppError(error)}`);
    }
  }

  async function refreshProvisioningStatus(): Promise<void> {
    try {
      const status = await provisioningStatus();
      setProvisioningState(status);
    } catch (statusError) {
      setError(`Could not read provisioning status: ${formatAppError(statusError)}`);
    }
  }

  async function refreshRuntimeHealth(): Promise<void> {
    try {
      const health = await fetchRuntimeHealth();
      setRuntimeHealth(health);
    } catch (healthError) {
      setError(`Could not read transcription runtime health: ${formatAppError(healthError)}`);
    }
  }

  async function refreshProvisioningModels(): Promise<void> {
    try {
      const [models, health] = await Promise.all([provisioningModels(), fetchRuntimeHealth()]);
      setModelCatalog(models);
      setRuntimeHealth(health);
    } catch (modelsError) {
      setError(`Could not read models catalog: ${formatAppError(modelsError)}`);
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
      const targetService = (current.remote_services ?? []).find((service) => service.id === targetId);
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
          name: "Audio/Video",
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
      setSelectedFile(picked);
      setError(null);
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

  async function onChangeTranscriptionEngine(engine: TranscriptionEngine): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        engine,
      },
    }));
  }

  async function onToggleAi(enabled: boolean): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        enable_ai_post_processing: enabled,
      },
    }));
  }

  async function onPatchWhisperOptions(
    mutator: (current: WhisperOptions) => WhisperOptions,
  ): Promise<void> {
    await patchSettings((current) => ({
      ...current,
      transcription: {
        ...current.transcription,
        whisper_options: sanitizeWhisperOptions(
          mutator(current.transcription.whisper_options ?? getDefaultWhisperOptions(platformIsAppleSilicon)),
        ),
      },
    }));
  }

  async function onStartTranscription(fileToProcess?: string): Promise<void> {
    const targetFile = fileToProcess && typeof fileToProcess === "string" ? fileToProcess : selectedFile;
    if (!settings || !targetFile) return;

    clearStartupWatchdog();
    setIsStarting(true);
    setError(null);
    try {
      const runtimeStatus = await withTimeout(
        ensureTranscriptionRuntime(),
        20_000,
        "Runtime setup timed out.",
      );
      if (!runtimeStatus.ready) {
        setError(runtimeStatus.message || "Transcription runtime is not ready.");
        setSection("home");
        return;
      }
      if (runtimeStatus.did_setup) {
        void refreshRuntimeHealth();
      }

      let preflight: { allowed: boolean; message: string } | null = null;
      try {
        preflight = await withTimeout(
          fetchTranscriptionStartPreflight({
            model: settings.transcription.model,
          }),
          8_000,
          "Preflight timed out.",
        );
      } catch (preflightError) {
        console.warn("Transcription preflight failed, continuing with backend start:", preflightError);
      }
      if (preflight && !preflight.allowed) {
        setError(preflight.message || "Transcription cannot start on this machine.");
        setSection("home");
        return;
      }

      const startResult = await withTimeout(
        startTranscription({
          input_path: targetFile,
          language: settings.transcription.language,
          model: settings.transcription.model,
          enable_ai: settings.transcription.enable_ai_post_processing,
          whisper_options: sanitizeWhisperOptions(
            settings.transcription.whisper_options ?? getDefaultWhisperOptions(platformIsAppleSilicon),
          ),
        }),
        12_000,
        "Start request timed out while waiting for backend response.",
      );

      const { job_id } = startResult;

      if (failedJobMessagesRef.current.has(job_id)) {
        const earlyFailure =
          failedJobMessagesRef.current.get(job_id) ?? "Transcription failed.";
        failedJobMessagesRef.current.delete(job_id);
        clearActiveJob();
        activeJobIdRef.current = null;
        setActiveJobPreviewText("");
        setActiveJobTitle("");
        activeJobDeltaSequenceRef.current = -1;
        setSection("home");
        setError(earlyFailure);
        return;
      }

      activeJobIdRef.current = job_id;
      setJobStarted(job_id);
      setQueueItems((previous) =>
        pushOrReplaceQueueItem(previous, {
          job_id,
          stage: "queued",
          message: "Queued transcription job.",
          percentage: 0,
          current_seconds: 0,
          total_seconds: null,
        }),
      );
      setActiveJobTitle(fileLabel(targetFile));
      setActiveJobPreviewText("");
      activeJobDeltaSequenceRef.current = -1;
      setActiveArtifact(null);
      setShowExportSheet(false);
      setSection("detail");
      setDetailMode("transcript");
      setInspectorMode("details");

      startupWatchdogRef.current = window.setTimeout(() => {
        if (activeJobIdRef.current !== job_id) {
          return;
        }
        clearActiveJob();
        activeJobIdRef.current = null;
        setActiveJobPreviewText("");
        setActiveJobTitle("");
        activeJobDeltaSequenceRef.current = -1;
        setSection("home");
        setError(
          "Transcription is not starting correctly. Check Whisper CLI/Whisper Stream paths in Settings > Local Models.",
        );
      }, 120_000);
    } catch (startError) {
      const friendlyError = formatAppError(startError);
      setError(`Failed to start transcription: ${friendlyError}`);
      clearStartupWatchdog();
      setSection("home");
    } finally {
      setIsStarting(false);
    }
  }

  async function onCancel(): Promise<void> {
    if (!activeJobId) return;

    try {
      await cancelTranscription(activeJobId);
    } catch (cancelError) {
      setError(`Failed to cancel transcription: ${formatAppError(cancelError)}`);
    }
  }

  async function onOpenArtifact(artifactId: string): Promise<void> {
    try {
      const artifact = await getArtifact(artifactId);
      if (!artifact) {
        setError("Transcript not found.");
        return;
      }

      hydrateDetail(artifact);
      setSection("detail");
      setError(null);
    } catch (artifactError) {
      setError(`Failed to open transcript: ${formatAppError(artifactError)}`);
    }
  }

  async function onOpenStandaloneSettingsWindow(pane?: SettingsPane): Promise<void> {
    try {
      await openSettingsWindow(pane);
    } catch (windowError) {
      setError(`Failed to open settings window: ${formatAppError(windowError)}`);
    }
  }

  function onFocusQueueJob(item: JobProgress): void {
    clearStartupWatchdog();
    const switchingJob = activeJobIdRef.current !== item.job_id;
    setJobStarted(item.job_id);
    setProgress(item);
    activeJobIdRef.current = item.job_id;
    if (switchingJob) {
      setActiveJobPreviewText("");
      activeJobDeltaSequenceRef.current = -1;
      setActiveJobTitle(activeJobTitle || "Transcribing");
    }
    setActiveArtifact(null);
    setDetailMode("transcript");
    setInspectorMode("details");
    setSection("detail");
    setError(null);
  }

  function onCloseOpenedArtifact(): void {
    setShowExportSheet(false);
    setActiveArtifact(null);
    setSection("history");
    setError(null);
  }

  async function onSaveArtifact(): Promise<void> {
    if (!activeArtifact) return;

    setIsSavingArtifact(true);
    try {
      const updated = await updateArtifact({
        id: activeArtifact.id,
        optimized_transcript: draftTranscript,
        summary: draftSummary,
        faqs: draftFaqs,
      });

      if (!updated) {
        setError("Artifact not found while saving.");
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
      setError(`Failed to save changes: ${formatAppError(saveError)}`);
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
      setError("No transcript selected.");
      return;
    }

    const parsedTimeline = parseTimelineV2Document(activeArtifact.metadata?.timeline_v2);
    if (!parsedTimeline) {
      setError("Segment timeline metadata is missing or invalid.");
      return;
    }
    if (sourceIndex < 0 || sourceIndex >= parsedTimeline.segments.length) {
      setError("Selected segment is out of range.");
      return;
    }

    const normalizedSpeakerLabel = speakerLabel?.trim() ?? "";
    const shouldSetSpeaker = normalizedSpeakerLabel.length > 0;
    const speakerId = shouldSetSpeaker ? normalizeSpeakerId(normalizedSpeakerLabel) : null;

    const nextSegments = parsedTimeline.segments.map((segment) => ({ ...segment }));
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
          currentEnd !== null
          && nextStart !== null
          && nextStart - currentEnd > maxGapSeconds
        ) {
          break;
        }

        assignSpeaker(index);
      }

      for (let index = sourceIndex + 1; index < nextSegments.length; index += 1) {
        const segment = nextSegments[index];
        if (readSegmentSpeakerLabel(segment)) {
          break;
        }

        const previousEnd = readSegmentEndSeconds(nextSegments[index - 1]);
        const currentStart = readSegmentStartSeconds(segment);
        if (
          previousEnd !== null
          && currentStart !== null
          && currentStart - previousEnd > maxGapSeconds
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
        setError("Transcript not found while assigning speaker.");
        return;
      }
      upsertArtifact(updated);
      hydrateDetail(updated);
      setError(null);
    } catch (assignError) {
      setError(`Failed to assign speaker: ${formatAppError(assignError)}`);
    } finally {
      setIsAssigningSpeaker(false);
    }
  }

  async function onAssignSpeakerToSelectedSegment(): Promise<void> {
    if (selectedSegmentSourceIndex === null) {
      setError("Select a segment first.");
      return;
    }
    const speakerLabel = speakerDraft.trim();
    if (!speakerLabel) {
      setError("Speaker name cannot be empty.");
      return;
    }
    await onAssignSpeakerToSegment(
      selectedSegmentSourceIndex,
      speakerLabel,
      propagateSpeakerAssignment,
    );
  }

  async function onClearSpeakerForSegment(sourceIndex: number): Promise<void> {
    await onAssignSpeakerToSegment(sourceIndex, null, false);
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
      setError("Title cannot be empty.");
      return;
    }

    if (newTitle === renameTarget.title) {
      closeRenameDialog();
      return;
    }

    setIsRenamingArtifact(true);
    try {
      const updated = await renameArtifact({ id: renameTarget.id, new_title: newTitle });
      if (!updated) {
        setError("Transcript not found while renaming.");
        return;
      }

      upsertArtifact(updated);
      if (activeArtifact?.id === updated.id) {
        hydrateDetail(updated);
      }

      setError(null);
      closeRenameDialog();
    } catch (renameError) {
      setError(`Rename failed: ${formatAppError(renameError)}`);
    } finally {
      setIsRenamingArtifact(false);
    }
  }

  async function onDeleteArtifactsWithConsent(ids: string[]): Promise<void> {
    const uniqueIds = Array.from(new Set(ids));
    if (uniqueIds.length === 0) return;

    const targets = artifacts.filter((artifact) => uniqueIds.includes(artifact.id));
    if (targets.length === 0) return;

    const details =
      targets.length === 1
        ? `"${targets[0].title}"`
        : `${targets.length} transcriptions:\n${targets
            .slice(0, 5)
            .map((artifact) => `- ${artifact.title}`)
            .join("\n")}${targets.length > 5 ? "\n- ..." : ""}`;

    const confirmed = await confirmDialog(
      `Move ${details} to Recently Deleted?\n\nYou can restore these items later from Recently Deleted.`,
      { title: "Move to Recently Deleted", kind: "warning" },
    );
    if (!confirmed) return;

    try {
      const result = await deleteArtifacts(uniqueIds);
      if (result.deleted <= 0) {
        setError("No transcriptions were deleted.");
        return;
      }

      removeArtifacts(uniqueIds);
      await refreshDeletedArtifactsList();

      if (activeArtifact && uniqueIds.includes(activeArtifact.id)) {
        setActiveArtifact(null);
        setSection("history");
      }

      setError(null);
    } catch (deleteError) {
      setError(`Delete failed: ${formatAppError(deleteError)}`);
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
    const visibleIds = section === "history" ? historyVisibleArtifactIds : homeVisibleArtifactIds;
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
        ? "Restore this transcription from Recently Deleted?"
        : `Restore ${uniqueIds.length} transcriptions from Recently Deleted?`,
      { title: "Restore transcription" },
    );
    if (!confirmed) return;

    try {
      const result = await restoreArtifacts(uniqueIds);
      if (result.restored <= 0) {
        setError("No transcriptions were restored.");
        return;
      }

      await Promise.all([refreshActiveArtifacts(), refreshDeletedArtifactsList()]);
      setError(null);
    } catch (restoreError) {
      setError(`Restore failed: ${formatAppError(restoreError)}`);
    }
  }

  async function onHardDeleteArtifactsWithConsent(ids: string[]): Promise<void> {
    const uniqueIds = Array.from(new Set(ids));
    if (uniqueIds.length === 0) return;

    const confirmed = await confirmDialog(
      uniqueIds.length === 1
        ? "Permanently delete this transcription from Recently Deleted? This action cannot be undone."
        : `Permanently delete ${uniqueIds.length} transcriptions from Recently Deleted? This action cannot be undone.`,
      { title: "Permanent delete", kind: "warning" },
    );
    if (!confirmed) return;

    try {
      const result = await hardDeleteArtifacts(uniqueIds);
      if (result.deleted <= 0) {
        await refreshDeletedArtifactsList();
        setError("No transcriptions were permanently deleted.");
        return;
      }

      await refreshDeletedArtifactsList();
      setError(null);
    } catch (deleteError) {
      setError(`Permanent delete failed: ${formatAppError(deleteError)}`);
    }
  }

  async function onEmptyTrash(): Promise<void> {
    const confirmed = await confirmDialog(
      "Empty Recently Deleted? This permanently deletes all trashed transcriptions.",
      { title: "Empty Recently Deleted", kind: "warning" },
    );
    if (!confirmed) return;

    try {
      const result = await emptyDeletedArtifacts();
      if (result.deleted <= 0) {
        await refreshDeletedArtifactsList();
        setError("Recently Deleted is already empty.");
        return;
      }
      await refreshDeletedArtifactsList();
      setError(null);
    } catch (emptyError) {
      setError(`Empty trash failed: ${formatAppError(emptyError)}`);
    }
  }

  function formatHomeTime(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  async function onExport(payload: ExportRequest): Promise<void> {
    if (!activeArtifact) return;

    let artifactForExport = activeArtifact;
    const hasDraftChanges =
      draftTranscript !== activeArtifact.optimized_transcript ||
      draftSummary !== activeArtifact.summary ||
      draftFaqs !== activeArtifact.faqs;

    if (hasDraftChanges) {
      try {
        const updated = await updateArtifact({
          id: activeArtifact.id,
          optimized_transcript: draftTranscript,
          summary: draftSummary,
          faqs: draftFaqs,
        });
        if (updated) {
          artifactForExport = updated;
          upsertArtifact(updated);
          setActiveArtifact(updated);
        }
      } catch (syncError) {
        setError(`Could not sync changes before export: ${formatAppError(syncError)}`);
        return;
      }
    }

    const destination = await save({
      defaultPath: `${artifactForExport.title.replace(/\s+/g, "_")}.${payload.format}`,
    });

    if (!destination) {
      return;
    }

    try {
      await exportArtifact({
        id: artifactForExport.id,
        format: payload.format,
        destination_path: destination,
        style: payload.style,
        options: {
          include_timestamps: payload.options.includeTimestamps,
          grouping: payload.options.grouping,
        },
        segments: payload.segments,
        content_override: payload.contentOverride,
      });
      setError(null);
    } catch (exportError) {
      setError(`Export failed: ${formatAppError(exportError)}`);
      throw exportError;
    }
  }

  async function onStartRealtime(): Promise<void> {
    if (!settings) return;

    try {
      setRealtimeFinalLines([]);
      setRealtimePreview("");
      await startRealtime({
        model: settings.transcription.model,
        language: settings.transcription.language,
      });
      setSection("realtime");
      setError(null);
    } catch (startError) {
      setError(`Realtime start failed: ${formatAppError(startError)}`);
    }
  }

  async function onPauseRealtime(): Promise<void> {
    try {
      await pauseRealtime();
    } catch (pauseError) {
      setError(`Realtime pause failed: ${formatAppError(pauseError)}`);
    }
  }

  async function onResumeRealtime(): Promise<void> {
    try {
      await resumeRealtime();
    } catch (resumeError) {
      setError(`Realtime resume failed: ${formatAppError(resumeError)}`);
    }
  }

  async function onStopRealtime(saveResult: boolean): Promise<void> {
    setIsStoppingRealtime(true);
    try {
      const result = await stopRealtime(saveResult);
      if (result.artifact) {
        prependArtifact(result.artifact);
        hydrateDetail(result.artifact);
        setSection("detail");
      }
      setRealtimePreview("");
      setRealtimeFinalLines([]);
      setError(null);
    } catch (stopError) {
      setError(`Realtime stop failed: ${formatAppError(stopError)}`);
    } finally {
      setIsStoppingRealtime(false);
    }
  }

  async function onGenerateSummary(): Promise<void> {
    if (!activeArtifact || isGeneratingSummary) return;

    const optionLines = [
      summarySections ? "Use sections." : "Use a single section.",
      summaryBulletPoints ? "Use bullet points." : "Avoid bullet points.",
      summaryActionItems ? "Include action items." : "Do not include action items.",
      summaryKeyPointsOnly ? "Keep only key points." : "Provide full detail.",
      summaryIncludeTimestamps ? "Include timestamps when available." : "Do not include timestamps.",
      summaryIncludeSpeakers ? "Include speaker names when available." : "Do not include speaker names.",
      `Write the output in ${summaryLanguage}.`,
    ];

    const prompt = summaryCustomPrompt.trim().length > 0
      ? summaryCustomPrompt.trim()
      : `Summarize this transcript.\n${optionLines.join("\n")}`;

    setIsGeneratingSummary(true);
    try {
      const answer = await summarizeArtifact({ id: activeArtifact.id, prompt });
      setDraftSummary(answer);
      setDetailMode("summary");
      setError(null);
    } catch (summaryError) {
      const code = formatAppErrorCode(summaryError);
      const message = formatAppError(summaryError);
      if (code === "missing_ai_provider" || code === "missing_api_key") {
        setError("Summary failed: configure an AI provider in Settings > AI Services.");
      } else {
        setError(`Summary failed: ${message}`);
      }
    } finally {
      setIsGeneratingSummary(false);
    }
  }

  async function onImproveText(): Promise<void> {
    if (!activeArtifact || isImprovingText) return;

    if (draftTranscript.trim() === "") {
        return;
    }

    setIsImprovingText(true);
    try {
      const optimizedText = await optimizeArtifact({ id: activeArtifact.id, text: draftTranscript });
      setDraftTranscript(optimizedText);
      setError(null);
    } catch (optimizeError) {
      const code = formatAppErrorCode(optimizeError);
      const message = formatAppError(optimizeError);
      if (code === "missing_ai_provider" || code === "missing_api_key") {
        setError("Improve text failed: configure an AI provider in Settings > AI Services.");
      } else {
        setError(`Improve text failed: ${message}`);
      }
    } finally {
      setIsImprovingText(false);
    }
  }

  async function onSendChat(prefilledPrompt?: string): Promise<void> {
    if (!activeArtifact || isAskingChat) return;

    const prompt = (prefilledPrompt ?? chatInput).trim();
    if (!prompt) return;

    if (!prefilledPrompt) {
      setChatInput("");
    }

    setIsAskingChat(true);
    setChatHistory((previous) => [...previous, { role: "user", text: prompt }]);

    try {
      const answer = await chatArtifact({ id: activeArtifact.id, prompt });
      setChatHistory((previous) => [...previous, { role: "assistant", text: answer }]);
    } catch (chatError) {
      const code = formatAppErrorCode(chatError);
      const message = formatAppError(chatError);
      const providerMissing = code === "missing_ai_provider" || code === "missing_api_key";
      setChatHistory((previous) => [
        ...previous,
        {
          role: "assistant",
          text: providerMissing
            ? "Chat unavailable. Configure AI provider in Settings > AI Services."
            : `Chat failed: ${message}`,
        },
      ]);
      setError(providerMissing ? "Chat unavailable. Configure AI provider in Settings > AI Services." : message);
    } finally {
      setIsAskingChat(false);
    }
  }

  async function onProvisionModels(): Promise<void> {
    try {
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        statusMessage: "Provisioning started...",
      }));
      await provisioningStart(true);
    } catch (provisionError) {
      setProvisioning((previous) => ({
        ...previous,
        running: false,
      }));
      setError(`Provisioning failed: ${formatAppError(provisionError)}`);
    }
  }

  async function onDownloadModel(model: SpeechModel): Promise<void> {
    try {
      setProvisioning((previous) => ({
        ...previous,
        running: true,
        statusMessage: `Downloading ${model}...`,
      }));
      await provisioningDownloadModel({ model, include_coreml: true });
    } catch (downloadError) {
      setProvisioning((previous) => ({
        ...previous,
        running: false,
      }));
      setError(`Model download failed: ${formatAppError(downloadError)}`);
    }
  }

  async function onCancelProvisioning(): Promise<void> {
    try {
      await provisioningCancel();
      setProvisioning((previous) => ({
        ...previous,
        running: false,
        statusMessage: "Provisioning cancelled",
      }));
    } catch (cancelError) {
      setError(`Provisioning cancel failed: ${formatAppError(cancelError)}`);
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
        setError(`Update check failed: ${formatAppError(updateError)}`);
      }
    } finally {
      if (!shouldCancel?.()) {
        setCheckingUpdates(false);
      }
    }
  }

  async function onRefreshUpdates(): Promise<void> {
    await refreshUpdates(false);
  }

  async function onInstallUpdate(): Promise<void> {
    if (!nativeUpdate) {
      return;
    }

    setInstallingUpdate(true);
    setUpdateStatusMessage("Downloading update...");
    setUpdateDownloadPercent(0);
    try {
      let expectedBytes = 0;
      let downloadedBytes = 0;
      await nativeUpdate.downloadAndInstall((event) => {
        if (event.event === "Started") {
          expectedBytes = event.data.contentLength ?? 0;
          downloadedBytes = 0;
          setUpdateStatusMessage("Downloading update...");
          return;
        }
        if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          if (expectedBytes > 0) {
            const percent = Math.max(
              0,
              Math.min(100, Math.round((downloadedBytes / expectedBytes) * 100)),
            );
            setUpdateDownloadPercent(percent);
          }
          return;
        }
        if (event.event === "Finished") {
          setUpdateDownloadPercent(100);
          setUpdateStatusMessage("Installing update...");
        }
      });

      setUpdateStatusMessage("Update installed. Restart the app to apply it.");
      setUpdateInfo((previous) =>
        previous
          ? {
              ...previous,
              has_update: false,
              current_version: previous.latest_version ?? previous.current_version,
            }
          : previous,
      );
      setNativeUpdate(null);
    } catch (installError) {
      setError(`Update install failed: ${formatAppError(installError)}`);
      setUpdateStatusMessage(null);
    } finally {
      setInstallingUpdate(false);
    }
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
        output: `Prompt test failed: ${formatAppError(error)}`,
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
      setError(`Could not reset prompts: ${formatAppError(error)}`);
    }
  }

  function renderHome(): JSX.Element {
    return (
      <div className="view-body home-view">
        <section className="main-input-bar">
          <input
            type="text"
            placeholder="Audio file"
            value={selectedFile ? fileLabel(selectedFile) : ""}
            readOnly
          />
          <button className="icon-button" onClick={() => void onPickFile()} title="Open Local File">
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
            {isStarting ? "Starting..." : "Start Transcription"}
          </button>
          <button className="quick-action" onClick={() => void onStartRealtime()} disabled={!canStartRealtime}>
            <Mic size={18} strokeWidth={2.5} />
            Start Live
          </button>
          <button
            className="quick-action"
            onClick={() => void onStopRealtime(true)}
            disabled={realtimeState === "idle" || isStoppingRealtime}
          >
            <Radio size={18} strokeWidth={2.5} />
            {isStoppingRealtime ? "Stopping..." : "Stop Live & Save"}
          </button>
          <button className="quick-action" onClick={() => setSection("queue")}>
            <ListChecks size={18} strokeWidth={2.5} />
            Queue
          </button>
          <button className="quick-action" onClick={() => setSection("history")}>
            <HistoryIcon size={18} strokeWidth={2.5} />
            History
          </button>
        </div>

        <section className="panel-card">
          {isSelectionMode ? (
            <div className="history-selection-toolbar">
              <button className="secondary-button" onClick={() => setSelectedArtifactIds([])}>
                Cancel
              </button>
              <button
                className="secondary-button"
                onClick={selectAllVisibleArtifacts}
                disabled={homeVisibleArtifactIds.length === 0}
              >
                Select all
              </button>
              <button
                className="secondary-button history-action-danger"
                onClick={() => void onDeleteSelectedArtifacts()}
                disabled={selectedArtifactIds.length === 0}
              >
                Delete selected ({selectedArtifactIds.length})
              </button>
            </div>
          ) : null}
          {groupedRecentArtifacts.length === 0 ? (
            <div className="center-empty compact">
              <h3>No transcripts yet</h3>
              <p>Open a file and start transcription to begin.</p>
            </div>
          ) : (
            <div className="history-groups">
              {groupedRecentArtifacts.map((group) => (
                <section key={group.label} className="history-group">
                  <h3 className="history-group-label">{group.label}</h3>
                  <div className={`history-list ${isSelectionMode ? "selection-active" : ""}`}>
                    {group.items.map((artifact) => (
                      <article
                        key={artifact.id}
                        className={`history-item home-history-item${selectedArtifactIdSet.has(artifact.id) ? " selected" : ""}`}
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
                            <div className="home-history-head">
                              <strong>
                                <HighlightMatch text={artifact.title} search={search} />
                              </strong>
                              <span className="history-inline-time">{formatHomeTime(artifact.updated_at)}</span>
                            </div>
                            <p className="history-preview">
                              <HighlightMatch
                                text={previewSnippet(artifact.optimized_transcript || artifact.raw_transcript, 210)}
                                search={search}
                              />
                            </p>
                          </div>
                        </button>
                        <div className="home-history-actions">
                          <label className="home-history-select" title="Select transcription" onClick={(e) => e.stopPropagation()}>
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
                              title="Move to trash"
                              aria-label={`Move ${artifact.title} to trash`}
                            >
                              <Trash2 size={14} />
                            </button>
                          ) : null}
                        </div>
                      </article>
                    ))}
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
          <h2>Queue</h2>
          <div className="toolbar-actions">
            <button className="secondary-button" onClick={() => setQueueItems([])}>
              Clear Finished
            </button>
            <button className="secondary-button" onClick={() => void onCancel()} disabled={!activeJobId}>
              Cancel Active Job
            </button>
          </div>
        </div>

        {queueActiveItems.length === 0 ? (
          <div className="center-empty">
            <div className="center-empty-icon"><ListChecks size={28} /></div>
            <h3>No Active Transcriptions</h3>
            <p>Active transcriptions will appear here.</p>
          </div>
        ) : (
          <div className="queue-list">
            {queueActiveItems.map((item) => {
              const displayPercentage =
                item.job_id === activeJobId
                  ? activeTranscriptionPercentage
                  : percentageFromJobProgress(item);

              return (
                <article
                  key={item.job_id}
                  className="queue-card queue-card-clickable"
                  onClick={() => onFocusQueueJob(item)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      onFocusQueueJob(item);
                    }
                  }}
                >
                  <div className="queue-card-head">
                    <strong>
                      {item.job_id === activeJobId && activeJobTitle ? activeJobTitle : "Transcription in progress"}
                    </strong>
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
    return (
      <div className="view-body history-view">
        {isSelectionMode ? (
          <div className="history-selection-toolbar">
            <button className="secondary-button" onClick={() => setSelectedArtifactIds([])}>
              Cancel
            </button>
            <button
              className="secondary-button"
              onClick={selectAllVisibleArtifacts}
              disabled={historyVisibleArtifactIds.length === 0}
            >
              Select all
            </button>
            <button
              className="secondary-button history-action-danger"
              onClick={() => void onDeleteSelectedArtifacts()}
              disabled={selectedArtifactIds.length === 0}
            >
              Delete selected ({selectedArtifactIds.length})
            </button>
          </div>
        ) : null}
        {groupedHistoryArtifacts.length === 0 ? (
          <div className="center-empty">
            <div className="center-empty-icon"><Clock3 size={28} /></div>
            <h3>No Transcriptions Yet</h3>
          </div>
        ) : (
          <div className="history-groups">
            {groupedHistoryArtifacts.map((group) => (
              <section key={group.label} className="history-group">
                <h3 className="history-group-label">{group.label}</h3>
                <div className={`history-list ${isSelectionMode ? "selection-active" : ""}`}>
                  {group.items.map((artifact) => (
                    <article
                      key={artifact.id}
                      className={`history-item${selectedArtifactIdSet.has(artifact.id) ? " selected" : ""}`}
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
                          <div className="home-history-head">
                            <strong>
                              <HighlightMatch text={artifact.title} search={search} />
                            </strong>
                            <span className="history-inline-time">{formatHomeTime(artifact.updated_at)}</span>
                          </div>
                          <p className="history-preview">
                            <HighlightMatch
                              text={previewSnippet(artifact.optimized_transcript || artifact.raw_transcript, 220)}
                              search={search}
                            />
                          </p>
                        </div>
                      </button>
                      <div className="history-actions">
                        <label className="home-history-select history-select" title="Select transcription" onClick={(e) => e.stopPropagation()}>
                          <input
                            type="checkbox"
                            checked={selectedArtifactIdSet.has(artifact.id)}
                            onChange={() => toggleArtifactSelection(artifact.id)}
                          />
                        </label>
                        {!isSelectionMode ? (
                          <>
                            <span className="kind-chip">{artifact.kind === "realtime" ? "Live" : "File"}</span>
                            <button className="secondary-button history-action-button" onClick={() => void onRenameArtifact(artifact)}>
                              <Pencil size={14} />
                              Rename
                            </button>
                            <button className="secondary-button history-action-button history-action-danger" onClick={(event) => {
                                event.stopPropagation();
                                void onDeleteArtifact(artifact);
                            }}>
                              <Trash2 size={14} />
                              Move to Trash
                            </button>
                          </>
                        ) : null}
                      </div>
                    </article>
                  ))}
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
            <div className="center-empty-icon"><Trash2 size={28} /></div>
            <h3>Recently Deleted Is Empty</h3>
            <p>Deleted transcriptions will appear here for up to 30 days.</p>
          </div>
        ) : (
          <div className="history-list">
            {filteredDeletedArtifacts.map((artifact) => (
              <article key={artifact.id} className="history-item deleted-history-item">
                <div className="history-main">
                  <div>
                    <strong>
                      <HighlightMatch text={artifact.title} search={search} />
                    </strong>
                    <p>
                      <HighlightMatch
                        text={previewSnippet(artifact.optimized_transcript || artifact.raw_transcript, 220)}
                        search={search}
                      />
                    </p>
                  </div>
                  <small>{formatDate(artifact.updated_at)}</small>
                </div>
                <div className="history-actions">
                  <span className="kind-chip">{artifact.kind === "realtime" ? "Live" : "File"}</span>
                  <button
                    className="secondary-button history-action-button"
                    onClick={() => void onRestoreArtifactsWithConsent([artifact.id])}
                  >
                    Restore
                  </button>
                  <button
                    className="secondary-button history-action-button history-action-danger"
                    onClick={() => void onHardDeleteArtifactsWithConsent([artifact.id])}
                  >
                    <Trash2 size={14} />
                    Delete Permanently
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
    if (!activeArtifact) {
      if (activeJobId) {
        if (!activeJobPreviewText) {
          return <LoadingAnimation />;
        }
        return (
          <textarea
            ref={activeJobPreviewTextareaRef}
            className="detail-editor"
            value={activeJobPreviewText}
            readOnly
            placeholder="Transcribing... text will appear here while Whisper is running."
            style={{ fontSize: `${fontSize}px` }}
          />
        );
      }

      return (
        <div className="center-empty">
          <h3>Select a transcript from History</h3>
        </div>
      );
    }

    if (detailMode === "summary") {
      if (isGeneratingSummary) {
        return (
          <LoadingAnimation
            icon={Sparkles}
            title="Summarizing..."
            description="Generating a summary from your transcript..."
            variant="summarizing"
          />
        );
      }

      if (!draftSummary) {
        return (
          <div className="detail-empty">
            <div className="center-empty-icon"><Sparkles size={28} /></div>
            <h2>AI Summary</h2>
            <p>Generate a summary from the right panel options.</p>
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

    if (detailMode === "chat") {
      return (
        <div className="chat-view">
          <div className="chat-thread">
            {isAskingChat ? (
              <LoadingAnimation
                icon={MessageSquareText}
                title="Thinking..."
                description="AI is analyzing your transcript and generating a response..."
                variant="chat"
              />
            ) : chatHistory.length === 0 ? (
              <div className="detail-empty">
                <div className="center-empty-icon"><MessageSquareText size={28} /></div>
                <h2>AI Chat</h2>
                <p>Ask questions on the current transcript.</p>
              </div>
            ) : null}
            {!isAskingChat && chatHistory.map((message, index) => (
              <article key={`${message.role}-${index}`} className={`chat-bubble ${message.role}`}>
                {message.text}
              </article>
            ))}
          </div>

          <div className="chat-input-bar">
            <input
              placeholder="Chat with your transcript..."
              value={chatInput}
              onChange={(event) => setChatInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  if (!isAskingChat && chatInput.trim().length > 0) {
                    void onSendChat();
                  }
                }
              }}
            />
            <select
              value={activeAiServiceSelectValue}
              onChange={(event) => {
                void onSelectAiService(event.target.value);
              }}
            >
              {aiServiceSelectOptions.map((option) => (
                <option key={option.value} value={option.value} disabled={option.disabled}>
                  {option.label}
                </option>
              ))}
            </select>
            <button 
              className="chat-submit-button" 
              onClick={() => void onSendChat()} 
              disabled={isAskingChat || chatInput.trim().length === 0}
              aria-label="Submit Chat"
            >
              <ArrowUp size={20} />
            </button>
          </div>
        </div>
      );
    }

    if (detailMode === "segments") {
      return (
        <div className="segments-view">
          {detailSegments.length === 0 ? <p className="muted">No segments yet</p> : null}
          {detailSegments.map((segment, index) => (
            <article
              className={`segment-row ${
                selectedSegmentSourceIndex === segment.sourceIndex ? "selected" : ""
              } ${segmentContextMenu?.sourceIndex === segment.sourceIndex ? "context-open" : ""}`}
              key={`${segment.time}-${index}`}
              onClick={() => setSelectedSegmentSourceIndex(segment.sourceIndex)}
              onContextMenu={(event) => openSegmentContextMenu(event, segment.sourceIndex)}
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
                  <span className="kind-chip">{segment.speakerLabel}</span>
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
          title="Improving Text..."
          description="AI is optimizing semantics and punctuation..."
          variant="transcribing"
        />
      );
    }

    return (
      <div className="transcript-container" style={{ position: "relative", height: "100%" }}>
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
            <HighlightMatch text={draftTranscript} search={search} />
          </div>
        ) : null}
        <textarea
          className="detail-editor"
          value={draftTranscript}
          onChange={(event) => setDraftTranscript(event.target.value)}
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
      </div>
    );
  }

  function renderDefaultInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <button className="secondary-button" onClick={() => void navigator.clipboard.writeText(draftTranscript)}>
          Copy
        </button>

        <TranscriptSegmentsTileSwitch
          detailMode={detailMode}
          onSelectMode={(mode) => setDetailMode(mode)}
        />

        <div className="inspector-block">
          <h4>Audio</h4>
          <div className="property-line">
            <span>File</span>
            <strong className="truncate-value" title={detailAudioFileLabel}>{detailAudioFileLabel}</strong>
          </div>
          <div className="property-line">
            <span>Format</span>
            <strong>{detailAudioFormat}</strong>
          </div>
          <div className="property-line">
            <span>Duration</span>
            <strong>{formatShortDuration(transcriptSeconds)}</strong>
          </div>
        </div>

        <div className="inspector-block">
          <h4>Title</h4>
          <input
            className="inspector-input"
            value={draftTitle}
            onChange={(event) => setDraftTitle(event.target.value)}
          />
        </div>

        <div className="inspector-block">
          <h4>People</h4>
          <div className="pill">{selectedDetailSegment?.speakerLabel ?? "Unknown"}</div>
          <div className="speaker-edit-row">
            <input
              ref={peopleSpeakerInputRef}
              className="inspector-input"
              placeholder={
                detailMode === "segments"
                  ? "Add a speaker..."
                  : "Open Segments and select a segment"
              }
              list="speaker-suggestions"
              value={speakerDraft}
              disabled={detailMode !== "segments" || !activeArtifact || selectedSegmentSourceIndex === null}
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
                isAssigningSpeaker
                || detailMode !== "segments"
                || !activeArtifact
                || selectedSegmentSourceIndex === null
                || !speakerDraft.trim()
              }
              onClick={() => void onAssignSpeakerToSelectedSegment()}
            >
              {isAssigningSpeaker ? "..." : "Assign"}
            </button>
          </div>
          <label className="toggle-row compact">
            <span>Propagate to adjacent unlabeled segments</span>
            <input
              type="checkbox"
              checked={propagateSpeakerAssignment}
              disabled={detailMode !== "segments"}
              onChange={(event) => setPropagateSpeakerAssignment(event.target.checked)}
            />
          </label>
          {knownSpeakerLabels.length > 0 ? (
            <datalist id="speaker-suggestions">
              {knownSpeakerLabels.map((speaker) => (
                <option key={speaker} value={speaker} />
              ))}
            </datalist>
          ) : null}
          <small className="muted">
            {detailMode === "segments"
              ? selectedSegmentSourceIndex === null
                ? "Select a segment to assign a speaker manually."
                : "Speaker label is saved into segment metadata. Right-click a segment for quick actions."
              : "Speaker assignment is available in Segments mode. Diarization controls are in Settings > Whisper C++."}
          </small>
        </div>

        <div className="inspector-block">
          <h4>Options</h4>
          <label className="toggle-row">
            <span>Font Size</span>
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

          <label className="toggle-row">
            <span>Favorites Only</span>
            <input
              type="checkbox"
              checked={favoritesOnly}
              onChange={(event) => setFavoritesOnly(event.target.checked)}
            />
          </label>

          <label className="toggle-row">
            <span>Group segments without speakers</span>
            <input
              type="checkbox"
              checked={groupSegmentsWithoutSpeakers}
              onChange={(event) => setGroupSegmentsWithoutSpeakers(event.target.checked)}
            />
          </label>
        </div>
      </div>
    );
  }

  function renderSummaryInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <label>
          AI Service
          <select
            className="inspector-select"
            value={activeAiServiceSelectValue}
            onChange={(event) => {
              void onSelectAiService(event.target.value);
            }}
          >
            {aiServiceSelectOptions.map((option) => (
              <option key={option.value} value={option.value} disabled={option.disabled}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <button className="secondary-button" onClick={() => void navigator.clipboard.writeText(draftSummary || draftTranscript)}>
          Copy
        </button>
        <button className="secondary-button" onClick={() => void onGenerateSummary()} disabled={isGeneratingSummary || !activeArtifact}>
          {isGeneratingSummary ? "Summarizing..." : "Summarize"}
        </button>
        <button className="secondary-button" onClick={() => setDraftSummary("")}>Clear</button>

        <label className="toggle-row">
          <span>Include timestamps</span>
          <input
            type="checkbox"
            checked={summaryIncludeTimestamps}
            onChange={(event) => setSummaryIncludeTimestamps(event.target.checked)}
          />
        </label>
        <label className="toggle-row">
          <span>Include speakers</span>
          <input
            type="checkbox"
            checked={summaryIncludeSpeakers}
            onChange={(event) => setSummaryIncludeSpeakers(event.target.checked)}
          />
        </label>
        <label className="toggle-row">
          <span>Autostart summary</span>
          <input
            type="checkbox"
            checked={summaryAutostart}
            onChange={(event) => setSummaryAutostart(event.target.checked)}
          />
        </label>

        <div className="inspector-block">
          <h4>Options</h4>
          <label className="toggle-row">
            <span>Sections</span>
            <input
              type="checkbox"
              checked={summarySections}
              onChange={(event) => setSummarySections(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>Bullet Points</span>
            <input
              type="checkbox"
              checked={summaryBulletPoints}
              onChange={(event) => setSummaryBulletPoints(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>Action Items</span>
            <input
              type="checkbox"
              checked={summaryActionItems}
              onChange={(event) => setSummaryActionItems(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>Key Points Only</span>
            <input
              type="checkbox"
              checked={summaryKeyPointsOnly}
              onChange={(event) => setSummaryKeyPointsOnly(event.target.checked)}
            />
          </label>
        </div>

        <label>
          Language
          <select
            className="inspector-select"
            value={summaryLanguage}
            onChange={(event) => setSummaryLanguage(event.target.value as LanguageCode)}
          >
            {languageOptions.filter((option) => option.value !== "auto").map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <label>
          Custom summary prompt
          <textarea
            className="inspector-prompt"
            value={summaryCustomPrompt}
            onChange={(event) => setSummaryCustomPrompt(event.target.value)}
            placeholder="Optional: override summary instructions..."
          />
        </label>
      </div>
    );
  }

  function renderChatInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <h4>Prompts</h4>
        <div className="prompt-list">
          {settings?.prompts.templates.map((prompt) => (
            <button key={prompt.id} className="prompt-item" onClick={() => void onSendChat(prompt.body)}>
              {prompt.name}
            </button>
          ))}
        </div>
        <button
          className="secondary-button"
          onClick={() => {
            void onOpenStandaloneSettingsWindow("prompts");
          }}
        >
          Manage Prompts
        </button>

        <div className="inspector-block">
          <h4>Options</h4>
          <label className="toggle-row">
            <span>Include timestamps</span>
            <input
              type="checkbox"
              checked={chatIncludeTimestamps}
              onChange={(event) => setChatIncludeTimestamps(event.target.checked)}
            />
          </label>
          <label className="toggle-row">
            <span>Include speakers</span>
            <input
              type="checkbox"
              checked={chatIncludeSpeakers}
              onChange={(event) => setChatIncludeSpeakers(event.target.checked)}
            />
          </label>
        </div>
      </div>
    );
  }

  function renderMetadataInspector(): JSX.Element {
    return (
      <div className="inspector-body">
        <h4>Model & Language</h4>

        <div className="property-grid">
          <label>Model</label>
          <select
            value={settings?.transcription.model ?? "base"}
            onChange={(event) => void onChangeModel(event.target.value as SpeechModel)}
          >
            {modelOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>

          <label>Language</label>
          <select
            value={settings?.transcription.language ?? "auto"}
            onChange={(event) => void onChangeLanguage(event.target.value as LanguageCode)}
          >
            {languageOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </div>

        {activeArtifact ? (
          <>
            <div className="property-line"><span>Kind</span><strong>{activeArtifact.kind}</strong></div>
            <div className="property-line"><span>Audio Duration</span><strong>{formatShortDuration(transcriptSeconds)}</strong></div>
            <div className="property-line"><span>Created</span><strong>{formatDate(activeArtifact.created_at)}</strong></div>
            <div className="property-line"><span>Updated</span><strong>{formatDate(activeArtifact.updated_at)}</strong></div>
            <div className="property-line"><span>Characters</span><strong>{draftTranscript.length}</strong></div>
            <div className="property-line"><span>Words</span><strong>{transcriptWordCount}</strong></div>
          </>
        ) : null}

        <button className="primary-button" onClick={() => void onSaveArtifact()} disabled={isSavingArtifact}>
          {isSavingArtifact ? "Saving..." : "Save"}
        </button>
      </div>
    );
  }

  function renderInspector(): JSX.Element {
    if (!activeArtifact) {
      if (activeJobId) {
        return (
          <div className="inspector-body">
            <button
              className="secondary-button"
              onClick={() => void navigator.clipboard.writeText(activeJobPreviewText)}
              disabled={!activeJobPreviewText}
            >
              Copy
            </button>
            <div className="inspector-block">
              <h4>Transcribing</h4>
              <p className="muted">{progress?.message ?? "Running Whisper transcription..."}</p>
            </div>
          </div>
        );
      }
      return <div className="inspector-body muted">No transcript selected</div>;
    }

    if (inspectorMode === "info") return renderMetadataInspector();
    if (detailMode === "summary") return renderSummaryInspector();
    if (detailMode === "chat") return renderChatInspector();
    return renderDefaultInspector();
  }

  function renderDetail(): JSX.Element {
    return (
      <div className={rightSidebarOpen ? "detail-layout" : "detail-layout right-collapsed"}>
        <section className="detail-main" ref={detailMainRef}>
          <DetailToolbar
            leftSidebarOpen={leftSidebarOpen}
            rightSidebarOpen={rightSidebarOpen}
            detailMode={detailMode}
            title={activeArtifact ? activeArtifact.title : (activeJobTitle || "Transcribing")}
            hasArtifact={Boolean(activeArtifact)}
            hasActiveJob={Boolean(activeJobId)}
            transcriptionProgress={activeTranscriptionPercentage}
            onToggleSidebar={() => setLeftSidebarOpen((open) => !open)}
            onBack={() => setSection("history")}
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
          />

          <div className="detail-body">{renderDetailMain()}</div>

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
                Add Speaker...
              </button>
              <div className="segment-context-separator" />
              <p className="segment-context-title">Assign Known Speaker</p>
              {knownSpeakerLabels.length > 0 ? (
                knownSpeakerLabels.map((speakerLabel) => (
                  <button
                    key={speakerLabel}
                    type="button"
                    className="segment-context-item"
                    role="menuitem"
                    disabled={isAssigningSpeaker}
                    onClick={() => onAssignKnownSpeakerFromContextMenu(speakerLabel)}
                  >
                    {speakerLabel}
                  </button>
                ))
              ) : (
                <p className="segment-context-empty">No known speaker labels yet</p>
              )}
              <div className="segment-context-separator" />
              <button
                type="button"
                className="segment-context-item danger"
                role="menuitem"
                disabled={isAssigningSpeaker || !contextMenuSegment?.speakerLabel}
                onClick={onClearSpeakerFromContextMenu}
              >
                Clear Speaker
              </button>
            </div>
          ) : null}

          <AudioPlayer
            inputPath={detailAudioInputPath}
            onMetadataLoaded={(metadata) => {
              setAudioDurationSeconds(metadata.durationSeconds);
            }}
          />
        </section>

        {rightSidebarOpen ? (
          <aside className="detail-inspector">
            <DetailInspectorHeader
              inspectorMode={inspectorMode}
              onInspectorModeChange={setInspectorMode}
              onHideDetailsPanel={() => setRightSidebarOpen(false)}
            />
            {renderInspector()}
          </aside>
        ) : null}

      </div>
    );
  }

  function renderRealtime(): JSX.Element {
    const combinedText = realtimeFinalLines.join("\n");

    return (
      <div className="view-body">
        <div className="view-toolbar">
          <h2>Live Transcription</h2>
          <div className="toolbar-actions">
            <button className="secondary-button" onClick={() => void onStartRealtime()} disabled={!canStartRealtime}>
              Start
            </button>
            <button className="secondary-button" onClick={() => void onPauseRealtime()} disabled={realtimeState !== "running"}>
              Pause
            </button>
            <button className="secondary-button" onClick={() => void onResumeRealtime()} disabled={realtimeState !== "paused"}>
              Resume
            </button>
            <button className="primary-button" onClick={() => void onStopRealtime(true)} disabled={realtimeState === "idle" || isStoppingRealtime}>
              Stop & Save
            </button>
          </div>
        </div>

        <section className="panel-card">
          <div className="panel-head">
            <strong>Status</strong>
            <span className={`status-chip ${realtimeState}`}>{realtimeMessage}</span>
          </div>

          <div className="live-view">
            {combinedText || realtimePreview ? (
              <>
                {combinedText ? <pre>{combinedText}</pre> : null}
                {realtimePreview ? <p className="preview-line">{realtimePreview}</p> : null}
              </>
            ) : (
              <div className="center-empty compact">
                <h3>No realtime transcript yet</h3>
                <p>Start live mode to stream transcript output.</p>
              </div>
            )}
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsGeneral(): JSX.Element {
    if (!settings) {
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>General</h3>
            <p>Application-level defaults and update behavior.</p>
          </header>

          <div className="settings-row">
            <div>
              <strong>Enable auto update checks</strong>
              <small>Checks Tauri updater and release feed in the background.</small>
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
              <strong>Updates repository</strong>
              <small>GitHub repository used for update checks.</small>
            </div>
            <input
              value={settings.general.auto_update_repo}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  general: {
                    ...current.general,
                    auto_update_repo: event.target.value,
                  },
                }));
              }}
            />
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>Appearance</strong>
              <small>Choose app theme behavior.</small>
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
              <option value="system">System</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </div>

          <div className="settings-actions-row">
            <button className="secondary-button" onClick={() => void onRefreshUpdates()} disabled={checkingUpdates}>
              {checkingUpdates ? "Checking..." : "Check Updates"}
            </button>
            {updateInfo ? (
              <small>
                {updateInfo.has_update
                  ? `Update ${updateInfo.latest_version} available${updateSource === "native" ? " (in-app install)" : ""}`
                  : `Up to date (${updateInfo.current_version})`}
              </small>
            ) : null}
          </div>
          {updateInfo?.has_update && nativeUpdate ? (
            <button className="cta-link-button" onClick={() => void onInstallUpdate()} disabled={installingUpdate}>
              {installingUpdate
                ? `Installing${updateDownloadPercent !== null ? ` (${updateDownloadPercent}%)` : "..."}`
                : "Download & Install"}
            </button>
          ) : null}
          {updateInfo?.has_update && updateInfo.download_url ? (
            <a className="cta-link-button" href={updateInfo.download_url} target="_blank" rel="noreferrer">
              {nativeUpdate ? "Manual Download" : "Download Update"}
            </a>
          ) : null}
          {updateStatusMessage ? <small>{updateStatusMessage}</small> : null}
        </section>
      </div>
    );
  }

  function renderSettingsTranscription(): JSX.Element {
    if (!settings) {
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>Transcription Defaults</h3>
            <p>Used for every new transcription job.</p>
          </header>

          <div className="settings-row settings-row-block">
            <div>
              <strong>Transcription engine</strong>
              <small>This app uses Whisper.cpp for local transcription.</small>
            </div>
            <select
              value={settings.transcription.engine}
              disabled={transcriptionEngineOptions.length <= 1}
              onChange={(event) => {
                void onChangeTranscriptionEngine(event.target.value as TranscriptionEngine);
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
              <strong>Default model</strong>
            </div>
            <select
              value={settings.transcription.model}
              onChange={(event) => {
                void onChangeModel(event.target.value as SpeechModel);
              }}
            >
              {modelOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>Default language</strong>
            </div>
            <select
              value={settings.transcription.language}
              onChange={(event) => {
                void onChangeLanguage(event.target.value as LanguageCode);
              }}
            >
              {languageOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row">
            <div>
              <strong>Enable AI post-processing</strong>
              <small>Run optimize/summary prompts after transcription.</small>
            </div>
            <input
              type="checkbox"
              checked={settings.transcription.enable_ai_post_processing}
              onChange={(event) => {
                void onToggleAi(event.target.checked);
              }}
            />
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsWhisperCpp(): JSX.Element {
    if (!settings) {
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    const whisperOptions = settings.transcription.whisper_options ?? getDefaultWhisperOptions(platformIsAppleSilicon);

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>Whisper C++</h3>
            <p>Decoding controls used by `whisper-cli` (whisper.cpp).</p>
          </header>

          <div className="settings-row">
            <div>
              <strong>Translate transcript to English</strong>
              <small>Equivalent to `--translate`.</small>
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
              <strong>No context between windows</strong>
              <small>Equivalent to `--max-context 0`.</small>
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
              <strong>Split on word</strong>
              <small>Use word boundaries when producing segments (`--split-on-word`).</small>
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
              <strong>Speaker diarization (tinydiarize)</strong>
              <small>Enable whisper.cpp tiny diarization (`-tdrz`) to infer speaker turns.</small>
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
              <strong>Stereo diarization</strong>
              <small>Enable whisper.cpp stereo diarization (`-di`) for stereo channel separation.</small>
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
              <strong>Threads</strong>
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
              <strong>Processors</strong>
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
              <strong>Beam size</strong>
              <small>`--beam-size` (when &gt; 1, best-of is ignored).</small>
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
              <strong>Best of</strong>
              <small>`--best-of` (used when beam size is 1).</small>
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
              <strong>Temperature</strong>
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
              <strong>Entropy threshold</strong>
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
              <strong>Logprob threshold</strong>
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
              <strong>Word threshold</strong>
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
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    const whisperOptions = settings.transcription.whisper_options ?? getDefaultWhisperOptions(platformIsAppleSilicon);

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>WhisperKit</h3>
            <p>Apple-native inference and model execution controls.</p>
          </header>

          <div className="settings-row settings-row-block">
            <div>
              <strong>Chunking strategy</strong>
            </div>
            <select
              value={whisperOptions.chunking_strategy}
              onChange={(event) => {
                void onPatchWhisperOptions((current) => ({
                  ...current,
                  chunking_strategy: event.target.value as WhisperOptions["chunking_strategy"],
                }));
              }}
            >
              {chunkingOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row settings-row-block">
            <div>
              <strong>Concurrent workers</strong>
              <small>Maximum parallel worker count for decoding.</small>
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
                <strong>Audio encoder compute units</strong>
                <small>CoreML execution unit for the audio encoder (Apple Silicon only).</small>
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
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          {platformIsAppleSilicon && (
            <div className="settings-row settings-row-block">
              <div>
                <strong>Text decoder compute units</strong>
                <small>CoreML execution unit for the text decoder (Apple Silicon only).</small>
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
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
          )}

          <div className="settings-row">
            <div>
              <strong>Use prefill prompt</strong>
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
              <strong>Use prefill cache</strong>
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
              <strong>Without timestamps</strong>
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
              <strong>Word timestamps</strong>
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
              <strong>Prompt override</strong>
              <small>Optional prompt used by WhisperKit decoders.</small>
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
              placeholder="Optional decoder prompt"
            />
          </div>
        </section>
      </div>
    );
  }

  function renderSettingsLocalModels(): JSX.Element {
    if (!settings) {
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <div className="settings-card-head">
            <h3>Local Models</h3>
            <button className="secondary-button" onClick={() => void refreshProvisioningModels()}>
              Refresh
            </button>
          </div>

          <p className="muted">
            Models are downloaded in background and used by local transcription.
          </p>
          <p className="muted">
            Directory: <code>{provisioning.modelsDir || settings.transcription.models_dir}</code>
          </p>

          {runtimeHealth ? (
            <div className="settings-health-block">
              <h4>Transcription Runtime Health</h4>
              <div className="settings-health-rows">
                <div className="settings-health-row">
                  <span className="settings-health-label">Platform</span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.host_os}</code>
                    <code>{runtimeHealth.host_arch}</code>
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">Engine policy</span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.preferred_engine}</code>
                    {runtimeHealth.configured_engine === runtimeHealth.preferred_engine ? (
                      <span className="kind-chip">Aligned</span>
                    ) : (
                      <span className="missing-chip">Will auto-fix on start</span>
                    )}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">Whisper CLI</span>
                  <span className="settings-health-value-inline">
                    <code className="settings-health-value">{runtimeHealth.whisper_cli_resolved || runtimeHealth.whisper_cli_path}</code>
                    {runtimeHealth.whisper_cli_available ? (
                      <span className="kind-chip">Runnable</span>
                    ) : (
                      <span className="missing-chip">Unavailable</span>
                    )}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">Whisper Stream</span>
                  <span className="settings-health-value-inline">
                    <code className="settings-health-value">{runtimeHealth.whisper_stream_resolved || runtimeHealth.whisper_stream_path}</code>
                    {runtimeHealth.whisper_stream_available ? (
                      <span className="kind-chip">Runnable</span>
                    ) : (
                      <span className="missing-chip">Unavailable</span>
                    )}
                  </span>
                </div>
                <div className="settings-health-row">
                  <span className="settings-health-label">Active model</span>
                  <span className="settings-health-value-inline">
                    <code>{runtimeHealth.model_filename}</code>
                    {runtimeHealth.model_present ? (
                      <span className="kind-chip">Installed</span>
                    ) : (
                      <span className="missing-chip">Missing</span>
                    )}
                  </span>
                </div>
                {platformIsAppleSilicon && (
                  <div className="settings-health-row">
                    <span className="settings-health-label">CoreML encoder</span>
                    {runtimeHealth.coreml_encoder_present ? (
                      <span className="kind-chip">Installed</span>
                    ) : (
                      <span className="missing-chip">Missing</span>
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
                  <strong>{model.label}</strong>
                  <small>{model.model_file}</small>
                </div>
                <div className="model-row-actions">
                  <span className={model.installed ? "kind-chip" : "missing-chip"}>
                    {model.installed ? "Installed" : "Missing"}
                  </span>
                  {platformIsAppleSilicon && (
                    <span className={model.coreml_installed ? "kind-chip" : "missing-chip"}>
                      {model.coreml_installed ? "CoreML Ready" : "CoreML Missing"}
                    </span>
                  )}
                  <button
                    className="secondary-button"
                    disabled={provisioning.running || (model.installed && (!platformIsAppleSilicon || model.coreml_installed))}
                    onClick={() => void onDownloadModel(model.key)}
                  >
                    {model.installed && (!platformIsAppleSilicon || model.coreml_installed) ? "Installed" : "Download"}
                  </button>
                </div>
              </div>
            ))}
          </div>

          <div className="notice-actions">
            <button className="primary-button" onClick={() => void onProvisionModels()} disabled={provisioning.running}>
              Download Missing Models
            </button>
            <button className="secondary-button" onClick={() => void onCancelProvisioning()} disabled={!provisioning.running}>
              Cancel
            </button>
          </div>

          {provisioning.progress ? (
            <div className="inline-progress">
              <div style={{ width: `${provisioning.progress.percentage}%` }} />
            </div>
          ) : null}
          {provisioning.statusMessage ? <small className="muted">{provisioning.statusMessage}</small> : null}
        </section>
      </div>
    );
  }

  function renderSettingsAiServices(): JSX.Element {
    if (!settings) {
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    const foundationAvailable = platformIsAppleSilicon;
    const foundationEnabled = settings.ai.providers.foundation_apple.enabled;
    const foundationActive = settings.ai.active_provider === "foundation_apple";
    const geminiActive =
      settings.ai.active_provider === "gemini"
      && Boolean(settings.ai.active_remote_service_id)
      && settings.ai.remote_services.some(
        (service) =>
          service.id === settings.ai.active_remote_service_id
          && service.kind === "google",
      );
    const geminiConfigured = Boolean(settings.ai.providers.gemini.api_key?.trim());
    const remoteServices = settings.ai.remote_services ?? [];
    const googleService = remoteServices.find((service) => service.kind === "google");
    const hasGoogleService = Boolean(googleService);
    const showGeminiService = hasGoogleService;
    const showGeminiConfig = aiServiceConfigOpen === googleService?.id;
    const configuredKinds = new Set(remoteServices.map((service) => service.kind));

    const createRemoteServiceEntry = (kind: RemoteServiceKind): RemoteServiceConfig | null => {
      const catalog = serviceCatalog.find((item) => item.kind === kind);
      if (!catalog) return null;
      return {
        id: createRemoteServiceId(kind),
        kind,
        label: catalog.label,
        enabled: true,
        api_key: kind === "google" ? settings.ai.providers.gemini.api_key : null,
        model: kind === "google" ? settings.ai.providers.gemini.model : catalog.defaultModel,
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
        const existing = remoteServices.find((service) => service.kind === kind);
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
        const target = (current.remote_services ?? []).find((service) => service.id === id);
        const nextServices = (current.remote_services ?? []).filter((service) => service.id !== id);
        const removingActiveRemote = current.active_remote_service_id === id;
        const shouldDeactivateGemini =
          target?.kind === "google" && current.active_provider === "gemini";

        return {
          ...current,
          active_provider: shouldDeactivateGemini
            ? current.providers.foundation_apple.enabled && platformIsAppleSilicon
              ? "foundation_apple"
              : "none"
            : current.active_provider,
          active_remote_service_id: removingActiveRemote ? null : current.active_remote_service_id,
          remote_services: nextServices,
        };
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
          <h3>Services</h3>

          <div className="ai-services-notice">
            <p>
              When you use remote AI services, your transcript will be sent to the service&apos;s
              servers and won&apos;t stay only on your Mac.
            </p>
            <button
              className={aiServicesAcknowledged ? "secondary-button ai-notice-button acknowledged" : "secondary-button ai-notice-button"}
              onClick={() => setAiServicesAcknowledged(true)}
            >
              {aiServicesAcknowledged ? "Understood" : "I Understand"}
            </button>
          </div>

          <article className={foundationActive ? "ai-service-card active" : "ai-service-card"}>
            <div className="ai-service-row">
              <span className="ai-service-icon foundation"></span>
              <div className="ai-service-title">
                <strong>Foundation Model</strong>
                <small>{foundationAvailable ? "Apple" : "Requires Apple Silicon"}</small>
              </div>
              <div className="ai-service-actions">
                <label className="toggle-row compact">
                  <span>Enabled</span>
                  <input
                    type="checkbox"
                    checked={foundationEnabled}
                    disabled={!foundationAvailable}
                    onChange={(event) => {
                      const enabled = event.target.checked;
                      void patchAiSettings((current) => ({
                        ...current,
                        active_provider:
                          !enabled && current.active_provider === "foundation_apple"
                            ? current.providers.gemini.api_key
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
                  disabled={!foundationAvailable || !foundationEnabled || foundationActive}
                  onClick={activateFoundation}
                >
                  {foundationActive ? "Active" : "Use"}
                </button>
              </div>
            </div>
          </article>

          {showGeminiService ? (
            <article className={geminiActive ? "ai-service-card active" : "ai-service-card"}>
            <div className="ai-service-row">
              <span className="ai-service-icon gemini">
                <Sparkles size={13} />
              </span>
              <div className="ai-service-title">
                <strong>{settings.ai.providers.gemini.model}</strong>
                <small>{geminiConfigured ? "Configured to use" : "Configure to use"}</small>
              </div>
              <div className="ai-service-actions">
                <button
                  className="secondary-button"
                  onClick={() => setAiServiceConfigOpen(showGeminiConfig ? null : googleService?.id ?? null)}
                >
                  {showGeminiConfig ? "Done" : "Configure"}
                </button>
                <button
                  className="secondary-button"
                  disabled={!geminiConfigured || geminiActive}
                  onClick={activateGemini}
                >
                  {geminiActive ? "Active" : "Use"}
                </button>
              </div>
            </div>

            {showGeminiConfig ? (
              <div className="ai-service-config">
                <label>
                  API Key
                  <input
                    type="password"
                    placeholder="Inserisci API Key..."
                    value={settings.ai.providers.gemini.api_key ?? ""}
                    onChange={(event) => {
                      const value = event.target.value.trim();
                      void patchAiSettings((current) => ({
                        ...current,
                        providers: {
                          ...current.providers,
                          gemini: {
                            ...current.providers.gemini,
                            api_key: value.length > 0 ? event.target.value : null,
                          },
                        },
                        remote_services: (current.remote_services ?? []).map((service) => (
                          service.kind === "google"
                            ? {
                                ...service,
                                api_key: value.length > 0 ? event.target.value : null,
                              }
                            : service
                        )),
                      }));
                    }}
                  />
                </label>

                <div className="ai-service-config-row">
                  <label>
                    Gemini Model
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
                          remote_services: (current.remote_services ?? []).map((service) => (
                            service.kind === "google"
                              ? {
                                  ...service,
                                  model: event.target.value,
                                }
                              : service
                          )),
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
                      onClick={() => setGeminiModelFetchNonce((value) => value + 1)}
                    >
                      {loadingGeminiModels ? "Loading models..." : "Refresh models"}
                    </button>
                    <a
                      className="cta-link-button ai-service-link"
                      href="https://aistudio.google.com/apikey"
                      target="_blank"
                      rel="noreferrer"
                    >
                      Create a Google API Key
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
                            ? current.providers.foundation_apple.enabled && platformIsAppleSilicon
                              ? "foundation_apple"
                              : "none"
                            : current.active_provider;

                        return {
                          ...current,
                          active_provider: nextProvider,
                          active_remote_service_id:
                            current.active_remote_service_id === googleService?.id
                              ? null
                              : current.active_remote_service_id,
                          providers: {
                            ...current.providers,
                            gemini: {
                              ...current.providers.gemini,
                              api_key: null,
                            },
                          },
                          remote_services: (current.remote_services ?? []).filter(
                            (service) => service.kind !== "google",
                          ),
                        };
                      });
                      setAiServiceConfigOpen(null);
                    }}
                  >
                    Delete
                  </button>
                </div>
              </div>
            ) : null}
            </article>
          ) : null}

          {remoteServices
            .filter((service) => service.kind !== "google")
            .map((service) => {
            const catalog = serviceCatalog.find((item) => item.kind === service.kind);
            const ServiceIcon = catalog?.icon ?? Settings2;
            const isOpen = aiServiceConfigOpen === service.id;
            const isActiveRemote =
              settings.ai.active_remote_service_id === service.id
              && settings.ai.active_provider !== "foundation_apple";

            return (
              <article
                key={service.id}
                className={isActiveRemote ? "ai-service-card active" : "ai-service-card"}
              >
                <div className="ai-service-row">
                  <span className={`ai-service-icon ${catalog?.tone ?? "custom"}`}>
                    <ServiceIcon size={13} />
                  </span>
                  <div className="ai-service-title">
                    <strong>{service.label || formatProviderLabel(service.kind)}</strong>
                    <small>{service.enabled ? "Configured" : "Disabled"}</small>
                  </div>
                  <div className="ai-service-actions">
                    <label className="toggle-row compact">
                      <span>Enabled</span>
                      <input
                        type="checkbox"
                        checked={service.enabled}
                        onChange={(event) => {
                          const enabled = event.target.checked;
                          void patchAiSettings((current) => {
                            const nextServices = (current.remote_services ?? []).map((entry) => (
                              entry.id === service.id
                                ? {
                                    ...entry,
                                    enabled,
                                  }
                                : entry
                            ));
                            const disablingActive = !enabled && current.active_remote_service_id === service.id;
                            const nextProvider = disablingActive
                              ? current.providers.foundation_apple.enabled && platformIsAppleSilicon
                                ? "foundation_apple"
                                : "none"
                              : current.active_provider;

                            return {
                              ...current,
                              active_provider: nextProvider,
                              active_remote_service_id: disablingActive ? null : current.active_remote_service_id,
                              remote_services: nextServices,
                            };
                          });
                        }}
                      />
                    </label>
                    <button
                      className="secondary-button"
                      onClick={() => setAiServiceConfigOpen(isOpen ? null : service.id)}
                    >
                      {isOpen ? "Done" : "Configure"}
                    </button>
                    <button
                      className="secondary-button"
                      disabled={!service.enabled || isActiveRemote}
                      onClick={() => {
                        void patchAiSettings((current) => ({
                          ...current,
                          active_provider: service.kind === "google" ? "gemini" : "none",
                          active_remote_service_id: service.id,
                        }));
                      }}
                    >
                      {isActiveRemote ? "Active" : "Use"}
                    </button>
                  </div>
                </div>

                {isOpen ? (
                  <div className="ai-service-config">
                    <label>
                      API Key
                      <input
                        type="password"
                        value={service.api_key ?? ""}
                        onChange={(event) =>
                          patchRemoteService(service.id, (current) => ({
                            ...current,
                            api_key: event.target.value.trim().length > 0 ? event.target.value : null,
                          }))
                        }
                      />
                    </label>
                    <label>
                      Model
                      <input
                        value={service.model ?? ""}
                        placeholder="Optional model name"
                        onChange={(event) =>
                          patchRemoteService(service.id, (current) => ({
                            ...current,
                            model: event.target.value.trim().length > 0 ? event.target.value : null,
                          }))
                        }
                      />
                    </label>
                    <label>
                      Base URL
                      <input
                        value={service.base_url ?? ""}
                        placeholder="Optional API endpoint"
                        onChange={(event) =>
                          patchRemoteService(service.id, (current) => ({
                            ...current,
                            base_url:
                              event.target.value.trim().length > 0 ? event.target.value : null,
                          }))
                        }
                      />
                    </label>
                    <div className="ai-service-config-actions">
                      <button
                        className="secondary-button"
                        onClick={() => removeRemoteService(service.id)}
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                ) : null}
              </article>
            );
            })}

          <div className="ai-service-library">
            <strong>Add another service</strong>
            <div className="ai-service-grid">
              {serviceCatalog.map((service) => {
                const ServiceIcon = service.icon;
                const isGoogle = service.kind === "google";
                const alreadyAdded = isGoogle ? hasGoogleService : configuredKinds.has(service.kind);
                return (
                  <button
                    key={service.kind}
                    className={`ai-service-chip available ${service.tone}`}
                    onClick={() => addRemoteService(service.kind)}
                  >
                    <span className="ai-service-chip-main">
                      <ServiceIcon size={13} />
                      <span>{service.label}</span>
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
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    return (
      <div className="settings-prompts-layout">
        <aside className="prompt-sidebar">
          <div className="settings-card-head">
            <h3>Prompt Templates</h3>
            <button className="secondary-button" onClick={() => void onResetPrompts()}>
              Reset
            </button>
          </div>

          <div className="prompt-template-list">
            {settings.prompts.templates.map((template) => (
              <button
                key={template.id}
                className={template.id === activePromptId ? "prompt-template-row active" : "prompt-template-row"}
                onClick={() => setActivePromptId(template.id)}
              >
                <span>{template.name}</span>
                {template.builtin ? <small>Built-in</small> : <small>Custom</small>}
              </button>
            ))}
          </div>

          <div className="settings-card-block compact">
            <h4>Prompt Bindings</h4>
            <label>
              Optimize
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
              Summary
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
              FAQ
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
          </div>
        </aside>

        <section className="settings-card-block prompt-editor-pane">
          {!promptDraft ? (
            <div className="settings-placeholder">Select a prompt template</div>
          ) : (
            <>
              <div className="settings-card-head">
                <h3>Edit Prompt</h3>
                <button className="primary-button" onClick={() => void onSavePromptTemplate()}>
                  <Save size={14} />
                  Save Prompt
                </button>
              </div>

              <label>
                Title
                <input
                  value={promptDraft.name}
                  onChange={(event) => setPromptDraft((current) => current ? { ...current, name: event.target.value } : current)}
                />
              </label>

              <label>
                Prompt Body
                <textarea
                  className="settings-textarea"
                  value={promptDraft.body}
                  onChange={(event) => setPromptDraft((current) => current ? { ...current, body: event.target.value } : current)}
                />
              </label>

              <div className="prompt-test-row">
                <label>
                  Test task
                  <select
                    value={promptBindingTask}
                    onChange={(event) => setPromptBindingTask(event.target.value as PromptTask)}
                  >
                    {promptTaskOptions.map((task) => (
                      <option key={task.value} value={task.value}>
                        {task.label}
                      </option>
                    ))}
                  </select>
                </label>
                <button className="secondary-button" onClick={() => void onRunPromptTest()} disabled={promptTest.running}>
                  {promptTest.running ? "Testing..." : "Run Test"}
                </button>
              </div>

              <label>
                Test input
                <textarea
                  className="settings-textarea small"
                  value={promptTest.input}
                  onChange={(event) => setPromptTest((current) => ({ ...current, input: event.target.value }))}
                />
              </label>

              <label>
                Output
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
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <h3>Advanced</h3>

          <label>
            Whisper CLI path
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
            Whisper Stream path
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
            FFmpeg path
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
            Models directory
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
            <button className="secondary-button" onClick={() => void refreshSettingsFromDisk()}>
              Reload from disk
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
    return <div className="settings-placeholder">Settings unavailable.</div>;
  }

  function renderSettings(): JSX.Element {
    return (
      <div className="settings-layout">
        <aside className="settings-sidebar">
          <label className="settings-search">
            <Search size={14} />
            <input
              placeholder="Search settings..."
              value={settingsQuery}
              onChange={(event) => setSettingsQuery(event.target.value)}
            />
          </label>
          <div className="settings-nav-list">
            {visibleSettingsPaneGroups.map(({ group, panes }) => (
              <section key={group} className="settings-nav-group">
                <p className="settings-nav-group-title">{group}</p>
                <div className="settings-nav-group-items">
                  {panes.map((pane) => {
                    const PaneIcon = pane.icon;
                    return (
                      <button
                        key={pane.key}
                        className={settingsPane === pane.key ? "settings-nav-item active" : "settings-nav-item"}
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
              <div className="settings-nav-empty">No section matches your search.</div>
            ) : null}
          </div>
        </aside>

        <section className="settings-content">
          {renderSettingsPane(settingsPane)}
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

  if (standaloneSettingsWindow) {
    return (
      <main className="settings-window-shell">
        <section className="settings-window-frame">
          <header className="settings-window-header" data-tauri-drag-region />
          {renderSettings()}
          {error ? (
            <div className="error-banner settings-window-error">
              <p>{error}</p>
              <button className="error-close" onClick={() => setError(null)} title="Dismiss">
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

  return (
    <main className="app-shell">
      <section className={leftSidebarOpen ? "window-frame" : "window-frame left-collapsed"}>
        {leftSidebarOpen ? (
          <aside className="left-sidebar">
            <div className="sidebar-section">
              <button className={section === "home" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("home")}>
                <House size={16} />
                Home
              </button>
              <button className={section === "queue" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("queue")}>
                <ListChecks size={16} />
                Queue
              </button>
              <button className={section === "history" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("history")}>
                <HistoryIcon size={16} />
                History
              </button>
            </div>

            {openArtifacts.length > 0 ? (
              <div className="sidebar-section">
                <h4>Open</h4>
                {openArtifacts.map((artifact) => (
                  <div key={artifact.id} className="sidebar-open-row" title={artifact.title}>
                    <button
                      className={
                        activeArtifactId === artifact.id
                          ? "sidebar-item sidebar-open-item active"
                          : "sidebar-item sidebar-open-item"
                      }
                      onClick={() => {
                        hydrateDetail(artifact);
                        setSection("detail");
                      }}
                    >
                      <FileAudio size={16} />
                      <span className="sidebar-item-label">{artifact.title}</span>
                    </button>
                    <button
                      className="sidebar-open-close"
                      onClick={(event) => {
                        event.stopPropagation();
                        setOpenArtifacts((prev) => prev.filter((a) => a.id !== artifact.id));
                        if (activeArtifactId === artifact.id) {
                            setActiveArtifactId(null);
                            setSection("home");
                        }
                      }}
                      title="Close opened transcription"
                      aria-label="Close opened transcription"
                    >
                      <X size={12} />
                    </button>
                  </div>
                ))}
              </div>
            ) : null}

            <div className="sidebar-section">
              <button
                className={section === "deleted_history" ? "sidebar-item active" : "sidebar-item"}
                onClick={() => setSection("deleted_history")}
              >
                <Trash2 size={16} />
                Recently Deleted
              </button>
            </div>

            <div className="sidebar-footer">
              <button className="sidebar-item" onClick={() => void onOpenStandaloneSettingsWindow("transcription")}>
                <Settings2 size={16} />
                Settings
              </button>
            </div>
          </aside>
        ) : null}

        <section className="main-area">
          {section !== "detail" ? (
            <header className="main-topbar" data-tauri-drag-region>
              <div className="topbar-title" data-tauri-drag-region>
                <button
                  className={`icon-button sidebar-toggle-btn sidebar-toggle-left ${leftSidebarOpen ? "is-open" : ""}`}
                  onClick={() => setLeftSidebarOpen(!leftSidebarOpen)}
                  title={leftSidebarOpen ? "Hide sidebar" : "Show sidebar"}
                >
                  <PanelLeftClose className="icon-close" size={16} />
                  <PanelLeftOpen className="icon-open" size={16} />
                </button>
                {section === "home" ? null : (
                  <h1 data-tauri-drag-region>
                    {section === "queue"
                      ? "Queue"
                      : section === "realtime"
                        ? "Live"
                        : section === "deleted_history"
                            ? "Recently Deleted"
                            : "Transcriptions"}
                  </h1>
                )}
              </div>

              {section === "home" || section === "queue" || section === "realtime" ? (
                <div className="topbar-controls">
                  <label className="select-chip">
                    <span className="chip-label">
                      <Mic size={12} />
                    </span>
                    <select
                      value={settings?.transcription.model ?? "base"}
                      onChange={(event) => void onChangeModel(event.target.value as SpeechModel)}
                    >
                      {modelOptions.map((option) => (
                        <option key={option.value} value={option.value}>{option.label}</option>
                      ))}
                    </select>
                  </label>

                  <label className="select-chip">
                    <span className="chip-label">
                      <Languages size={12} />
                    </span>
                    <select
                      value={settings?.transcription.language ?? "auto"}
                      onChange={(event) => void onChangeLanguage(event.target.value as LanguageCode)}
                    >
                      {languageOptions.map((option) => (
                        <option key={option.value} value={option.value}>{option.label}</option>
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
                      onChange={(event) => setHistoryKind(event.target.value as "all" | ArtifactKind)}
                    >
                      <option value="all">All</option>
                      <option value="file">Files</option>
                      <option value="realtime">Live</option>
                    </select>
                  </label>
                  <label className="search-chip history-top-search">
                    <Search size={14} />
                    <input
                      placeholder="Search history..."
                      value={search}
                      onChange={(event) => setSearch(event.target.value)}
                    />
                  </label>
                </div>
              ) : null}

              {section === "deleted_history" ? (
                <div className="topbar-controls deleted-topbar-controls">
                  <button className="secondary-button history-action-button" onClick={() => void onEmptyTrash()}>
                    <Trash2 size={14} />
                    Empty Trash
                  </button>
                  <label className="search-chip history-top-search">
                    <Search size={14} />
                    <input
                      placeholder="Search history..."
                      value={deletedSearch}
                      onChange={(event) => setDeletedSearch(event.target.value)}
                    />
                  </label>
                </div>
              ) : null}
            </header>
          ) : null}

          <div className="main-content">{renderContent()}</div>

          {error ? (
            <div className="error-banner">
              <p>{error}</p>
              <button className="error-close" onClick={() => setError(null)} title="Dismiss">
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
            aria-label="Rename transcription"
          >
            <header className="rename-sheet-head">
              <h3>Rename transcription</h3>
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
              placeholder="Transcription title"
            />
            <div className="rename-sheet-actions">
              <button className="secondary-button" onClick={closeRenameDialog} disabled={isRenamingArtifact}>
                Cancel
              </button>
              <button
                className="primary-button"
                onClick={() => void confirmRenameArtifact()}
                disabled={isRenamingArtifact || renameDraft.trim().length === 0}
              >
                {isRenamingArtifact ? "Saving..." : "Save"}
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
        segments={detailSegments}
        onClose={() => setShowExportSheet(false)}
        onExport={onExport}
      />
    </main>
  );
}
