import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AiSettings,
  AppSettings,
  ChatArtifactPayload,
  ArtifactKind,
  EnsureRuntimeResponse,
  JobProgress,
  PromptTask,
  PromptTemplate,
  ProvisioningProgressEvent,
  ProvisioningModelCatalogEntry,
  ProvisioningStatus,
  RealtimeDelta,
  RealtimeStatusEvent,
  RuntimeHealth,
  StartTranscriptionPayload,
  TestPromptResponse,
  TranscriptionStartPreflight,
  TranscriptionDelta,
  TranscriptArtifact,
  SummarizeArtifactPayload,
  UpdateAiProvidersPayload,
  UpdateCheckResponse,
  UpdateSettingsPartialPayload,
} from "../types";

export async function fetchSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

export async function fetchSettingsSnapshot(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings_snapshot");
}

export async function saveSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("update_settings", { settings });
}

export async function saveSettingsPartial(
  payload: UpdateSettingsPartialPayload,
): Promise<AppSettings> {
  return invoke<AppSettings>("update_settings_partial", { payload });
}

export async function fetchAiProviders(): Promise<AiSettings> {
  return invoke<AiSettings>("get_ai_providers");
}

export async function updateAiProviders(
  payload: UpdateAiProvidersPayload,
): Promise<AiSettings> {
  return invoke<AiSettings>("update_ai_providers", {
    payload: {
      ...payload,
      gemini_api_key:
        payload.gemini_api_key === undefined ? undefined : payload.gemini_api_key,
    },
  });
}

export async function listGeminiModels(api_key?: string): Promise<string[]> {
  if (!api_key || api_key.trim().length === 0) {
    return invoke<string[]>("list_gemini_models");
  }
  return invoke<string[]>("list_gemini_models", { payload: { api_key } });
}

export async function listPromptTemplates(): Promise<PromptTemplate[]> {
  return invoke<PromptTemplate[]>("list_prompts");
}

export async function savePromptTemplate(payload: {
  template: PromptTemplate;
  bind_task?: PromptTask;
}): Promise<PromptTemplate[]> {
  return invoke<PromptTemplate[]>("save_prompt", {
    payload: {
      template: payload.template,
      bind_task: payload.bind_task ?? null,
    },
  });
}

export async function deletePromptTemplate(id: string): Promise<PromptTemplate[]> {
  return invoke<PromptTemplate[]>("delete_prompt", { payload: { id } });
}

export async function resetPromptTemplates(): Promise<PromptTemplate[]> {
  return invoke<PromptTemplate[]>("reset_prompts");
}

export async function testPromptTemplate(payload: {
  input: string;
  task: PromptTask;
  prompt_override?: string;
  model_override?: string;
  language?: AppSettings["language"];
}): Promise<TestPromptResponse> {
  return invoke<TestPromptResponse>("test_prompt", { payload });
}

export async function startTranscription(
  payload: StartTranscriptionPayload,
): Promise<{ job_id: string }> {
  return invoke<{ job_id: string }>("start_transcription", { payload });
}

export async function cancelTranscription(job_id: string): Promise<void> {
  return invoke<void>("cancel_transcription", { payload: { job_id } });
}

export async function listRecentArtifacts(limit = 40): Promise<TranscriptArtifact[]> {
  return invoke<TranscriptArtifact[]>("list_recent_artifacts", { limit });
}

export async function listArtifacts(payload?: {
  kind?: ArtifactKind;
  query?: string;
  limit?: number;
  offset?: number;
}): Promise<TranscriptArtifact[]> {
  return invoke<TranscriptArtifact[]>("list_artifacts", { payload });
}

export async function listDeletedArtifacts(payload?: {
  kind?: ArtifactKind;
  query?: string;
  limit?: number;
  offset?: number;
}): Promise<TranscriptArtifact[]> {
  return invoke<TranscriptArtifact[]>("list_deleted_artifacts", { payload });
}

export async function getArtifact(id: string): Promise<TranscriptArtifact | null> {
  return invoke<TranscriptArtifact | null>("get_artifact", { payload: { id } });
}

export async function updateArtifact(payload: {
  id: string;
  optimized_transcript: string;
  summary: string;
  faqs: string;
}): Promise<TranscriptArtifact | null> {
  return invoke<TranscriptArtifact | null>("update_artifact", { payload });
}

export async function updateArtifactTimeline(payload: {
  id: string;
  timeline_v2: string;
}): Promise<TranscriptArtifact | null> {
  return invoke<TranscriptArtifact | null>("update_artifact_timeline", { payload });
}

export async function renameArtifact(payload: {
  id: string;
  new_title: string;
}): Promise<TranscriptArtifact | null> {
  return invoke<TranscriptArtifact | null>("rename_artifact", { payload });
}

export async function deleteArtifacts(ids: string[]): Promise<{ deleted: number }> {
  return invoke<{ deleted: number }>("delete_artifacts", { payload: { ids } });
}

export async function restoreArtifacts(ids: string[]): Promise<{ restored: number }> {
  return invoke<{ restored: number }>("restore_artifacts", { payload: { ids } });
}

export async function hardDeleteArtifacts(ids: string[]): Promise<{ deleted: number }> {
  return invoke<{ deleted: number }>("hard_delete_artifacts", { payload: { ids } });
}

export async function emptyDeletedArtifacts(): Promise<{ deleted: number }> {
  return invoke<{ deleted: number }>("empty_deleted_artifacts");
}

export async function exportArtifact(payload: {
  id: string;
  format: "txt" | "docx" | "html" | "pdf" | "json" | "srt" | "vtt" | "csv" | "md";
  destination_path: string;
  style?: "transcript" | "subtitles" | "segments";
  options?: {
    include_timestamps: boolean;
    grouping: "none" | "speaker_paragraphs";
  };
  segments?: Array<{
    time: string;
    line: string;
  }>;
  content_override?: string;
}): Promise<{ path: string }> {
  return invoke<{ path: string }>("export_artifact", { payload });
}

export async function chatArtifact(payload: ChatArtifactPayload): Promise<string> {
  return invoke<string>("chat_artifact", { payload });
}

export async function summarizeArtifact(payload: SummarizeArtifactPayload): Promise<string> {
  return invoke<string>("summarize_artifact", { payload });
}

export type OptimizeArtifactPayload = {
  id: string;
  text: string;
};

export async function optimizeArtifact(payload: OptimizeArtifactPayload): Promise<string> {
  return invoke<string>("optimize_artifact", { payload });
}

export async function startRealtime(payload?: {
  model?: AppSettings["model"];
  language?: AppSettings["language"];
  resume_artifact_id?: string;
}): Promise<{ started: boolean }> {
  return invoke<{ started: boolean }>("start_realtime", { payload });
}

export async function pauseRealtime(): Promise<void> {
  return invoke<void>("pause_realtime");
}

export async function resumeRealtime(): Promise<void> {
  return invoke<void>("resume_realtime");
}

export async function stopRealtime(save = true): Promise<{
  saved: boolean;
  artifact: TranscriptArtifact | null;
}> {
  return invoke("stop_realtime", { payload: { save } });
}

export async function listRealtimeSessions(): Promise<TranscriptArtifact[]> {
  return invoke<TranscriptArtifact[]>("list_realtime_sessions");
}

export async function loadRealtimeSession(id: string): Promise<TranscriptArtifact | null> {
  return invoke<TranscriptArtifact | null>("load_realtime_session", { payload: { id } });
}

export async function provisioningStatus(): Promise<ProvisioningStatus> {
  return invoke<ProvisioningStatus>("provisioning_status");
}

export async function provisioningStart(include_coreml = true): Promise<{ started: boolean }> {
  return invoke<{ started: boolean }>("provisioning_start", {
    payload: { include_coreml },
  });
}

export async function provisioningModels(): Promise<ProvisioningModelCatalogEntry[]> {
  return invoke<ProvisioningModelCatalogEntry[]>("provisioning_models");
}

export async function provisioningDownloadModel(payload: {
  model: AppSettings["model"];
  include_coreml?: boolean;
}): Promise<{ started: boolean }> {
  return invoke<{ started: boolean }>("provisioning_download_model", { payload });
}

export async function provisioningCancel(): Promise<void> {
  return invoke<void>("provisioning_cancel");
}

export async function fetchRuntimeHealth(): Promise<RuntimeHealth> {
  return invoke<RuntimeHealth>("get_transcription_runtime_health");
}

export async function ensureTranscriptionRuntime(): Promise<EnsureRuntimeResponse> {
  return invoke<EnsureRuntimeResponse>("ensure_transcription_runtime");
}

export async function fetchTranscriptionStartPreflight(payload: {
  model: StartTranscriptionPayload["model"];
}): Promise<TranscriptionStartPreflight> {
  return invoke<TranscriptionStartPreflight>("get_transcription_start_preflight", { payload });
}

export async function readAudioFile(path: string): Promise<number[]> {
  return invoke<number[]>("read_audio_file", { payload: { path } });
}

export async function writeTrimmedAudio(
  inputPath: string,
  regions: Array<{ start: number; end: number }>,
): Promise<{ path: string }> {
  return invoke<{ path: string }>("write_trimmed_audio", {
    payload: { input_path: inputPath, regions },
  });
}

export async function checkUpdates(): Promise<UpdateCheckResponse> {
  return invoke<UpdateCheckResponse>("check_updates");
}

export async function openSettingsWindow(pane?: string): Promise<boolean> {
  if (!pane) {
    return invoke<boolean>("open_settings_window");
  }
  return invoke<boolean>("open_settings_window", { payload: { pane } });
}

export async function subscribeSettingsUpdated(
  onUpdated: (settings: AppSettings) => void,
): Promise<() => void> {
  const unlisten = await listen<AppSettings>("settings://updated", (event) => {
    onUpdated(event.payload);
  });
  return unlisten;
}

export async function subscribeSettingsNavigate(
  onNavigate: (pane: string) => void,
): Promise<() => void> {
  const unlisten = await listen<string>("settings://navigate", (event) => {
    onNavigate(event.payload);
  });
  return unlisten;
}

export async function subscribeJobProgress(
  onProgress: (progress: JobProgress) => void,
): Promise<() => void> {
  const unlisten = await listen<JobProgress>("transcription://progress", (event) => {
    onProgress(event.payload);
  });
  return unlisten;
}

export async function subscribeJobCompleted(
  onCompleted: (artifact: TranscriptArtifact) => void,
): Promise<() => void> {
  const unlisten = await listen<TranscriptArtifact>("transcription://completed", (event) => {
    onCompleted(event.payload);
  });
  return unlisten;
}

export async function subscribeTranscriptionDelta(
  onDelta: (delta: TranscriptionDelta) => void,
): Promise<() => void> {
  const unlisten = await listen<TranscriptionDelta>("transcription://delta", (event) => {
    onDelta(event.payload);
  });
  return unlisten;
}

export async function subscribeJobFailed(
  onFailed: (error: { job_id: string; message: string }) => void,
): Promise<() => void> {
  const unlisten = await listen<{ job_id: string; message: string }>(
    "transcription://failed",
    (event) => {
      onFailed(event.payload);
    },
  );
  return unlisten;
}

export async function subscribeRealtimeDelta(
  onDelta: (delta: RealtimeDelta) => void,
): Promise<() => void> {
  const unlisten = await listen<RealtimeDelta>("realtime://delta", (event) => {
    onDelta(event.payload);
  });
  return unlisten;
}

export async function subscribeRealtimeStatus(
  onStatus: (status: RealtimeStatusEvent) => void,
): Promise<() => void> {
  const unlisten = await listen<RealtimeStatusEvent>("realtime://status", (event) => {
    onStatus(event.payload);
  });
  return unlisten;
}

export async function subscribeRealtimeSaved(
  onSaved: (artifact: TranscriptArtifact) => void,
): Promise<() => void> {
  const unlisten = await listen<TranscriptArtifact>("realtime://saved", (event) => {
    onSaved(event.payload);
  });
  return unlisten;
}

export async function subscribeProvisioningProgress(
  onProgress: (progress: ProvisioningProgressEvent) => void,
): Promise<() => void> {
  const unlisten = await listen<ProvisioningProgressEvent>(
    "provisioning://progress",
    (event) => {
      onProgress(event.payload);
    },
  );
  return unlisten;
}

export async function subscribeProvisioningStatus(
  onStatus: (payload: { state: string; message: string }) => void,
): Promise<() => void> {
  const unlisten = await listen<{ state: string; message: string }>(
    "provisioning://status",
    (event) => {
      onStatus(event.payload);
    },
  );
  return unlisten;
}
