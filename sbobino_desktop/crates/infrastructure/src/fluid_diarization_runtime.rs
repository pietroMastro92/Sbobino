//! Selection and integrity checks for the bundled FluidAudio offline runtime.
//!
//! This module deliberately does not download, execute, or mutate anything.
//! The Swift helper is started by `adapters::fluid_diarization`; this module
//! only decides whether a verified, target-compatible bundle may be selected.

use std::{
    env,
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

#[cfg(target_os = "macos")]
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use sbobino_domain::settings::SpeakerDiarizationBackend;

pub const FLUID_RESOURCE_DIR_NAME: &str = "fluid-diarizer";
pub const FLUID_HELPER_RELATIVE_PATH: &str = "bin/sbobino-diarizer";
pub const FLUID_MODELS_RELATIVE_PATH: &str = "models";
pub const FLUID_SPEAKER_MODEL_RELATIVE_PATH: &str = "models/speaker-diarization";
pub const FLUID_MANIFEST_RELATIVE_PATH: &str = "manifest.json";
pub const FLUID_REVISION_FILENAME: &str = ".fluidaudio-revision";
pub const FLUID_MANIFEST_VERSION: u8 = 1;
pub const FLUID_MINIMUM_MACOS_MAJOR: u32 = 15;
pub const FLUID_EXPECTED_SDK_REVISION: &str = "793a2c5a0e99705a55fefbad00e65ca684329349";
pub const FLUID_EXPECTED_MODEL_REVISION: &str = "df2625ac79a7ac6b65ad868fee6d80f320da4232";

pub const REQUIRED_MODEL_ARTIFACTS: [&str; 5] = [
    "Segmentation.mlmodelc/coremldata.bin",
    "FBank.mlmodelc/coremldata.bin",
    "Embedding.mlmodelc/coremldata.bin",
    "PldaRho.mlmodelc/coremldata.bin",
    "plda-parameters.json",
];

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPlatform {
    pub os: String,
    pub os_major: Option<u32>,
    pub arch: String,
}

impl HostPlatform {
    pub fn current() -> Self {
        Self {
            os: env::consts::OS.to_string(),
            os_major: current_macos_major(),
            arch: env::consts::ARCH.to_string(),
        }
    }

    pub fn supports_fluid(&self) -> bool {
        self.os.eq_ignore_ascii_case("macos")
            && self.os_major.is_some_and(|major| major >= 15)
            && matches!(
                canonical_architecture(&self.arch).as_str(),
                "aarch64" | "x86_64"
            )
    }

    pub fn is_apple_silicon(&self) -> bool {
        canonical_architecture(&self.arch) == "aarch64"
    }

    pub fn canonical_arch(&self) -> String {
        canonical_architecture(&self.arch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FluidCompute {
    Cpu,
    Ane,
}

impl FluidCompute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Ane => "ane",
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FluidDiarizationHealth {
    pub supported: bool,
    pub ready: bool,
    pub manifest_version: u8,
    pub minimum_macos_major: u32,
    pub helper_installed: bool,
    pub models_installed: bool,
    pub manifest_verified: bool,
    pub helper_path: String,
    pub models_dir: String,
    pub manifest_path: String,
    pub architecture: String,
    pub sdk_revision: String,
    pub model_revision: String,
    pub missing_artifacts: Vec<String>,
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelectedSpeakerDiarizationHealth {
    pub enabled: bool,
    pub requested_backend: SpeakerDiarizationBackend,
    pub selected_backend: Option<SpeakerDiarizationBackend>,
    pub compute: Option<String>,
    pub ready: bool,
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSelection {
    Fluid(FluidCompute),
    Pyannote,
}

#[derive(Debug, Clone)]
pub struct VerifiedFluidBundle {
    pub helper_path: PathBuf,
    pub models_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct FluidBundleManifest {
    version: u8,
    minimum_macos_major: u32,
    sdk_revision: String,
    model_revision: String,
    architecture: String,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug)]
struct ValidatedManifest {
    manifest: FluidBundleManifest,
    helper_path: PathBuf,
    models_dir: PathBuf,
}

pub fn fluid_bundle_health(
    bundle_resources_dir: Option<&Path>,
    host: &HostPlatform,
) -> FluidDiarizationHealth {
    let root = bundle_resources_dir
        .map(|path| path.join(FLUID_RESOURCE_DIR_NAME))
        .unwrap_or_default();
    let helper_path = root.join(FLUID_HELPER_RELATIVE_PATH);
    let models_dir = root.join(FLUID_MODELS_RELATIVE_PATH);
    let manifest_path = root.join(FLUID_MANIFEST_RELATIVE_PATH);

    let mut health = FluidDiarizationHealth {
        supported: host.supports_fluid(),
        manifest_version: FLUID_MANIFEST_VERSION,
        minimum_macos_major: FLUID_MINIMUM_MACOS_MAJOR,
        helper_path: helper_path.to_string_lossy().to_string(),
        models_dir: models_dir.to_string_lossy().to_string(),
        manifest_path: manifest_path.to_string_lossy().to_string(),
        architecture: host.canonical_arch(),
        ..Default::default()
    };

    if !health.supported {
        health.reason_code = "fluid_unsupported_platform".to_string();
        health.message = format!(
            "FluidAudio requires macOS 15+; current host is {} {} ({}).",
            host.os,
            host.os_major
                .map(|major| major.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            host.arch
        );
        return health;
    }

    if bundle_resources_dir.is_none() {
        health.reason_code = "fluid_bundle_missing".to_string();
        health.message = "FluidAudio bundle resources are not installed.".to_string();
        health.helper_path.clear();
        health.models_dir.clear();
        health.manifest_path.clear();
        return health;
    }

    health.helper_installed = is_regular_executable(&helper_path);
    if !health.helper_installed {
        health.reason_code = "fluid_helper_missing".to_string();
        health.message = format!(
            "FluidAudio helper is missing or not executable at '{}'.",
            helper_path.display()
        );
        return health;
    }

    let validated = match validate_bundle(&root, host) {
        Ok(bundle) => bundle,
        Err(error) => {
            health.reason_code = error.reason_code;
            health.message = error.message;
            health.missing_artifacts = error.missing_artifacts;
            return health;
        }
    };

    health.models_installed = true;
    health.manifest_verified = true;
    health.ready = true;
    health.sdk_revision = validated.manifest.sdk_revision;
    health.model_revision = validated.manifest.model_revision;
    health.reason_code = "ok".to_string();
    health.message = "FluidAudio offline diarization resources are ready.".to_string();
    health
}

pub fn verified_fluid_bundle(
    bundle_resources_dir: Option<&Path>,
    host: &HostPlatform,
) -> Result<VerifiedFluidBundle, String> {
    if !host.supports_fluid() {
        return Err(format!(
            "FluidAudio is unavailable on {} {} (requires macOS 15+).",
            host.os,
            host.os_major
                .map(|major| major.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    let root = bundle_resources_dir
        .map(|path| path.join(FLUID_RESOURCE_DIR_NAME))
        .ok_or_else(|| "FluidAudio bundle resources are not installed.".to_string())?;
    let bundle = validate_bundle(&root, host).map_err(|error| error.message)?;
    Ok(VerifiedFluidBundle {
        helper_path: bundle.helper_path,
        models_dir: bundle.models_dir,
    })
}

pub fn native_compute_for_device(
    device: &str,
    host: &HostPlatform,
) -> Result<Option<FluidCompute>, String> {
    match device.trim().to_ascii_lowercase().as_str() {
        "cpu" => Ok(Some(FluidCompute::Cpu)),
        "ane" => {
            if !host.is_apple_silicon() {
                Err("FluidAudio ANE compute requires Apple Silicon; use cpu.".to_string())
            } else {
                Ok(Some(FluidCompute::Ane))
            }
        }
        "auto" => Ok(Some(if host.is_apple_silicon() {
            FluidCompute::Ane
        } else {
            FluidCompute::Cpu
        })),
        // These are valid Pyannote preferences but are not FluidAudio compute
        // modes.  Returning None is intentional: never reinterpret MPS/CUDA
        // as ANE.
        "mps" | "cuda" | "gpu" => Ok(None),
        "" => Ok(Some(FluidCompute::Cpu)),
        _ => Ok(None),
    }
}

pub fn choose_backend(
    enabled: bool,
    requested: SpeakerDiarizationBackend,
    device: &str,
    fluid: &FluidDiarizationHealth,
    pyannote_ready: bool,
    pyannote_message: &str,
    host: &HostPlatform,
) -> Result<BackendSelection, String> {
    if !enabled {
        return Err("Speaker diarization is disabled in transcription settings.".to_string());
    }

    match requested {
        SpeakerDiarizationBackend::Pyannote => {
            if device.trim().eq_ignore_ascii_case("ane") {
                return Err(
                    "Pyannote does not support Neural Engine; use cpu, auto, mps, or cuda."
                        .to_string(),
                );
            }
            if pyannote_ready {
                Ok(BackendSelection::Pyannote)
            } else {
                Err(pyannote_message.to_string())
            }
        }
        SpeakerDiarizationBackend::Fluid => {
            let compute = native_compute_for_device(device, host)?.ok_or_else(|| {
                format!(
                    "FluidAudio does not support diarization device '{}'; use cpu, auto, or ane.",
                    device.trim()
                )
            })?;
            if !fluid.ready {
                return Err(fluid.message.clone());
            }
            Ok(BackendSelection::Fluid(compute))
        }
        SpeakerDiarizationBackend::Auto => {
            let compute = native_compute_for_device(device, host)?;
            if let Some(compute) = compute {
                if fluid.ready {
                    return Ok(BackendSelection::Fluid(compute));
                }
                // An explicit ANE request is a hard native requirement.  It
                // must not silently become Pyannote CPU after a native miss.
                if device.trim().eq_ignore_ascii_case("ane") {
                    return Err(fluid.message.clone());
                }
            }
            if pyannote_ready {
                Ok(BackendSelection::Pyannote)
            } else {
                Err(format!(
                    "Neither FluidAudio nor Pyannote is ready. FluidAudio: {}; Pyannote: {}",
                    fluid.message, pyannote_message
                ))
            }
        }
    }
}

pub fn selected_backend_health(
    enabled: bool,
    requested: SpeakerDiarizationBackend,
    device: &str,
    fluid: &FluidDiarizationHealth,
    pyannote_ready: bool,
    pyannote_message: &str,
    host: &HostPlatform,
) -> SelectedSpeakerDiarizationHealth {
    if !enabled {
        return SelectedSpeakerDiarizationHealth {
            enabled: false,
            requested_backend: requested,
            selected_backend: None,
            compute: None,
            ready: true,
            reason_code: "disabled".to_string(),
            message: "Speaker diarization is disabled in transcription settings.".to_string(),
        };
    }

    match choose_backend(
        enabled,
        requested,
        device,
        fluid,
        pyannote_ready,
        pyannote_message,
        host,
    ) {
        Ok(BackendSelection::Fluid(compute)) => SelectedSpeakerDiarizationHealth {
            enabled: true,
            requested_backend: requested,
            selected_backend: Some(SpeakerDiarizationBackend::Fluid),
            compute: Some(compute.as_str().to_string()),
            ready: true,
            reason_code: "ok".to_string(),
            message: "FluidAudio offline diarization is selected.".to_string(),
        },
        Ok(BackendSelection::Pyannote) => SelectedSpeakerDiarizationHealth {
            enabled: true,
            requested_backend: requested,
            selected_backend: Some(SpeakerDiarizationBackend::Pyannote),
            compute: None,
            ready: true,
            reason_code: "ok".to_string(),
            message: "Pyannote offline diarization is selected.".to_string(),
        },
        Err(error) => SelectedSpeakerDiarizationHealth {
            enabled: true,
            requested_backend: requested,
            selected_backend: None,
            compute: native_compute_for_device(device, host)
                .ok()
                .flatten()
                .map(|compute| compute.as_str().to_string()),
            ready: false,
            reason_code: selection_failure_reason(requested, device, fluid, host),
            message: error,
        },
    }
}

fn selection_failure_reason(
    requested: SpeakerDiarizationBackend,
    device: &str,
    fluid: &FluidDiarizationHealth,
    host: &HostPlatform,
) -> String {
    match requested {
        SpeakerDiarizationBackend::Pyannote => "pyannote_not_ready".to_string(),
        SpeakerDiarizationBackend::Fluid => match native_compute_for_device(device, host) {
            Err(_) | Ok(None) => "fluid_device_unsupported".to_string(),
            Ok(Some(_)) if !fluid.reason_code.is_empty() => fluid.reason_code.clone(),
            Ok(Some(_)) => "fluid_not_ready".to_string(),
        },
        SpeakerDiarizationBackend::Auto => {
            if native_compute_for_device(device, host).is_err() {
                "fluid_device_unsupported".to_string()
            } else {
                "speaker_diarization_not_ready".to_string()
            }
        }
    }
}

#[derive(Debug)]
struct ValidationError {
    reason_code: String,
    message: String,
    missing_artifacts: Vec<String>,
}

fn validate_bundle(root: &Path, host: &HostPlatform) -> Result<ValidatedManifest, ValidationError> {
    let helper_path = root.join(FLUID_HELPER_RELATIVE_PATH);
    let models_dir = root.join(FLUID_MODELS_RELATIVE_PATH);
    let model_root = root.join(FLUID_SPEAKER_MODEL_RELATIVE_PATH);
    let manifest_path = root.join(FLUID_MANIFEST_RELATIVE_PATH);

    if !is_regular_executable(&helper_path) {
        return Err(ValidationError {
            reason_code: "fluid_helper_missing".to_string(),
            message: format!(
                "FluidAudio helper is missing or not executable at '{}'.",
                helper_path.display()
            ),
            missing_artifacts: vec![FLUID_HELPER_RELATIVE_PATH.to_string()],
        });
    }

    let mut manifest_bytes = Vec::new();
    File::open(&manifest_path)
        .and_then(|file| {
            file.take(MAX_MANIFEST_BYTES + 1)
                .read_to_end(&mut manifest_bytes)
        })
        .map_err(|error| ValidationError {
            reason_code: "fluid_manifest_missing".to_string(),
            message: format!(
                "FluidAudio manifest '{}' could not be read: {error}.",
                manifest_path.display()
            ),
            missing_artifacts: vec![FLUID_MANIFEST_RELATIVE_PATH.to_string()],
        })?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: "FluidAudio manifest exceeds the bounded size limit.".to_string(),
            missing_artifacts: vec![],
        });
    }
    let manifest: FluidBundleManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|error| ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: format!("FluidAudio manifest is invalid JSON: {error}."),
            missing_artifacts: vec![],
        })?;

    if manifest.version != FLUID_MANIFEST_VERSION
        || manifest.minimum_macos_major != FLUID_MINIMUM_MACOS_MAJOR
        || manifest.sdk_revision.trim() != FLUID_EXPECTED_SDK_REVISION
        || manifest.model_revision.trim() != FLUID_EXPECTED_MODEL_REVISION
        || !is_hex_revision(&manifest.sdk_revision)
        || !is_hex_revision(&manifest.model_revision)
    {
        return Err(ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: format!(
                "FluidAudio manifest must be version {} with macOS {}, SDK {}, and model {}.",
                FLUID_MANIFEST_VERSION,
                FLUID_MINIMUM_MACOS_MAJOR,
                FLUID_EXPECTED_SDK_REVISION,
                FLUID_EXPECTED_MODEL_REVISION
            ),
            missing_artifacts: vec![],
        });
    }
    if host
        .os_major
        .is_none_or(|major| major < manifest.minimum_macos_major)
    {
        return Err(ValidationError {
            reason_code: "fluid_unsupported_platform".to_string(),
            message: format!(
                "FluidAudio requires macOS {} or newer.",
                manifest.minimum_macos_major
            ),
            missing_artifacts: vec![],
        });
    }
    if !architecture_matches(&manifest.architecture, &host.arch) {
        return Err(ValidationError {
            reason_code: "fluid_arch_mismatch".to_string(),
            message: format!(
                "FluidAudio bundle architecture '{}' does not match host '{}'.",
                manifest.architecture, host.arch
            ),
            missing_artifacts: vec![],
        });
    }
    if manifest.files.is_empty() {
        return Err(ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: "FluidAudio manifest contains no file checksums.".to_string(),
            missing_artifacts: vec![],
        });
    }

    let mut seen = std::collections::HashSet::new();
    for entry in &manifest.files {
        if !safe_relative_path(&entry.path)
            || entry.path == FLUID_MANIFEST_RELATIVE_PATH
            || !seen.insert(entry.path.as_str())
            || entry.sha256.len() != 64
            || !entry
                .sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(ValidationError {
                reason_code: "fluid_manifest_invalid".to_string(),
                message: format!(
                    "FluidAudio manifest contains an unsafe or duplicate entry '{}'.",
                    entry.path
                ),
                missing_artifacts: vec![],
            });
        }
        let path = root.join(&entry.path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) | Err(_) => {
                return Err(ValidationError {
                    reason_code: "fluid_checksum_invalid".to_string(),
                    message: format!(
                        "FluidAudio manifest file '{}' is missing or not a regular file.",
                        path.display()
                    ),
                    missing_artifacts: vec![entry.path.clone()],
                });
            }
        };
        if metadata.len() != entry.bytes
            || !sha256_matches_file(&path, &entry.sha256).unwrap_or(false)
        {
            return Err(ValidationError {
                reason_code: "fluid_checksum_invalid".to_string(),
                message: format!("FluidAudio checksum or size mismatch for '{}'.", entry.path),
                missing_artifacts: vec![entry.path.clone()],
            });
        }
    }

    let manifest_paths = seen
        .into_iter()
        .map(str::to_string)
        .collect::<std::collections::HashSet<_>>();
    let mut actual_files = std::collections::HashSet::new();
    collect_bundle_inventory(root, root, &mut actual_files)?;
    let unlisted_files = actual_files
        .difference(&manifest_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !unlisted_files.is_empty() {
        return Err(ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: "FluidAudio manifest does not cover every bundled file.".to_string(),
            missing_artifacts: unlisted_files,
        });
    }

    let mut required = Vec::with_capacity(REQUIRED_MODEL_ARTIFACTS.len() + 2);
    required.push(FLUID_HELPER_RELATIVE_PATH.to_string());
    required.push(format!(
        "{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{FLUID_REVISION_FILENAME}"
    ));
    required.extend(
        REQUIRED_MODEL_ARTIFACTS
            .iter()
            .map(|path| format!("{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{path}")),
    );
    let missing_from_manifest = required
        .iter()
        .filter(|path| !manifest_paths.contains(path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_from_manifest.is_empty() {
        return Err(ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: "FluidAudio manifest omits required helper/model files.".to_string(),
            missing_artifacts: missing_from_manifest,
        });
    }

    let revision_path = model_root.join(FLUID_REVISION_FILENAME);
    let revision = fs::read_to_string(&revision_path).map_err(|_| ValidationError {
        reason_code: "fluid_models_missing".to_string(),
        message: format!(
            "FluidAudio model revision marker is missing at '{}'.",
            revision_path.display()
        ),
        missing_artifacts: vec![format!(
            "{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{FLUID_REVISION_FILENAME}"
        )],
    })?;
    if revision.trim() != manifest.model_revision.trim() {
        return Err(ValidationError {
            reason_code: "fluid_models_revision_mismatch".to_string(),
            message: format!(
                "FluidAudio model revision marker '{}' does not match manifest '{}'.",
                revision.trim(),
                manifest.model_revision
            ),
            missing_artifacts: vec![format!(
                "{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{FLUID_REVISION_FILENAME}"
            )],
        });
    }

    let missing_models = REQUIRED_MODEL_ARTIFACTS
        .iter()
        .filter(|path| {
            let candidate = model_root.join(path);
            !candidate.is_file()
        })
        .map(|path| format!("{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{path}"))
        .collect::<Vec<_>>();
    if !missing_models.is_empty() {
        return Err(ValidationError {
            reason_code: "fluid_models_missing".to_string(),
            message: "FluidAudio offline model artifacts are incomplete.".to_string(),
            missing_artifacts: missing_models,
        });
    }

    Ok(ValidatedManifest {
        manifest,
        helper_path,
        models_dir,
    })
}

fn safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.contains('\\') || value.contains(':') {
        return false;
    }
    let path = Path::new(value);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn collect_bundle_inventory(
    root: &Path,
    directory: &Path,
    files: &mut std::collections::HashSet<String>,
) -> Result<(), ValidationError> {
    let entries = fs::read_dir(directory).map_err(|error| ValidationError {
        reason_code: "fluid_manifest_invalid".to_string(),
        message: format!(
            "Could not enumerate FluidAudio bundle '{}': {error}.",
            directory.display()
        ),
        missing_artifacts: vec![],
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: format!("Could not enumerate FluidAudio bundle: {error}."),
            missing_artifacts: vec![],
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: format!(
                "Could not inspect FluidAudio bundle entry '{}': {error}.",
                path.display()
            ),
            missing_artifacts: vec![],
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ValidationError {
                reason_code: "fluid_manifest_invalid".to_string(),
                message: format!(
                    "FluidAudio bundle must not contain symlinks: '{}'.",
                    path.display()
                ),
                missing_artifacts: vec![],
            });
        }
        if metadata.is_dir() {
            collect_bundle_inventory(root, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(ValidationError {
                reason_code: "fluid_manifest_invalid".to_string(),
                message: format!(
                    "FluidAudio bundle entry '{}' is not a regular file or directory.",
                    path.display()
                ),
                missing_artifacts: vec![],
            });
        }
        let relative = path.strip_prefix(root).map_err(|_| ValidationError {
            reason_code: "fluid_manifest_invalid".to_string(),
            message: format!(
                "FluidAudio bundle entry '{}' escaped its root.",
                path.display()
            ),
            missing_artifacts: vec![],
        })?;
        let relative = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if relative != FLUID_MANIFEST_RELATIVE_PATH {
            files.insert(relative);
        }
    }
    Ok(())
}

fn is_regular_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn sha256_matches_file(path: &Path, expected: &str) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected.trim()))
}

fn is_hex_revision(value: &str) -> bool {
    let value = value.trim();
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn canonical_architecture(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "arm64" | "aarch64" | "aarch64-apple-darwin" | "macos-aarch64" => "aarch64".to_string(),
        "x86_64" | "amd64" | "x86-64" | "x86_64-apple-darwin" | "macos-x86_64" => {
            "x86_64".to_string()
        }
        other => other.to_string(),
    }
}

fn architecture_matches(bundle: &str, host: &str) -> bool {
    let bundle = canonical_architecture(bundle);
    let host = canonical_architecture(host);
    !bundle.is_empty() && bundle == host
}

#[cfg(target_os = "macos")]
fn current_macos_major() -> Option<u32> {
    static VERSION: OnceLock<Option<u32>> = OnceLock::new();
    *VERSION.get_or_init(read_macos_major)
}

#[cfg(not(target_os = "macos"))]
fn current_macos_major() -> Option<u32> {
    None
}

#[cfg(target_os = "macos")]
fn read_macos_major() -> Option<u32> {
    const SYSTEM_VERSION_PLIST: &str = "/System/Library/CoreServices/SystemVersion.plist";
    let bytes = fs::read(SYSTEM_VERSION_PLIST).ok()?;
    if bytes.len() > 64 * 1024 {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    let after_key = text.split_once("<key>ProductVersion</key>")?.1;
    let version = after_key
        .split_once("<string>")?
        .1
        .split_once("</string>")?
        .0;
    version.split('.').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(os: &str, major: Option<u32>, arch: &str) -> HostPlatform {
        HostPlatform {
            os: os.to_string(),
            os_major: major,
            arch: arch.to_string(),
        }
    }

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn digest(path: &Path) -> (u64, String) {
        let contents = fs::read(path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        (contents.len() as u64, format!("{:x}", hasher.finalize()))
    }

    fn make_bundle(root: &Path, architecture: &str) {
        let helper = root.join(FLUID_HELPER_RELATIVE_PATH);
        write_file(&helper, b"native-helper");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&helper).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&helper, permissions).unwrap();
        }
        let model_root = root.join(FLUID_SPEAKER_MODEL_RELATIVE_PATH);
        let revision = format!("{FLUID_EXPECTED_MODEL_REVISION}\n");
        write_file(
            &model_root.join(FLUID_REVISION_FILENAME),
            revision.as_bytes(),
        );
        for artifact in REQUIRED_MODEL_ARTIFACTS {
            write_file(&model_root.join(artifact), artifact.as_bytes());
        }

        let mut files = Vec::new();
        let required = std::iter::once(FLUID_HELPER_RELATIVE_PATH.to_string())
            .chain(std::iter::once(format!(
                "{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{FLUID_REVISION_FILENAME}"
            )))
            .chain(
                REQUIRED_MODEL_ARTIFACTS
                    .iter()
                    .map(|artifact| format!("{FLUID_SPEAKER_MODEL_RELATIVE_PATH}/{artifact}")),
            )
            .collect::<Vec<_>>();
        for path in required {
            let full = root.join(&path);
            let (bytes, sha256) = digest(&full);
            files.push(serde_json::json!({"path":path,"bytes":bytes,"sha256":sha256}));
        }
        write_file(
            &root.join(FLUID_MANIFEST_RELATIVE_PATH),
            serde_json::to_vec(&serde_json::json!({
                "version":1,
                "minimum_macos_major":15,
                "sdk_revision":FLUID_EXPECTED_SDK_REVISION,
                "model_revision":FLUID_EXPECTED_MODEL_REVISION,
                "architecture":architecture,
                "files":files
            }))
            .unwrap()
            .as_slice(),
        );
    }

    #[test]
    fn healthy_bundle_requires_manifest_marker_and_all_five_artifacts() {
        let temporary = tempfile::tempdir().unwrap();
        make_bundle(
            temporary.path().join(FLUID_RESOURCE_DIR_NAME).as_path(),
            "aarch64",
        );
        let health = fluid_bundle_health(Some(temporary.path()), &host("macos", Some(15), "arm64"));
        assert!(health.ready, "{health:?}");
        assert!(health.manifest_verified);
        assert_eq!(health.model_revision.len(), 40);
    }

    #[test]
    fn missing_resources_has_no_relative_fallback() {
        let health = fluid_bundle_health(None, &host("macos", Some(15), "aarch64"));
        assert_eq!(health.reason_code, "fluid_bundle_missing");
        assert!(!health.ready);
        assert!(health.helper_path.is_empty());
        assert!(verified_fluid_bundle(None, &host("macos", Some(15), "aarch64")).is_err());
    }

    #[test]
    fn oversized_manifest_is_rejected() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join(FLUID_RESOURCE_DIR_NAME);
        make_bundle(&root, "aarch64");
        File::create(root.join(FLUID_MANIFEST_RELATIVE_PATH))
            .unwrap()
            .set_len(MAX_MANIFEST_BYTES + 1)
            .unwrap();
        let health =
            fluid_bundle_health(Some(temporary.path()), &host("macos", Some(15), "aarch64"));
        assert_eq!(health.reason_code, "fluid_manifest_invalid");
        assert!(!health.ready);
    }

    #[test]
    fn failed_selection_does_not_claim_an_effective_backend() {
        let health = selected_backend_health(
            true,
            SpeakerDiarizationBackend::Auto,
            "cpu",
            &FluidDiarizationHealth::default(),
            false,
            "unavailable",
            &host("macos", Some(15), "aarch64"),
        );
        assert!(!health.ready);
        assert_eq!(health.selected_backend, None);
        assert_eq!(health.requested_backend, SpeakerDiarizationBackend::Auto);
    }

    #[test]
    fn unsupported_os_is_rejected_before_bundle_use() {
        let temporary = tempfile::tempdir().unwrap();
        make_bundle(
            temporary.path().join(FLUID_RESOURCE_DIR_NAME).as_path(),
            "x86_64",
        );
        for major in [13, 14] {
            let health = fluid_bundle_health(
                Some(temporary.path()),
                &host("macos", Some(major), "x86_64"),
            );
            assert!(!health.ready);
            assert_eq!(health.reason_code, "fluid_unsupported_platform");
        }
    }

    #[test]
    fn missing_or_corrupt_cache_is_never_reported_ready() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join(FLUID_RESOURCE_DIR_NAME);
        make_bundle(&root, "x86_64");
        fs::remove_file(
            root.join(FLUID_SPEAKER_MODEL_RELATIVE_PATH)
                .join("FBank.mlmodelc/coremldata.bin"),
        )
        .unwrap();
        let health =
            fluid_bundle_health(Some(temporary.path()), &host("macos", Some(15), "x86_64"));
        assert!(!health.ready);
        assert!(matches!(
            health.reason_code.as_str(),
            "fluid_checksum_invalid" | "fluid_manifest_invalid"
        ));
    }

    #[test]
    fn architecture_mismatch_is_diagnostic() {
        let temporary = tempfile::tempdir().unwrap();
        make_bundle(
            temporary.path().join(FLUID_RESOURCE_DIR_NAME).as_path(),
            "x86_64",
        );
        let health =
            fluid_bundle_health(Some(temporary.path()), &host("macos", Some(15), "aarch64"));
        assert_eq!(health.reason_code, "fluid_arch_mismatch");
        assert!(!health.ready);
    }

    #[test]
    fn device_selection_never_reinterprets_mps_or_cuda_as_ane() {
        let apple = host("macos", Some(15), "aarch64");
        assert_eq!(
            native_compute_for_device("cpu", &apple),
            Ok(Some(FluidCompute::Cpu))
        );
        assert_eq!(
            native_compute_for_device("auto", &apple),
            Ok(Some(FluidCompute::Ane))
        );
        assert_eq!(native_compute_for_device("mps", &apple), Ok(None));
        assert_eq!(native_compute_for_device("cuda", &apple), Ok(None));
        assert!(native_compute_for_device("ane", &host("macos", Some(15), "x86_64")).is_err());
    }

    #[test]
    fn auto_ane_does_not_fallback_to_pyannote_when_native_is_missing() {
        let host = host("macos", Some(15), "aarch64");
        let fluid = FluidDiarizationHealth {
            ready: false,
            reason_code: "fluid_models_missing".to_string(),
            message: "models missing".to_string(),
            ..Default::default()
        };
        let selection = choose_backend(
            true,
            SpeakerDiarizationBackend::Auto,
            "ane",
            &fluid,
            true,
            "Pyannote ready",
            &host,
        );
        assert!(selection.is_err());
    }

    #[test]
    fn auto_mps_preserves_pyannote_choice() {
        let host = host("macos", Some(15), "aarch64");
        let fluid = FluidDiarizationHealth {
            ready: true,
            ..Default::default()
        };
        assert_eq!(
            choose_backend(
                true,
                SpeakerDiarizationBackend::Auto,
                "mps",
                &fluid,
                true,
                "Pyannote ready",
                &host,
            ),
            Ok(BackendSelection::Pyannote)
        );
    }

    #[test]
    fn explicit_pyannote_rejects_persisted_ane_preference() {
        let host = host("macos", Some(15), "aarch64");
        assert!(choose_backend(
            true,
            SpeakerDiarizationBackend::Pyannote,
            "ane",
            &FluidDiarizationHealth::default(),
            true,
            "ready",
            &host
        )
        .is_err());
    }

    #[test]
    fn selected_health_exposes_requested_and_effective_backend() {
        let host = host("macos", Some(15), "x86_64");
        let fluid = FluidDiarizationHealth {
            ready: true,
            ..Default::default()
        };
        let health = selected_backend_health(
            true,
            SpeakerDiarizationBackend::Auto,
            "cpu",
            &fluid,
            false,
            "Pyannote unavailable",
            &host,
        );
        assert_eq!(
            health.selected_backend,
            Some(SpeakerDiarizationBackend::Fluid)
        );
        assert_eq!(health.compute.as_deref(), Some("cpu"));
        assert!(health.ready);
    }
}
