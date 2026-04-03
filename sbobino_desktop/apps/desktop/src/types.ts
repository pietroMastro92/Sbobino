export type LanguageCode =
  | "auto"
  | "en"
  | "it"
  | "fr"
  | "de"
  | "es"
  | "pt"
  | "zh"
  | "ja";

export type AppLanguage = "en" | "it" | "es" | "de";

export type SpeechModel = "tiny" | "base" | "small" | "medium" | "large_turbo";
export type TranscriptionEngine = "whisper_cpp" | "whisper_kit";
export type ArtifactKind = "file" | "realtime";
export type ArtifactSourceOrigin = "imported" | "trimmed" | "realtime" | "legacy_external";
export type ArtifactAudioBackfillStatus = "imported" | "pending_backfill" | "missing";
export type AppearanceMode = "system" | "light" | "dark";

export type AiProvider = "none" | "foundation_apple" | "gemini";
export type RemoteServiceKind =
  | "google"
  | "open_ai"
  | "anthropic"
  | "azure"
  | "lm_studio"
  | "ollama"
  | "open_router"
  | "xai"
  | "hugging_face"
  | "custom";
export type PromptCategory =
  | "cleanup"
  | "summary"
  | "insights"
  | "qa"
  | "rewrite"
  | "custom";
export type PromptTask = "optimize" | "summary" | "faq" | "emotion_analysis";

export type GeneralSettings = {
  auto_update_enabled: boolean;
  auto_update_repo: string;
  privacy_policy_version_accepted: string | null;
  privacy_policy_accepted_at: string | null;
  appearance_mode: AppearanceMode;
  app_language: AppLanguage;
};

export type WhisperOptions = {
  translate_to_english: boolean;
  no_context: boolean;
  split_on_word: boolean;
  tinydiarize: boolean;
  diarize: boolean;
  temperature: number;
  temperature_increment_on_fallback: number;
  temperature_fallback_count: number;
  entropy_threshold: number;
  logprob_threshold: number;
  first_token_logprob_threshold: number;
  no_speech_threshold: number;
  word_threshold: number;
  best_of: number;
  beam_size: number;
  threads: number;
  processors: number;
  use_prefill_prompt: boolean;
  use_prefill_cache: boolean;
  without_timestamps: boolean;
  word_timestamps: boolean;
  prompt: string | null;
  concurrent_worker_count: number;
  chunking_strategy: "none" | "vad";
  audio_encoder_compute_units:
    | "all"
    | "cpu_only"
    | "cpu_and_gpu"
    | "cpu_and_neural_engine";
  text_decoder_compute_units:
    | "all"
    | "cpu_only"
    | "cpu_and_gpu"
    | "cpu_and_neural_engine";
};

export type SpeakerDiarizationSettings = {
  enabled: boolean;
  device: "auto" | "cpu" | "mps";
  speaker_colors: Record<string, string>;
};

export type TranscriptionSettings = {
  engine: TranscriptionEngine;
  model: SpeechModel;
  language: LanguageCode;
  whisper_cli_path: string;
  whisperkit_cli_path: string;
  ffmpeg_path: string;
  models_dir: string;
  enable_ai_post_processing: boolean;
  speaker_diarization: SpeakerDiarizationSettings;
  whisper_options: WhisperOptions;
};

export type FoundationProviderSettings = {
  enabled: boolean;
};

export type GeminiProviderSettings = {
  api_key: string | null;
  has_api_key: boolean;
  model: string;
};

export type AiProviderSettings = {
  foundation_apple: FoundationProviderSettings;
  gemini: GeminiProviderSettings;
};

export type AiSettings = {
  active_provider: AiProvider;
  active_remote_service_id: string | null;
  providers: AiProviderSettings;
  remote_services: RemoteServiceConfig[];
};

export type AiCapabilityStatus = {
  available: boolean;
  fallback_available: boolean;
  unavailable_reason?: string | null;
};

export type RemoteServiceConfig = {
  id: string;
  kind: RemoteServiceKind;
  label: string;
  enabled: boolean;
  api_key: string | null;
  has_api_key: boolean;
  model: string | null;
  base_url: string | null;
};

export type PromptTemplate = {
  id: string;
  name: string;
  icon: string;
  category: PromptCategory;
  body: string;
  builtin: boolean;
  updated_at: string;
};

export type PromptBindings = {
  optimize_prompt_id: string;
  summary_prompt_id: string;
  faq_prompt_id: string;
  emotion_prompt_id: string;
};

export type PromptSettings = {
  templates: PromptTemplate[];
  bindings: PromptBindings;
};

export type AppSettings = {
  // Legacy fields kept for wire compatibility with existing commands.
  transcription_engine: TranscriptionEngine;
  model: SpeechModel;
  language: LanguageCode;
  ai_post_processing: boolean;
  gemini_model: string;
  gemini_api_key: string | null;
  gemini_api_key_present: boolean;
  whisper_cli_path: string;
  whisperkit_cli_path: string;
  ffmpeg_path: string;
  models_dir: string;
  auto_update_enabled: boolean;
  auto_update_repo: string;

  // Structured settings.
  general: GeneralSettings;
  transcription: TranscriptionSettings;
  ai: AiSettings;
  prompts: PromptSettings;
};

export type JobStage =
  | "queued"
  | "preparing_audio"
  | "transcribing"
  | "diarizing"
  | "optimizing"
  | "summarizing"
  | "persisting"
  | "completed"
  | "failed"
  | "cancelled";

export type JobProgress = {
  job_id: string;
  stage: JobStage;
  message: string;
  percentage: number;
  current_seconds?: number | null;
  total_seconds?: number | null;
};

export type TranscriptionDelta = {
  job_id: string;
  text: string;
  sequence: number;
  mode?: "append" | "replace";
};

export type TranscriptArtifact = {
  id: string;
  job_id: string;
  title: string;
  kind: ArtifactKind;
  source_label: string;
  source_origin: ArtifactSourceOrigin;
  audio_available: boolean;
  audio_backfill_status: ArtifactAudioBackfillStatus;
  revision: number;
  raw_transcript: string;
  optimized_transcript: string;
  summary: string;
  faqs: string;
  metadata: Record<string, string>;
  parent_artifact_id?: string | null;
  processing_engine?: string | null;
  processing_model?: string | null;
  processing_language?: string | null;
  audio_duration_seconds?: number | null;
  audio_byte_size?: number | null;
  created_at: string;
  updated_at: string;
};

export type ExportAppBackupResponse = {
  path: string;
  artifact_count: number;
  deleted_artifact_count: number;
  audio_file_count: number;
  exported_at: string;
};

export type ImportAppBackupResponse = {
  artifact_count: number;
  deleted_artifact_count: number;
  imported_at: string;
};

export type ArtifactAiContextOptions = {
  include_timestamps: boolean;
  include_speakers: boolean;
};

export type ChatArtifactPayload = ArtifactAiContextOptions & {
  id: string;
  prompt: string;
};

export type SummarizeArtifactPayload = ArtifactAiContextOptions & {
  id: string;
  language: LanguageCode;
  sections: boolean;
  bullet_points: boolean;
  action_items: boolean;
  key_points_only: boolean;
  custom_prompt?: string | null;
};

export type EmotionAnalysisPayload = ArtifactAiContextOptions & {
  id: string;
  language: LanguageCode;
  speaker_dynamics: boolean;
};

export type EmotionOverview = {
  primary_emotions: string[];
  emotional_arc: string;
  speaker_dynamics?: string | null;
  confidence_note?: string | null;
};

export type EmotionTimelineEntry = {
  segment_index: number;
  time_label?: string | null;
  start_seconds?: number | null;
  end_seconds?: number | null;
  speaker_label?: string | null;
  dominant_emotions: string[];
  valence_score: number;
  intensity_score: number;
  evidence_text: string;
  shift_label?: string | null;
};

export type EmotionSemanticNode = {
  id: string;
  label: string;
  kind: string;
  weight: number;
};

export type EmotionSemanticEdge = {
  source: string;
  target: string;
  weight: number;
  relation: string;
};

export type EmotionSemanticCluster = {
  id: string;
  label: string;
  node_ids: string[];
  segment_indices: number[];
  summary: string;
};

export type EmotionSemanticMap = {
  nodes: EmotionSemanticNode[];
  edges: EmotionSemanticEdge[];
  clusters: EmotionSemanticCluster[];
};

export type EmotionBridge = {
  from_segment_index: number;
  to_segment_index: number;
  bridge_theme: string;
  reason: string;
  shared_keywords: string[];
};

export type EmotionAnalysisResult = {
  overview: EmotionOverview;
  timeline: EmotionTimelineEntry[];
  semantic_map: EmotionSemanticMap;
  bridges: EmotionBridge[];
  reflection_prompts: string[];
  narrative_markdown: string;
};

export type TimelineV2Word = {
  text: string;
  start_seconds?: number;
  end_seconds?: number;
  confidence?: number;
};

export type TimelineV2Segment = {
  text: string;
  start_seconds?: number;
  end_seconds?: number;
  speaker_id?: string;
  speaker_label?: string;
  words?: TimelineV2Word[];
};

export type TimelineV2 = {
  version: number;
  segments: TimelineV2Segment[];
};

export type StartTranscriptionPayload = {
  input_path: string;
  engine: TranscriptionEngine;
  language: LanguageCode;
  model: SpeechModel;
  enable_ai: boolean;
  whisper_options: WhisperOptions;
  title?: string;
  parent_id?: string;
};

export type WriteTrimmedAudioResponse = {
  path: string;
  duration_seconds: number;
  file_size_bytes: number;
};

export type RealtimeDeltaKind = "append_final" | "replace_final" | "update_preview";

export type RealtimeDelta = {
  kind: RealtimeDeltaKind;
  text: string;
};

export type RealtimeStatusEvent = {
  state: string;
  message: string;
};

export type ProvisioningStatus = {
  ready: boolean;
  models_dir: string;
  missing_models: string[];
  missing_encoders: string[];
  pyannote: PyannoteRuntimeHealth;
};

export type ProvisioningProgressEvent = {
  current: number;
  total: number;
  asset: string;
  asset_kind: "whisper_model" | "whisper_encoder" | "pyannote_runtime" | "pyannote_model";
  stage: string;
  percentage: number;
};

export type ProvisioningModelCatalogEntry = {
  key: SpeechModel;
  label: string;
  model_file: string;
  installed: boolean;
  coreml_installed: boolean;
};

export type RuntimeHealth = {
  host_os: string;
  host_arch: string;
  is_apple_silicon: boolean;
  preferred_engine: TranscriptionEngine;
  configured_engine: TranscriptionEngine;
  whisper_cli_path: string;
  whisper_cli_resolved: string;
  whisper_cli_available: boolean;
  whisper_stream_path: string;
  whisper_stream_resolved: string;
  whisper_stream_available: boolean;
  models_dir_configured: string;
  models_dir_resolved: string;
  model_filename: string;
  model_present: boolean;
  coreml_encoder_present: boolean;
  missing_models: string[];
  missing_encoders: string[];
  pyannote: PyannoteRuntimeHealth;
};

export type TranscriptionStartPreflight = {
  allowed: boolean;
  reason_code: string;
  message: string;
  engine: TranscriptionEngine;
  model_filename: string;
  model_path: string;
  whisper_cli_resolved: string;
  whisper_stream_resolved: string;
  pyannote: PyannoteRuntimeHealth;
};

export type PyannoteRuntimeHealth = {
  enabled: boolean;
  ready: boolean;
  runtime_installed: boolean;
  model_installed: boolean;
  arch: string;
  device: string;
  source: string;
  reason_code: string;
  message: string;
};

export type EnsureRuntimeResponse = {
  ready: boolean;
  engine: TranscriptionEngine;
  did_setup: boolean;
  message: string;
  whisper_cli_resolved: string;
  whisper_stream_resolved: string;
};

export type UpdateCheckResponse = {
  has_update: boolean;
  current_version: string;
  latest_version: string | null;
  download_url: string | null;
};

export type UpdateSettingsPartialPayload = {
  general?: GeneralSettings;
  transcription?: TranscriptionSettings;
  ai?: AiSettings;
  prompts?: PromptSettings;
};

export type UpdateAiProvidersPayload = {
  active_provider?: AiProvider;
  foundation_apple_enabled?: boolean;
  gemini_api_key?: string | null;
  gemini_model?: string;
};

export type TestPromptResponse = {
  output: string;
  summary: string;
  faqs: string;
  model: string;
};
