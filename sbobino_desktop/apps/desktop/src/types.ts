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

export type SpeechModel = "tiny" | "base" | "small" | "medium" | "large_turbo";
export type ArtifactKind = "file" | "realtime";

export type AiProvider = "none" | "foundation_apple" | "gemini";
export type PromptCategory =
  | "cleanup"
  | "summary"
  | "insights"
  | "qa"
  | "rewrite"
  | "custom";
export type PromptTask = "optimize" | "summary" | "faq";

export type GeneralSettings = {
  auto_update_enabled: boolean;
  auto_update_repo: string;
};

export type WhisperOptions = {
  translate_to_english: boolean;
  no_context: boolean;
  split_on_word: boolean;
  temperature: number;
  entropy_threshold: number;
  logprob_threshold: number;
  word_threshold: number;
  best_of: number;
  beam_size: number;
  threads: number;
  processors: number;
};

export type TranscriptionSettings = {
  model: SpeechModel;
  language: LanguageCode;
  whisper_cli_path: string;
  ffmpeg_path: string;
  models_dir: string;
  enable_ai_post_processing: boolean;
  whisper_options: WhisperOptions;
};

export type FoundationProviderSettings = {
  enabled: boolean;
};

export type GeminiProviderSettings = {
  api_key: string | null;
  model: string;
};

export type AiProviderSettings = {
  foundation_apple: FoundationProviderSettings;
  gemini: GeminiProviderSettings;
};

export type AiSettings = {
  active_provider: AiProvider;
  providers: AiProviderSettings;
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
};

export type PromptSettings = {
  templates: PromptTemplate[];
  bindings: PromptBindings;
};

export type AppSettings = {
  // Legacy fields kept for wire compatibility with existing commands.
  model: SpeechModel;
  language: LanguageCode;
  ai_post_processing: boolean;
  gemini_model: string;
  gemini_api_key: string | null;
  whisper_cli_path: string;
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
};

export type TranscriptArtifact = {
  id: string;
  job_id: string;
  title: string;
  kind: ArtifactKind;
  input_path: string;
  raw_transcript: string;
  optimized_transcript: string;
  summary: string;
  faqs: string;
  metadata: Record<string, string>;
  created_at: string;
  updated_at: string;
};

export type StartTranscriptionPayload = {
  input_path: string;
  language: LanguageCode;
  model: SpeechModel;
  enable_ai: boolean;
  whisper_options: WhisperOptions;
};

export type RealtimeDeltaKind = "append_final" | "update_preview";

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
};

export type ProvisioningProgressEvent = {
  current: number;
  total: number;
  asset: string;
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
  whisper_cli_path: string;
  whisper_cli_resolved: string;
  whisper_stream_path: string;
  whisper_stream_resolved: string;
  models_dir_configured: string;
  models_dir_resolved: string;
  model_filename: string;
  model_present: boolean;
  coreml_encoder_present: boolean;
  missing_models: string[];
  missing_encoders: string[];
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
