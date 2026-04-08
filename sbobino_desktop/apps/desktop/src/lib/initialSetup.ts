import type {
  ProvisioningModelCatalogEntry,
  PyannoteRuntimeHealth,
  RuntimeHealth,
  SpeechModel,
} from "../types";

export const INITIAL_SETUP_REQUIRED_MODELS: SpeechModel[] = ["base", "large_turbo"];
export const INITIAL_SETUP_REQUIRES_PYANNOTE = true;

export type InitialSetupStepId =
  | "privacy"
  | "speech-runtime"
  | "pyannote-runtime"
  | "whisper-models"
  | "final-validation";

export type InitialSetupStepStatus = "pending" | "running" | "completed" | "failed";

export type InitialSetupReportStep = {
  id: InitialSetupStepId;
  label: string;
  status: InitialSetupStepStatus;
  detail: string | null;
  started_at: string | null;
  finished_at: string | null;
};

export type InitialSetupReport = {
  build_version: string;
  privacy_accepted: boolean;
  setup_complete: boolean;
  final_reason_code: string | null;
  final_error: string | null;
  runtime_health: RuntimeHealth | null;
  steps: InitialSetupReportStep[];
  updated_at: string;
};

const PYANNOTE_REPAIR_REASON_CODES = new Set([
  "pyannote_arch_mismatch",
  "pyannote_version_mismatch",
  "pyannote_repair_required",
  "pyannote_install_incomplete",
  "pyannote_checksum_invalid",
]);

export function isProvisionedModelReady(
  entry: ProvisioningModelCatalogEntry | undefined,
  requireCoreml: boolean,
): boolean {
  if (!entry?.installed) {
    return false;
  }
  if (!requireCoreml) {
    return true;
  }
  return entry.coreml_installed;
}

export function findProvisioningModelEntry(
  modelCatalog: ProvisioningModelCatalogEntry[],
  model: SpeechModel,
): ProvisioningModelCatalogEntry | undefined {
  return modelCatalog.find((entry) => entry.key === model);
}

export function getInitialSetupMissingModels(
  modelCatalog: ProvisioningModelCatalogEntry[],
  requireCoreml: boolean,
): SpeechModel[] {
  return INITIAL_SETUP_REQUIRED_MODELS.filter((model) =>
    !isProvisionedModelReady(findProvisioningModelEntry(modelCatalog, model), requireCoreml));
}

export function shouldRepairPyannoteRuntime(
  health: PyannoteRuntimeHealth | null | undefined,
): boolean {
  const reasonCode = health?.reason_code?.trim();
  if (!reasonCode) {
    return false;
  }
  return PYANNOTE_REPAIR_REASON_CODES.has(reasonCode);
}

export function isInitialSetupComplete(
  privacyAccepted: boolean,
  runtimeHealth: RuntimeHealth | null | undefined,
  modelCatalog: ProvisioningModelCatalogEntry[],
): boolean {
  if (!privacyAccepted || !runtimeHealth) {
    return false;
  }

  const runtimeReady = runtimeHealth.ffmpeg_available
    && runtimeHealth.whisper_cli_available
    && runtimeHealth.whisper_stream_available;
  const pyannoteReady = !INITIAL_SETUP_REQUIRES_PYANNOTE || runtimeHealth.pyannote.ready;
  const modelsReady = getInitialSetupMissingModels(
    modelCatalog,
    runtimeHealth.is_apple_silicon,
  ).length === 0;

  return runtimeReady && pyannoteReady && modelsReady;
}
