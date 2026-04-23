import type {
  ProvisioningModelCatalogEntry,
  PyannoteRuntimeHealth,
  RuntimeHealth,
  SpeechModel,
} from "../types";

export const INITIAL_SETUP_REQUIRED_MODELS: SpeechModel[] = [
  "base",
  "large_turbo",
];
export const INITIAL_SETUP_REQUIRES_PYANNOTE = false;

export type InitialSetupStepId =
  | "privacy"
  | "speech-runtime"
  | "pyannote-runtime"
  | "whisper-models"
  | "final-validation";

export type InitialSetupStepStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed";

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
  startup_gate_complete?: boolean;
  validated_build_version?: string | null;
  final_reason_code: string | null;
  final_error: string | null;
  last_validation_reason_code?: string | null;
  last_validation_error?: string | null;
  runtime_health: RuntimeHealth | null;
  steps: InitialSetupReportStep[];
  updated_at: string;
  trusted_for_fast_start?: boolean;
  requires_post_update_validation?: boolean;
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
  return INITIAL_SETUP_REQUIRED_MODELS.filter(
    (model) =>
      !isProvisionedModelReady(
        findProvisioningModelEntry(modelCatalog, model),
        requireCoreml,
      ),
  );
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

  const runtimeReady = isRuntimeToolchainReady(runtimeHealth);
  const pyannoteReady =
    !INITIAL_SETUP_REQUIRES_PYANNOTE || runtimeHealth.pyannote.ready;
  const modelsReady =
    getInitialSetupMissingModels(modelCatalog, runtimeHealth.is_apple_silicon)
      .length === 0;

  return runtimeReady && pyannoteReady && modelsReady;
}

export function canWarmStartFromSetupReport(
  privacyAccepted: boolean,
  report: InitialSetupReport | null | undefined,
): boolean {
  if (!privacyAccepted || !report) {
    return false;
  }

  if (report.trusted_for_fast_start === false) {
    return false;
  }

  return Boolean(report.startup_gate_complete ?? report.setup_complete);
}

export function shouldBlockMainUiDuringStartup(payload: {
  hasSettings: boolean;
  privacyAccepted: boolean;
  warmStartEligible: boolean;
  startupRequirementsLoaded: boolean;
  initialSetupReady: boolean;
}): boolean {
  const {
    hasSettings,
    privacyAccepted,
    warmStartEligible,
    startupRequirementsLoaded,
    initialSetupReady,
  } = payload;

  if (!hasSettings || !privacyAccepted) {
    return true;
  }

  if (warmStartEligible) {
    return false;
  }

  return !startupRequirementsLoaded || !initialSetupReady;
}

export function isRuntimeToolchainReady(
  runtimeHealth: RuntimeHealth | null | undefined,
): boolean {
  if (!runtimeHealth) {
    return false;
  }

  const managedRuntime = getManagedRuntime(runtimeHealth);
  if (runtimeHealth.managed_runtime_required) {
    return managedRuntime.ready;
  }

  return (
    runtimeHealth.ffmpeg_available &&
    runtimeHealth.whisper_cli_available &&
    runtimeHealth.whisper_stream_available
  );
}

export function getRuntimeToolchainFailureMessage(
  runtimeHealth: RuntimeHealth | null | undefined,
): string | null {
  if (!runtimeHealth) {
    return null;
  }

  const managedRuntime = getManagedRuntime(runtimeHealth);
  if (!managedRuntime.ffmpeg.available) {
    return managedRuntime.ffmpeg.failure_message || null;
  }
  if (!managedRuntime.whisper_cli.available) {
    return managedRuntime.whisper_cli.failure_message || null;
  }
  if (!managedRuntime.whisper_stream.available) {
    return managedRuntime.whisper_stream.failure_message || null;
  }

  return null;
}

function getManagedRuntime(
  runtimeHealth: RuntimeHealth,
): RuntimeHealth["managed_runtime"] {
  const fallbackReady =
    runtimeHealth.ffmpeg_available &&
    runtimeHealth.whisper_cli_available &&
    runtimeHealth.whisper_stream_available;

  return (
    runtimeHealth.managed_runtime ?? {
      source: runtimeHealth.runtime_source || "unknown",
      ready: fallbackReady,
      ffmpeg: {
        resolved_path:
          runtimeHealth.ffmpeg_resolved || runtimeHealth.ffmpeg_path,
        available: runtimeHealth.ffmpeg_available,
        failure_reason: "",
        failure_message: "",
      },
      whisper_cli: {
        resolved_path:
          runtimeHealth.whisper_cli_resolved || runtimeHealth.whisper_cli_path,
        available: runtimeHealth.whisper_cli_available,
        failure_reason: "",
        failure_message: "",
      },
      whisper_stream: {
        resolved_path:
          runtimeHealth.whisper_stream_resolved ||
          runtimeHealth.whisper_stream_path,
        available: runtimeHealth.whisper_stream_available,
        failure_reason: "",
        failure_message: "",
      },
    }
  );
}
