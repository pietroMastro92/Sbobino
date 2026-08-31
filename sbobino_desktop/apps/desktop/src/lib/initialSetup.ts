import type {
  ParakeetModel,
  ProvisioningModelCatalogEntry,
  PyannoteRuntimeHealth,
  RuntimeHealth,
  SpeechModel,
  TranscriptionEngine,
} from "../types";

export const INITIAL_SETUP_REQUIRED_MODELS: SpeechModel[] = [
  "base",
  "large_turbo",
];
export const INITIAL_SETUP_REQUIRED_PARAKEET_MODELS: ParakeetModel[] = [
  "tdt06b_v3_q4",
  "nemotron35_asr_streaming_06b_q4",
];
export const INITIAL_SETUP_REQUIRES_PYANNOTE = false;

export type InitialSetupStepId =
  | "privacy"
  | "speech-runtime"
  | "pyannote-runtime"
  | "whisper-models"
  | "parakeet-models"
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
  final_reason_code: string | null;
  final_error: string | null;
  runtime_health: RuntimeHealth | null;
  steps: InitialSetupReportStep[];
  updated_at: string;
  trusted_for_fast_start?: boolean;
};

const PYANNOTE_REPAIR_REASON_CODES = new Set([
  "pyannote_arch_mismatch",
  "pyannote_version_mismatch",
  "pyannote_repair_required",
  "pyannote_validation_required",
  "pyannote_install_incomplete",
  "pyannote_checksum_invalid",
  "pyannote_receipt_required",
  "pyannote_receipt_invalid",
  "pyannote_import_load_failed",
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
  model: SpeechModel | ParakeetModel,
): ProvisioningModelCatalogEntry | undefined {
  return modelCatalog.find((entry) => entry.key === model);
}

export function getInitialSetupMissingModels(
  modelCatalog: ProvisioningModelCatalogEntry[],
  requireCoreml: boolean,
  engine?: TranscriptionEngine,
): Array<SpeechModel | ParakeetModel> {
  const includeWhisper = !engine || engine === "whisper_cpp";
  const includeParakeet = !engine || engine === "parakeet_cpp";

  const missing: Array<SpeechModel | ParakeetModel> = [];
  if (includeWhisper) {
    for (const model of INITIAL_SETUP_REQUIRED_MODELS) {
      if (
        !isProvisionedModelReady(
          findProvisioningModelEntry(modelCatalog, model),
          requireCoreml,
        )
      ) {
        missing.push(model);
      }
    }
  }
  if (includeParakeet) {
    for (const model of INITIAL_SETUP_REQUIRED_PARAKEET_MODELS) {
      if (
        !isProvisionedModelReady(
          findProvisioningModelEntry(modelCatalog, model),
          false,
        )
      ) {
        missing.push(model);
      }
    }
  }
  return missing;
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
    getInitialSetupMissingModels(
      modelCatalog,
      runtimeHealth.is_apple_silicon,
      runtimeHealth.configured_engine,
    ).length === 0;

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

  return (
    report.setup_complete &&
    !report.final_error &&
    report.final_reason_code === "setup_complete"
  );
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
  // `managed_runtime.ready` is aggregate health for every bundled binary. The
  // startup gate is engine-aware, so a missing Whisper binary must not block a
  // Parakeet user, and vice versa.
  if (runtimeHealth.managed_runtime_required) {
    return engineBinaryRequirements(managedRuntime, runtimeHealth).every(
      (binary) => binary.available,
    );
  }

  return engineFlatRequirements(runtimeHealth).every((available) => available);
}

export function getRuntimeToolchainFailureMessage(
  runtimeHealth: RuntimeHealth | null | undefined,
): string | null {
  if (!runtimeHealth) {
    return null;
  }

  const managedRuntime = getManagedRuntime(runtimeHealth);
  // For the managed path we surface the first engine-relevant binary that is
  // explicitly unavailable (carrying its own failure_message). A generic
  // `ready = false` without a specific missing binary returns null so the UI
  // falls back to the engine-neutral "setup incomplete" messaging.
  if (runtimeHealth.managed_runtime_required) {
    const missing = engineBinaryRequirements(managedRuntime, runtimeHealth).find(
      (binary) => !binary.available,
    );
    return missing?.failure_message || null;
  }

  if (!runtimeHealth.ffmpeg_available) {
    return managedRuntime.ffmpeg.failure_message || null;
  }
  if (runtimeHealth.configured_engine === "parakeet_cpp") {
    if (!runtimeHealth.parakeet_cli_available) {
      return managedRuntime.parakeet_cli.failure_message || null;
    }
    return null;
  }
  if (!runtimeHealth.whisper_cli_available) {
    return managedRuntime.whisper_cli.failure_message || null;
  }
  if (!runtimeHealth.whisper_stream_available) {
    return managedRuntime.whisper_stream.failure_message || null;
  }

  return null;
}

type RuntimeBinary = {
  available: boolean;
  failure_message: string;
};

// Binaries that are hard requirements for the *configured* engine, in priority
// order (used to pick the most relevant failure message). ffmpeg is always
// required; the speech CLI depends on the active engine.
function engineBinaryRequirements(
  managedRuntime: RuntimeHealth["managed_runtime"],
  runtimeHealth: RuntimeHealth,
): RuntimeBinary[] {
  const binaries: RuntimeBinary[] = [
    {
      available: managedRuntime.ffmpeg.available,
      failure_message: managedRuntime.ffmpeg.failure_message,
    },
  ];
  if (runtimeHealth.configured_engine === "parakeet_cpp") {
    binaries.push({
      available: managedRuntime.parakeet_cli.available,
      failure_message: managedRuntime.parakeet_cli.failure_message,
    });
    // Setup reports written before the worker health field was introduced
    // remain readable, but they cannot claim Parakeet readiness until the
    // worker has been observed. New backend responses always include it.
    binaries.push({
      available: managedRuntime.parakeet_worker?.available ?? false,
      failure_message:
        managedRuntime.parakeet_worker?.failure_message ||
        "Parakeet batch worker health is unavailable; repair the local runtime.",
    });
  } else {
    binaries.push(
      {
        available: managedRuntime.whisper_cli.available,
        failure_message: managedRuntime.whisper_cli.failure_message,
      },
      {
        available: managedRuntime.whisper_stream.available,
        failure_message: managedRuntime.whisper_stream.failure_message,
      },
    );
  }
  return binaries;
}

function engineFlatRequirements(runtimeHealth: RuntimeHealth): boolean[] {
  if (runtimeHealth.configured_engine === "parakeet_cpp") {
    return [
      runtimeHealth.ffmpeg_available,
      runtimeHealth.parakeet_cli_available,
    ];
  }
  return [
    runtimeHealth.ffmpeg_available,
    runtimeHealth.whisper_cli_available,
    runtimeHealth.whisper_stream_available,
  ];
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
      parakeet_cli: {
        resolved_path:
          runtimeHealth.parakeet_cli_resolved ||
          runtimeHealth.parakeet_cli_path,
        available: runtimeHealth.parakeet_cli_available,
        failure_reason: "",
        failure_message: "",
      },
      parakeet_worker: undefined,
    }
  );
}
