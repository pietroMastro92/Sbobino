import { type CSSProperties, useEffect, useMemo, useRef, useState } from "react";
import { confirm as confirmDialog, open, save } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  AudioLines,
  Clock3,
  ChevronDown,
  FileAudio,
  FileText,
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
  Search,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import {
  cancelTranscription,
  chatArtifact,
  checkUpdates,
  deleteArtifacts,
  emptyDeletedArtifacts,
  exportArtifact,
  fetchRuntimeHealth,
  fetchSettingsSnapshot,
  getArtifact,
  hardDeleteArtifacts,
  listDeletedArtifacts,
  listRecentArtifacts,
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
  testPromptTemplate,
  updateArtifact,
} from "./lib/tauri";
import { useAppStore } from "./state/useAppStore";
import type {
  AppSettings,
  ArtifactKind,
  JobProgress,
  LanguageCode,
  PromptTask,
  PromptTemplate,
  ProvisioningProgressEvent,
  ProvisioningModelCatalogEntry,
  RealtimeDelta,
  RuntimeHealth,
  SpeechModel,
  TranscriptArtifact,
  UpdateCheckResponse,
  WhisperOptions,
} from "./types";
import { AudioPlayer } from "./components/AudioPlayer";
import { ExportSheet, type ExportRequest } from "./components/ExportSheet";
import { ModelManagerSheet } from "./components/ModelManagerSheet";

type Section =
  | "home"
  | "queue"
  | "history"
  | "deleted_history"
  | "detail"
  | "realtime"
  | "settings";
type DetailMode = "transcript" | "segments" | "summary" | "chat";
type InspectorMode = "details" | "info";
type SettingsPane =
  | "general"
  | "transcription"
  | "local_models"
  | "ai_services"
  | "prompts"
  | "advanced";
type ChatMessage = { role: "user" | "assistant"; text: string };

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

const promptTaskOptions: Array<{ value: PromptTask; label: string }> = [
  { value: "optimize", label: "Optimize transcript" },
  { value: "summary", label: "Summary" },
  { value: "faq", label: "FAQ" },
];

const defaultPromptTestInput = "This is an example of some transcribed text.";
const defaultWhisperOptions: WhisperOptions = {
  translate_to_english: false,
  no_context: false,
  split_on_word: false,
  temperature: 0,
  entropy_threshold: 2.5,
  logprob_threshold: -1,
  word_threshold: 0.01,
  best_of: 5,
  beam_size: 1,
  threads: 4,
  processors: 1,
};

const settingsPaneDefinitions: Array<{
  key: SettingsPane;
  label: string;
  description: string;
}> = [
  { key: "transcription", label: "Transcription", description: "Whisper decoding and defaults" },
  { key: "local_models", label: "Local Models", description: "Model downloads and health" },
  { key: "advanced", label: "Advanced", description: "Runtime binary paths" },
  { key: "general", label: "General", description: "Updates and app basics" },
  { key: "ai_services", label: "AI Services", description: "Provider configuration" },
  { key: "prompts", label: "Prompts", description: "Template management" },
];

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

function buildSegments(text: string): Array<{ time: string; line: string }> {
  const lines = text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

  return lines.map((line, index) => {
    const seconds = index * 4;
    const mm = String(Math.floor(seconds / 60)).padStart(2, "0");
    const ss = String(seconds % 60).padStart(2, "0");
    return { time: `${mm}:${ss}`, line };
  });
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

function normalizeSettings(settings: AppSettings): AppSettings {
  const normalizedWhisperOptions = sanitizeWhisperOptions({
    ...defaultWhisperOptions,
    ...settings.transcription.whisper_options,
  });

  const normalized: AppSettings = {
    ...settings,
    general: {
      ...settings.general,
    },
    transcription: {
      ...settings.transcription,
      whisper_options: normalizedWhisperOptions,
    },
    ai: {
      ...settings.ai,
      providers: {
        ...settings.ai.providers,
        foundation_apple: {
          ...settings.ai.providers.foundation_apple,
        },
        gemini: {
          ...settings.ai.providers.gemini,
        },
      },
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
  normalized.ai_post_processing = normalized.transcription.enable_ai_post_processing;
  normalized.whisper_cli_path = normalized.transcription.whisper_cli_path;
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
    temperature: clamp(options.temperature, 0, 1),
    entropy_threshold: clamp(options.entropy_threshold, 0, 10),
    logprob_threshold: clamp(options.logprob_threshold, -10, 0),
    word_threshold: clamp(options.word_threshold, 0, 1),
    best_of: Math.round(clamp(options.best_of, 1, 20)),
    beam_size: Math.round(clamp(options.beam_size, 1, 20)),
    threads: Math.round(clamp(options.threads, 1, 32)),
    processors: Math.round(clamp(options.processors, 1, 16)),
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
  onShowSidebar: () => void;
  onBack: () => void;
  onSelectMode: (mode: "transcript" | "summary" | "chat") => void;
  onOpenExport: () => void;
  onShowDetailsPanel: () => void;
  onCancel: () => void;
};

function DetailToolbar({
  leftSidebarOpen,
  rightSidebarOpen,
  detailMode,
  title,
  hasArtifact,
  hasActiveJob,
  transcriptionProgress,
  onShowSidebar,
  onBack,
  onSelectMode,
  onOpenExport,
  onShowDetailsPanel,
  onCancel,
}: DetailToolbarProps): JSX.Element {
  return (
    <header className="detail-toolbar">
      <div className="detail-toolbar-left">
        {!leftSidebarOpen ? (
          <button className="icon-button" onClick={onShowSidebar} title="Show sidebar">
            <PanelLeftOpen size={16} />
          </button>
        ) : null}
        <button className="icon-button" onClick={onBack} title="Back to history">
          <ArrowLeft size={16} />
        </button>
        <strong className="detail-title">{title}</strong>
      </div>

      <DetailCenterModeControl
        detailMode={detailMode}
        summaryDisabled={!hasArtifact}
        chatDisabled={!hasArtifact}
        onSelect={onSelectMode}
      />

      <div className="detail-toolbar-right">
        {hasArtifact ? (
          <button className="secondary-button export-toolbar-button" onClick={onOpenExport}>
            Export
            <ChevronDown size={14} />
          </button>
        ) : null}
        {!rightSidebarOpen ? (
          <button className="icon-button" onClick={onShowDetailsPanel} title="Show details panel">
            <PanelRightOpen size={16} />
          </button>
        ) : null}
        {!hasArtifact && hasActiveJob ? (
          <>
            <span className="transcribing-pill">
              <ProgressRing percentage={transcriptionProgress} size={16} />
              <span>Transcribing</span>
            </span>
            <button className="secondary-button" onClick={onCancel}>
              Cancel
            </button>
          </>
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
      <button className="icon-button" onClick={onHideDetailsPanel} title="Hide details panel">
        <PanelRightClose size={16} />
      </button>
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

export function App() {
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

  const [isStarting, setIsStarting] = useState(false);
  const [isSavingArtifact, setIsSavingArtifact] = useState(false);
  const [activeArtifact, setActiveArtifact] = useState<TranscriptArtifact | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [draftTranscript, setDraftTranscript] = useState("");
  const [draftSummary, setDraftSummary] = useState("");
  const [draftFaqs, setDraftFaqs] = useState("");
  const [showExportSheet, setShowExportSheet] = useState(false);
  const [renameTarget, setRenameTarget] = useState<TranscriptArtifact | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [isRenamingArtifact, setIsRenamingArtifact] = useState(false);

  const [chatInput, setChatInput] = useState("");
  const [chatHistory, setChatHistory] = useState<ChatMessage[]>([]);
  const [isAskingChat, setIsAskingChat] = useState(false);
  const [activeJobPreviewText, setActiveJobPreviewText] = useState("");
  const [activeJobTitle, setActiveJobTitle] = useState("");

  const [queueItems, setQueueItems] = useState<JobProgress[]>([]);
  const [modelCatalog, setModelCatalog] = useState<ProvisioningModelCatalogEntry[]>([]);
  const [runtimeHealth, setRuntimeHealth] = useState<RuntimeHealth | null>(null);

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
  const [checkingUpdates, setCheckingUpdates] = useState(false);

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
  const settingsSaveSequenceRef = useRef(0);
  const isMacOS = useMemo(() => navigator.userAgent.toLowerCase().includes("mac"), []);

  useEffect(() => {
    activeJobIdRef.current = activeJobId;
  }, [activeJobId]);

  useEffect(() => {
    window.localStorage.setItem("sbobino.layout.leftSidebarOpen", String(leftSidebarOpen));
  }, [leftSidebarOpen]);

  useEffect(() => {
    window.localStorage.setItem("sbobino.layout.rightSidebarOpen", String(rightSidebarOpen));
  }, [rightSidebarOpen]);

  useEffect(() => {
    if (section === "settings" && settingsPane === "local_models") {
      void refreshRuntimeHealth();
    }
  }, [section, settingsPane]);

  useEffect(() => {
    if (section !== "deleted_history") {
      return;
    }

    void (async () => {
      try {
        const deletedArtifactsSnapshot = await listDeletedArtifacts({ limit: 200 });
        setDeletedArtifacts(deletedArtifactsSnapshot);
      } catch (deletedError) {
        setError(`Could not load Recently Deleted: ${String(deletedError)}`);
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
        } catch {
          // keep app booting even if health probe fails
        }

        if (initialSettings.general.auto_update_enabled) {
          setCheckingUpdates(true);
          try {
            const update = await checkUpdates();
            if (!disposed) {
              setUpdateInfo(update);
            }
          } catch {
            // ignore initial update check errors
          } finally {
            if (!disposed) {
              setCheckingUpdates(false);
            }
          }
        }
      } catch (bootstrapError) {
        setError(`Bootstrap failed: ${String(bootstrapError)}`);
      }
    })();

    return () => {
      disposed = true;
    };
  }, [setArtifacts, setError, setSettings]);

  useEffect(() => {
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
      unsubProgress = await subscribeJobProgress((event) => {
        setQueueItems((previous) => pushOrReplaceQueueItem(previous, event));
        if (event.job_id === activeJobIdRef.current) {
          setProgress(event);
        }
        if (event.stage === "cancelled" || event.stage === "failed") {
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobPreviewText("");
          setActiveJobTitle("");
          activeJobDeltaSequenceRef.current = -1;
        }
      });

      unsubCompleted = await subscribeJobCompleted((artifact) => {
        prependArtifact(artifact);
        setQueueItems((previous) => previous.filter((entry) => entry.job_id !== artifact.job_id));
        setActiveJobPreviewText("");
        setActiveJobTitle("");
        activeJobDeltaSequenceRef.current = -1;

        if (artifact.job_id === activeJobIdRef.current) {
          clearActiveJob();
          activeJobIdRef.current = null;
          hydrateDetail(artifact);
          setSection("detail");
          setError(null);
        }
      });

      unsubFailed = await subscribeJobFailed((payload) => {
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
          clearActiveJob();
          activeJobIdRef.current = null;
          setActiveJobPreviewText("");
          setActiveJobTitle("");
          activeJobDeltaSequenceRef.current = -1;
          setError(payload.message);
        }
      });

      unsubTranscriptionDelta = await subscribeTranscriptionDelta((delta) => {
        if (delta.job_id !== activeJobIdRef.current) {
          return;
        }
        if (delta.sequence <= activeJobDeltaSequenceRef.current) {
          return;
        }
        activeJobDeltaSequenceRef.current = delta.sequence;

        setActiveJobPreviewText((previous) => {
          const nextLine = delta.text.trim();
          if (!nextLine) return previous;
          if (!previous) return nextLine;
          if (previous.endsWith(nextLine)) return previous;
          return `${previous}\n${nextLine}`;
        });
      });

      unsubRealtimeDelta = await subscribeRealtimeDelta((delta: RealtimeDelta) => {
        if (delta.kind === "append_final") {
          setRealtimeFinalLines((previous) => [...previous, delta.text]);
          setRealtimePreview("");
        }

        if (delta.kind === "update_preview") {
          setRealtimePreview(delta.text);
        }
      });

      unsubRealtimeStatus = await subscribeRealtimeStatus((event) => {
        setRealtimeMessage(event.message);
        if (event.state === "running") {
          setRealtimeState("running");
        } else if (event.state === "paused") {
          setRealtimeState("paused");
        } else {
          setRealtimeState("idle");
        }
      });

      unsubRealtimeSaved = await subscribeRealtimeSaved((artifact) => {
        prependArtifact(artifact);
      });

      unsubProvisioningProgress = await subscribeProvisioningProgress((event) => {
        setProvisioning((previous) => ({
          ...previous,
          running: true,
          progress: event,
          statusMessage: `Downloading ${event.asset} (${event.current}/${event.total})`,
        }));
      });

      unsubProvisioningStatus = await subscribeProvisioningStatus((event) => {
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
    })();

    return () => {
      unsubProgress?.();
      unsubCompleted?.();
      unsubFailed?.();
      unsubTranscriptionDelta?.();
      unsubRealtimeDelta?.();
      unsubRealtimeStatus?.();
      unsubRealtimeSaved?.();
      unsubProvisioningProgress?.();
      unsubProvisioningStatus?.();
    };
  }, [clearActiveJob, prependArtifact, setError, setProgress]);

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
        artifact.title.toLowerCase().includes(needle) ||
        artifact.input_path.toLowerCase().includes(needle) ||
        artifact.optimized_transcript.toLowerCase().includes(needle) ||
        artifact.raw_transcript.toLowerCase().includes(needle)
      );
    });
  }, [artifacts, historyKind, search]);

  const filteredDeletedArtifacts = useMemo(() => {
    const needle = deletedSearch.trim().toLowerCase();
    return deletedArtifacts.filter((artifact) => {
      if (!needle) return true;
      return (
        artifact.title.toLowerCase().includes(needle) ||
        artifact.input_path.toLowerCase().includes(needle) ||
        artifact.optimized_transcript.toLowerCase().includes(needle) ||
        artifact.raw_transcript.toLowerCase().includes(needle)
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

  const detailSegments = useMemo(
    () => buildSegments(draftTranscript || activeArtifact?.raw_transcript || ""),
    [activeArtifact?.raw_transcript, draftTranscript],
  );

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
    if (detailSegments.length > 0) {
      return detailSegments.length * 4;
    }
    return 0;
  }, [audioDurationSeconds, detailSegments.length]);

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
  }, [settingsQuery]);

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
      setError(`Could not save settings: ${String(settingsError)}`);
    }
  }

  async function patchSettings(mutator: (current: AppSettings) => AppSettings): Promise<void> {
    if (!settings) return;
    const previous = normalizeSettings(settings);
    const next = normalizeSettings(mutator(normalizeSettings(settings)));
    await persistSettings(next, previous);
  }

  async function refreshSettingsFromDisk(): Promise<void> {
    try {
      const next = await fetchSettingsSnapshot();
      setSettings(normalizeSettings(next));
    } catch (error) {
      setError(`Could not reload settings: ${String(error)}`);
    }
  }

  async function refreshProvisioningStatus(): Promise<void> {
    try {
      const status = await provisioningStatus();
      setProvisioningState(status);
    } catch (statusError) {
      setError(`Could not read provisioning status: ${String(statusError)}`);
    }
  }

  async function refreshRuntimeHealth(): Promise<void> {
    try {
      const health = await fetchRuntimeHealth();
      setRuntimeHealth(health);
    } catch (healthError) {
      setError(`Could not read transcription runtime health: ${String(healthError)}`);
    }
  }

  async function refreshProvisioningModels(): Promise<void> {
    try {
      const [models, health] = await Promise.all([provisioningModels(), fetchRuntimeHealth()]);
      setModelCatalog(models);
      setRuntimeHealth(health);
    } catch (modelsError) {
      setError(`Could not read models catalog: ${String(modelsError)}`);
    }
  }

  async function refreshActiveArtifacts(): Promise<void> {
    const activeArtifactsSnapshot = await listRecentArtifacts();
    setArtifacts(activeArtifactsSnapshot);
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
          mutator(current.transcription.whisper_options ?? defaultWhisperOptions),
        ),
      },
    }));
  }

  async function onStartTranscription(): Promise<void> {
    if (!settings || !selectedFile) return;

    setIsStarting(true);
    setError(null);
    try {
      const { job_id } = await startTranscription({
        input_path: selectedFile,
        language: settings.transcription.language,
        model: settings.transcription.model,
        enable_ai: settings.transcription.enable_ai_post_processing,
        whisper_options: sanitizeWhisperOptions(
          settings.transcription.whisper_options ?? defaultWhisperOptions,
        ),
      });

      setJobStarted(job_id);
      activeJobIdRef.current = job_id;
      setActiveJobTitle(fileLabel(selectedFile));
      setActiveJobPreviewText("");
      activeJobDeltaSequenceRef.current = -1;
      setSection("detail");
      setDetailMode("transcript");
      setInspectorMode("details");
    } catch (startError) {
      setError(`Failed to start transcription: ${String(startError)}`);
    } finally {
      setIsStarting(false);
    }
  }

  async function onCancel(): Promise<void> {
    if (!activeJobId) return;

    try {
      await cancelTranscription(activeJobId);
    } catch (cancelError) {
      setError(`Failed to cancel transcription: ${String(cancelError)}`);
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
      setError(`Failed to open transcript: ${String(artifactError)}`);
    }
  }

  function onFocusQueueJob(item: JobProgress): void {
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
      setError(`Failed to save changes: ${String(saveError)}`);
    } finally {
      setIsSavingArtifact(false);
    }
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
      setError(`Rename failed: ${String(renameError)}`);
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
      setError(`Delete failed: ${String(deleteError)}`);
    }
  }

  async function onDeleteArtifact(artifact: TranscriptArtifact): Promise<void> {
    await onDeleteArtifactsWithConsent([artifact.id]);
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
      setError(`Restore failed: ${String(restoreError)}`);
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
      setError(`Permanent delete failed: ${String(deleteError)}`);
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
      setError(`Empty trash failed: ${String(emptyError)}`);
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
        setError(`Could not sync changes before export: ${String(syncError)}`);
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
      setError(`Export failed: ${String(exportError)}`);
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
      setError(`Realtime start failed: ${String(startError)}`);
    }
  }

  async function onPauseRealtime(): Promise<void> {
    try {
      await pauseRealtime();
    } catch (pauseError) {
      setError(`Realtime pause failed: ${String(pauseError)}`);
    }
  }

  async function onResumeRealtime(): Promise<void> {
    try {
      await resumeRealtime();
    } catch (resumeError) {
      setError(`Realtime resume failed: ${String(resumeError)}`);
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
      setError(`Realtime stop failed: ${String(stopError)}`);
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
      const answer = await chatArtifact({ id: activeArtifact.id, prompt });
      setDraftSummary(answer);
      setDetailMode("summary");
      setError(null);
    } catch (summaryError) {
      setError(`Summary failed: ${String(summaryError)}`);
    } finally {
      setIsGeneratingSummary(false);
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
      setChatHistory((previous) => [
        ...previous,
        {
          role: "assistant",
          text: "Chat unavailable. Configure AI provider in Settings > AI Services.",
        },
      ]);
      setError(String(chatError));
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
      setError(`Provisioning failed: ${String(provisionError)}`);
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
      setError(`Model download failed: ${String(downloadError)}`);
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
      setError(`Provisioning cancel failed: ${String(cancelError)}`);
    }
  }

  async function onRefreshUpdates(): Promise<void> {
    setCheckingUpdates(true);
    try {
      const update = await checkUpdates();
      setUpdateInfo(update);
    } catch (updateError) {
      setError(`Update check failed: ${String(updateError)}`);
    } finally {
      setCheckingUpdates(false);
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
        output: `Prompt test failed: ${String(error)}`,
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
      setError(`Could not reset prompts: ${String(error)}`);
    }
  }

  function renderHome(): JSX.Element {
    return (
      <div className="view-body home-view">
        <section className="file-picker-card">
          <div className="file-chip">
            <AudioLines size={15} />
            <span>{selectedFile ? fileLabel(selectedFile) : "No file selected"}</span>
          </div>
          <button className="secondary-button" onClick={() => void onPickFile()}>
            <Upload size={15} />
            Open File
          </button>
        </section>

        <div className="quick-actions-grid">
          <button
            className="quick-action"
            onClick={() => void onStartTranscription()}
            disabled={!canStartFileTranscription}
          >
            <FileAudio size={16} />
            {isStarting ? "Starting..." : "Start Transcription"}
          </button>
          <button className="quick-action" onClick={() => void onStartRealtime()} disabled={!canStartRealtime}>
            <Mic size={16} />
            Start Live
          </button>
          <button
            className="quick-action"
            onClick={() => void onStopRealtime(true)}
            disabled={realtimeState === "idle" || isStoppingRealtime}
          >
            <Radio size={16} />
            {isStoppingRealtime ? "Stopping..." : "Stop Live & Save"}
          </button>
          <button className="quick-action" onClick={() => setSection("queue")}>
            <ListChecks size={16} />
            Queue
          </button>
          <button className="quick-action" onClick={() => setSection("history")}>
            <HistoryIcon size={16} />
            History
          </button>
          <button
            className="quick-action quick-action-models"
            onClick={() => {
              setSection("settings");
              setSettingsPane("local_models");
            }}
          >
            <Settings2 size={16} />
            Manage Models
          </button>
        </div>

        <section className="panel-card">
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
                  <div className="history-list">
                    {group.items.map((artifact) => (
                      <article key={artifact.id} className="history-item home-history-item">
                        <button className="home-history-main" onClick={() => void onOpenArtifact(artifact.id)}>
                          <span className="history-audio-dot">
                            <AudioLines size={12} />
                          </span>
                          <div className="home-history-copy">
                            <div className="home-history-head">
                              <strong>{artifact.title}</strong>
                              <span className="history-inline-time">{formatHomeTime(artifact.updated_at)}</span>
                            </div>
                            <p className="history-preview">
                              {previewSnippet(artifact.optimized_transcript || artifact.raw_transcript, 210)}
                            </p>
                          </div>
                        </button>
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
                <div className="history-list">
                  {group.items.map((artifact) => (
                    <article key={artifact.id} className="history-item">
                      <button className="history-main history-main-rich" onClick={() => void onOpenArtifact(artifact.id)}>
                        <span className="history-audio-dot">
                          <AudioLines size={12} />
                        </span>
                        <div className="history-main-copy">
                          <div className="home-history-head">
                            <strong>{artifact.title}</strong>
                            <span className="history-inline-time">{formatHomeTime(artifact.updated_at)}</span>
                          </div>
                          <p className="history-preview">
                            {previewSnippet(artifact.optimized_transcript || artifact.raw_transcript, 220)}
                          </p>
                        </div>
                      </button>
                      <div className="history-actions">
                        <span className="kind-chip">{artifact.kind === "realtime" ? "Live" : "File"}</span>
                        <button className="secondary-button history-action-button" onClick={() => void onRenameArtifact(artifact)}>
                          <Pencil size={14} />
                          Rename
                        </button>
                        <button className="secondary-button history-action-button history-action-danger" onClick={() => void onDeleteArtifact(artifact)}>
                          <Trash2 size={14} />
                          Move to Trash
                        </button>
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
                    <strong>{artifact.title}</strong>
                    <p>{previewSnippet(artifact.optimized_transcript || artifact.raw_transcript, 220)}</p>
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
        return (
          <textarea
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
            {chatHistory.length === 0 ? (
              <div className="detail-empty">
                <div className="center-empty-icon"><MessageSquareText size={28} /></div>
                <h2>AI Chat</h2>
                <p>Ask questions on the current transcript.</p>
              </div>
            ) : null}
            {chatHistory.map((message, index) => (
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
            />
            <select
              value={settings?.ai.active_provider ?? "none"}
              onChange={(event) => {
                const provider = event.target.value as AppSettings["ai"]["active_provider"];
                void patchSettings((current) => ({
                  ...current,
                  ai: {
                    ...current.ai,
                    active_provider: provider,
                  },
                }));
              }}
            >
              <option value="none">No AI provider</option>
              <option value="foundation_apple" disabled={!isMacOS}>Foundation Model</option>
              <option value="gemini">Gemini</option>
            </select>
            <button className="primary-button" onClick={() => void onSendChat()} disabled={isAskingChat}>
              {isAskingChat ? "..." : "Submit"}
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
            <article className="segment-row" key={`${segment.time}-${index}`}>
              <p style={{ fontSize: `${fontSize}px` }}>{segment.line}</p>
              <small>{segment.time}</small>
            </article>
          ))}
        </div>
      );
    }

    return (
      <textarea
        className="detail-editor"
        value={draftTranscript}
        onChange={(event) => setDraftTranscript(event.target.value)}
        style={{ fontSize: `${fontSize}px` }}
      />
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
          <div className="pill">Unknown</div>
          <input className="inspector-input" placeholder="Add a speaker..." disabled />
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
            value={settings?.ai.active_provider ?? "none"}
            onChange={(event) => {
              const provider = event.target.value as AppSettings["ai"]["active_provider"];
              void patchSettings((current) => ({
                ...current,
                ai: {
                  ...current.ai,
                  active_provider: provider,
                },
              }));
            }}
          >
            <option value="none">No AI provider</option>
            <option value="foundation_apple" disabled={!isMacOS}>Foundation Model</option>
            <option value="gemini">Gemini</option>
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
            setSection("settings");
            setSettingsPane("prompts");
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
        <section className="detail-main">
          <DetailToolbar
            leftSidebarOpen={leftSidebarOpen}
            rightSidebarOpen={rightSidebarOpen}
            detailMode={detailMode}
            title={activeArtifact ? activeArtifact.title : (activeJobTitle || "Transcribing")}
            hasArtifact={Boolean(activeArtifact)}
            hasActiveJob={Boolean(activeJobId)}
            transcriptionProgress={activeTranscriptionPercentage}
            onShowSidebar={() => setLeftSidebarOpen(true)}
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
            onCancel={() => void onCancel()}
          />

          <div className="detail-body">{renderDetailMain()}</div>

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
              <small>Checks GitHub releases in the background.</small>
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

          <div className="settings-actions-row">
            <button className="secondary-button" onClick={() => void onRefreshUpdates()} disabled={checkingUpdates}>
              {checkingUpdates ? "Checking..." : "Check Updates"}
            </button>
            {updateInfo ? (
              <small>
                {updateInfo.has_update
                  ? `Update ${updateInfo.latest_version} available`
                  : `Up to date (${updateInfo.current_version})`}
              </small>
            ) : null}
          </div>
          {updateInfo?.has_update && updateInfo.download_url ? (
            <a className="cta-link-button" href={updateInfo.download_url} target="_blank" rel="noreferrer">
              Download Update
            </a>
          ) : null}
        </section>
      </div>
    );
  }

  function renderSettingsTranscription(): JSX.Element {
    if (!settings) {
      return <div className="settings-placeholder">Settings unavailable.</div>;
    }

    const whisperOptions = settings.transcription.whisper_options ?? defaultWhisperOptions;

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <header>
            <h3>Transcription Defaults</h3>
            <p>Used for every new transcription job.</p>
          </header>

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

        <section className="settings-panel">
          <header>
            <h3>Whisper Settings</h3>
            <p>Direct decoding controls from whisper.cpp.</p>
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
            <div className="inspector-block">
              <h4>Transcription Runtime Health</h4>
              <p className="muted">
                Whisper CLI:{" "}
                <code>{runtimeHealth.whisper_cli_resolved || runtimeHealth.whisper_cli_path}</code>
              </p>
              <p className="muted">
                Whisper Stream:{" "}
                <code>{runtimeHealth.whisper_stream_resolved || runtimeHealth.whisper_stream_path}</code>
              </p>
              <p className="muted">
                Active model: <code>{runtimeHealth.model_filename}</code>{" "}
                {runtimeHealth.model_present ? (
                  <span className="kind-chip">Installed</span>
                ) : (
                  <span className="missing-chip">Missing</span>
                )}
              </p>
              <p className="muted">
                CoreML encoder{" "}
                {runtimeHealth.coreml_encoder_present ? (
                  <span className="kind-chip">Installed</span>
                ) : (
                  <span className="missing-chip">Missing</span>
                )}
              </p>
              {runtimeHealth.missing_models.length > 0 ? (
                <p className="muted">
                  Missing models: <code>{runtimeHealth.missing_models.join(", ")}</code>
                </p>
              ) : null}
              {runtimeHealth.missing_encoders.length > 0 ? (
                <p className="muted">
                  Missing encoders: <code>{runtimeHealth.missing_encoders.join(", ")}</code>
                </p>
              ) : null}
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
                  <span className={model.coreml_installed ? "kind-chip" : "missing-chip"}>
                    {model.coreml_installed ? "CoreML Ready" : "CoreML Missing"}
                  </span>
                  <button
                    className="secondary-button"
                    disabled={provisioning.running || (model.installed && model.coreml_installed)}
                    onClick={() => void onDownloadModel(model.key)}
                  >
                    {model.installed && model.coreml_installed ? "Installed" : "Download"}
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

    return (
      <div className="settings-stack">
        <section className="settings-panel">
          <h3>AI Services</h3>

          <label>
            Active provider
            <select
              value={settings.ai.active_provider}
              onChange={(event) => {
                const provider = event.target.value as AppSettings["ai"]["active_provider"];
                void patchSettings((current) => ({
                  ...current,
                  ai: {
                    ...current.ai,
                    active_provider: provider,
                  },
                }));
              }}
            >
              <option value="none">None</option>
              <option value="foundation_apple" disabled={!isMacOS}>
                Apple Foundation Model {isMacOS ? "" : "(macOS only)"}
              </option>
              <option value="gemini">Gemini</option>
            </select>
          </label>

          <label className="toggle-row">
            <span>Enable Apple Foundation provider</span>
            <input
              type="checkbox"
              checked={settings.ai.providers.foundation_apple.enabled}
              disabled={!isMacOS}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  ai: {
                    ...current.ai,
                    providers: {
                      ...current.ai.providers,
                      foundation_apple: {
                        ...current.ai.providers.foundation_apple,
                        enabled: event.target.checked,
                      },
                    },
                  },
                }));
              }}
            />
          </label>

          <label>
            Gemini API key
            <input
              type="password"
              placeholder="AIza..."
              value={settings.ai.providers.gemini.api_key ?? ""}
              onChange={(event) => {
                const value = event.target.value.trim();
                void patchSettings((current) => ({
                  ...current,
                  ai: {
                    ...current.ai,
                    providers: {
                      ...current.ai.providers,
                      gemini: {
                        ...current.ai.providers.gemini,
                        api_key: value.length > 0 ? event.target.value : null,
                      },
                    },
                  },
                }));
              }}
            />
          </label>

          <label>
            Gemini model
            <input
              value={settings.ai.providers.gemini.model}
              onChange={(event) => {
                void patchSettings((current) => ({
                  ...current,
                  ai: {
                    ...current.ai,
                    providers: {
                      ...current.ai.providers,
                      gemini: {
                        ...current.ai.providers.gemini,
                        model: event.target.value,
                      },
                    },
                  },
                }));
              }}
            />
          </label>
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
            {visibleSettingsPanes.map((pane) => (
              <button
                key={pane.key}
                className={settingsPane === pane.key ? "settings-nav-item active" : "settings-nav-item"}
                onClick={() => setSettingsPane(pane.key)}
              >
                <span>{pane.label}</span>
                <small>{pane.description}</small>
              </button>
            ))}
            {visibleSettingsPanes.length === 0 ? (
              <div className="settings-nav-empty">No section matches your search.</div>
            ) : null}
          </div>
        </aside>

        <section className="settings-content">
          {settingsPane === "transcription" ? renderSettingsTranscription() : null}
          {settingsPane === "local_models" ? renderSettingsLocalModels() : null}
          {settingsPane === "advanced" ? renderSettingsAdvanced() : null}
          {settingsPane === "general" ? renderSettingsGeneral() : null}
          {settingsPane === "ai_services" ? renderSettingsAiServices() : null}
          {settingsPane === "prompts" ? renderSettingsPrompts() : null}
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
    if (section === "settings") return renderSettings();
    return renderDetail();
  }

  return (
    <main className="app-shell">
      <section className={leftSidebarOpen ? "window-frame" : "window-frame left-collapsed"}>
        {leftSidebarOpen ? (
          <aside className="left-sidebar">
            <div className="sidebar-header">
              <button
                className="icon-button"
                onClick={() => setLeftSidebarOpen(false)}
                title="Hide sidebar"
              >
                <PanelLeftClose size={16} />
              </button>
            </div>
            <div className="sidebar-section">
              <button className={section === "home" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("home")}>
                <House size={16} />
                Home
              </button>
              <button className={section === "queue" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("queue")}>
                <ListChecks size={16} />
                Queue
              </button>
              <button className={section === "realtime" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("realtime")}>
                <Radio size={16} />
                Live
              </button>
            </div>

            {activeArtifact ? (
              <div className="sidebar-section">
                <h4>Open</h4>
                <div className="sidebar-open-row" title={activeArtifact.title}>
                  <button
                    className="sidebar-item sidebar-open-item"
                    onClick={() => {
                      setSection("detail");
                      setDetailMode("transcript");
                      setInspectorMode("details");
                    }}
                  >
                    <FileAudio size={16} />
                    <span className="sidebar-item-label">{activeArtifact.title}</span>
                  </button>
                  <button
                    className="sidebar-open-close"
                    onClick={(event) => {
                      event.stopPropagation();
                      onCloseOpenedArtifact();
                    }}
                    title="Close opened transcription"
                    aria-label="Close opened transcription"
                  >
                    <X size={12} />
                  </button>
                </div>
              </div>
            ) : null}

            <div className="sidebar-section">
              <h4>History</h4>
              <button
                className={section === "history" ? "sidebar-item active" : "sidebar-item"}
                onClick={() => setSection("history")}
              >
                <HistoryIcon size={16} />
                Transcriptions
              </button>
              <button
                className={section === "deleted_history" ? "sidebar-item active" : "sidebar-item"}
                onClick={() => setSection("deleted_history")}
              >
                <Trash2 size={16} />
                Recently Deleted
              </button>
            </div>

            <div className="sidebar-footer">
              <button className={section === "settings" ? "sidebar-item active" : "sidebar-item"} onClick={() => setSection("settings")}>
                <Settings2 size={16} />
                Settings
              </button>
            </div>
          </aside>
        ) : null}

        <section className="main-area">
          {section !== "detail" ? (
            <header className="main-topbar">
              <div className="topbar-title">
                {!leftSidebarOpen ? (
                  <button
                    className="icon-button"
                    onClick={() => setLeftSidebarOpen(true)}
                    title="Show sidebar"
                  >
                    <PanelLeftOpen size={16} />
                  </button>
                ) : null}
                {section === "home" ? null : (
                  <h1>
                    {section === "queue"
                      ? "Queue"
                      : section === "realtime"
                        ? "Live"
                        : section === "settings"
                          ? "Settings"
                          : section === "deleted_history"
                            ? "Recently Deleted"
                            : "Transcriptions"}
                  </h1>
                )}
              </div>

              {section === "home" || section === "queue" || section === "realtime" ? (
                <div className="topbar-controls home-topbar-controls">
                  <label className="select-chip">
                    <span className="chip-label">
                      <Mic size={12} />
                      Model
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
                      Language
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

          {error ? <p className="error-banner">{error}</p> : null}
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
