use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::warn;

use sbobino_domain::{
    LanguageCode, ParakeetModel, SpeechModel, TranscriptionComputeDevice, TranscriptionEngine,
};
use sbobino_infrastructure::{
    background_process::tokio_background_command, ManagedRuntimeHealth, PyannoteRuntimeHealth,
};
use tokio_util::sync::CancellationToken;

use crate::commands::realtime::{
    parakeet_live_target_lang, resolve_parakeet_live_library_path, select_parakeet_live_model,
};
use crate::parakeet_realtime::ParakeetRealtimeEngine;
use crate::realtime_audio::probe_input_device_name;
use crate::{error::CommandError, state::AppState};

const DEEP_RUNTIME_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const DEEP_RUNTIME_PROBE_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
static DEEP_RUNTIME_PROBE_COUNTER: AtomicU64 = AtomicU64::new(0);
static DEEP_RUNTIME_PROBE_CACHE: OnceLock<Mutex<HashMap<String, CachedRuntimeProbe>>> =
    OnceLock::new();
static PYANNOTE_PROBE_IN_FLIGHT: OnceLock<Mutex<bool>> = OnceLock::new();

#[derive(Debug, Clone)]
struct RuntimeProbeFailure {
    reason_code: String,
    message: String,
}

#[derive(Debug, Clone)]
struct CachedRuntimeProbe {
    checked_at: Instant,
    result: Result<(), RuntimeProbeFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeHealthResponse {
    pub app_version: String,
    pub host_os: String,
    pub host_arch: String,
    pub is_apple_silicon: bool,
    pub preferred_engine: String,
    pub configured_engine: String,
    pub runtime_source: String,
    pub managed_runtime_required: bool,
    pub managed_runtime: ManagedRuntimeHealth,
    pub ffmpeg_path: String,
    pub ffmpeg_resolved: String,
    pub ffmpeg_available: bool,
    pub whisper_cli_path: String,
    pub whisper_cli_resolved: String,
    pub whisper_cli_available: bool,
    pub whisper_stream_path: String,
    pub whisper_stream_resolved: String,
    pub whisper_stream_available: bool,
    pub parakeet_cli_path: String,
    pub parakeet_cli_resolved: String,
    pub parakeet_cli_available: bool,
    pub models_dir_configured: String,
    pub models_dir_resolved: String,
    pub parakeet_models_dir_configured: String,
    pub parakeet_models_dir_resolved: String,
    pub model_filename: String,
    pub model_present: bool,
    pub parakeet_model_filename: String,
    pub parakeet_model_present: bool,
    pub missing_parakeet_models: Vec<String>,
    pub coreml_encoder_present: bool,
    pub missing_models: Vec<String>,
    pub missing_encoders: Vec<String>,
    pub pyannote: PyannoteRuntimeHealth,
    pub setup_complete: bool,
}

#[derive(Debug, Deserialize)]
pub struct StartPreflightPayload {
    #[serde(default)]
    pub engine: Option<TranscriptionEngine>,
    pub model: SpeechModel,
    #[serde(default)]
    pub parakeet_model: Option<ParakeetModel>,
    #[serde(default)]
    pub language: Option<LanguageCode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartPreflightResponse {
    pub allowed: bool,
    pub reason_code: String,
    pub message: String,
    pub engine: String,
    pub model_filename: String,
    pub model_path: String,
    pub whisper_cli_resolved: String,
    pub whisper_stream_resolved: String,
    pub parakeet_cli_resolved: String,
    pub pyannote: PyannoteRuntimeHealth,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStartReadinessResponse {
    pub allowed: bool,
    pub reason_code: String,
    pub message: String,
    pub engine: String,
    pub model_filename: String,
    pub model_path: String,
    pub ffmpeg_resolved: String,
    pub whisper_stream_resolved: String,
    pub parakeet_cli_resolved: String,
    pub input_device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnsureRuntimeResponse {
    pub ready: bool,
    pub engine: String,
    pub did_setup: bool,
    pub message: String,
    pub ffmpeg_resolved: String,
    pub whisper_cli_resolved: String,
    pub whisper_stream_resolved: String,
    pub parakeet_cli_resolved: String,
}

fn engine_to_wire(engine: &TranscriptionEngine) -> &'static str {
    match engine {
        TranscriptionEngine::WhisperCpp => "whisper_cpp",
        TranscriptionEngine::ParakeetCpp => "parakeet_cpp",
    }
}

fn recover_interrupted_runtime_install_for_slot(
    slot: &std::sync::Arc<tokio::sync::Mutex<Option<CancellationToken>>>,
    data_dir: &Path,
) -> Result<(), CommandError> {
    // Provisioning owns the journal/stage/backup while an install is active.
    // Readiness commands can run concurrently with the download worker, so
    // never let them recover (and delete) an in-flight transaction.  The
    // worker performs its own serialized recovery/finalization under the
    // process-wide runtime transaction lock.
    let _slot_guard = match slot.try_lock() {
        Ok(active) if active.is_none() => active,
        // A contended slot is deliberately treated as active/unknown.  Keep
        // the guard alive through the synchronous recovery below: checking
        // `None` and then dropping it would leave a TOCTOU window in which a
        // provisioning worker could acquire the slot before recovery starts.
        Ok(_) | Err(_) => return Ok(()),
    };
    crate::commands::provisioning::recover_interrupted_runtime_install(data_dir)
        .map_err(|error| CommandError::new("runtime_install_recovery", error))
}

fn recover_interrupted_runtime_install(state: &AppState) -> Result<(), CommandError> {
    recover_interrupted_runtime_install_for_slot(
        &state.provisioning.cancel_token,
        state.runtime_factory.data_dir(),
    )
}

fn runtime_probe_cache() -> &'static Mutex<HashMap<String, CachedRuntimeProbe>> {
    DEEP_RUNTIME_PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn schedule_optional_pyannote_probe(
    state: &AppState,
    health: &sbobino_infrastructure::RuntimeHealth,
) {
    if !health.pyannote.enabled
        || !health.pyannote.runtime_installed
        || !health.pyannote.model_installed
    {
        return;
    }

    let in_flight = PYANNOTE_PROBE_IN_FLIGHT.get_or_init(|| Mutex::new(false));
    let Ok(mut active) = in_flight.lock() else {
        return;
    };
    if *active {
        return;
    }
    *active = true;
    drop(active);

    let runtime_factory = state.runtime_factory.clone();
    let in_flight = PYANNOTE_PROBE_IN_FLIGHT
        .get()
        .expect("pyannote probe gate initialized");
    tauri::async_runtime::spawn(async move {
        let result = crate::commands::provisioning::probe_pyannote_import_and_load(
            &runtime_factory,
            &CancellationToken::new(),
        )
        .await;
        match result {
            Ok(()) => {
                let _ = runtime_factory.write_managed_pyannote_status(
                    "ok",
                    "Pyannote diarization runtime import/load probe passed.",
                );
            }
            Err(error) if error != "cancelled" => {
                let _ = runtime_factory
                    .write_managed_pyannote_status("pyannote_import_load_failed", &error);
                tracing::warn!("Pyannote optional readiness probe failed: {error}");
            }
            Err(_) => {}
        }
        if let Ok(mut active) = in_flight.lock() {
            *active = false;
        }
    });
}

fn runtime_probe_cache_key(
    engine: &TranscriptionEngine,
    ffmpeg_path: &Path,
    engine_path: &Path,
    model_path: &Path,
    compute_device: TranscriptionComputeDevice,
) -> String {
    fn file_stamp(path: &Path) -> String {
        let metadata = std::fs::metadata(path);
        let modified = metadata
            .as_ref()
            .ok()
            .and_then(|value| value.modified().ok())
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| value.as_nanos().to_string())
            .unwrap_or_default();
        let size = metadata.map(|value| value.len()).unwrap_or_default();
        format!("{}:{size}:{modified}", path.display())
    }

    format!(
        "{}|{}|{}|{}|{}",
        engine_to_wire(engine),
        file_stamp(ffmpeg_path),
        file_stamp(engine_path),
        file_stamp(model_path),
        compute_device_to_wire(compute_device)
    )
}

fn compute_device_to_wire(device: TranscriptionComputeDevice) -> &'static str {
    match device {
        TranscriptionComputeDevice::Auto => "auto",
        TranscriptionComputeDevice::Gpu => "gpu",
        TranscriptionComputeDevice::Cpu => "cpu",
    }
}

fn configure_probe_compute_environment(
    command: &mut tokio::process::Command,
    engine: &TranscriptionEngine,
    compute_device: TranscriptionComputeDevice,
) {
    if *engine != TranscriptionEngine::ParakeetCpp {
        return;
    }

    match compute_device {
        TranscriptionComputeDevice::Cpu => {
            command
                .env("PARAKEET_DEVICE", "cpu")
                .env_remove("SBOBINO_PARAKEET_FORCE_METAL");
        }
        TranscriptionComputeDevice::Gpu => {
            // Explicit GPU must not inherit a stale diagnostic CPU override
            // from the app environment or an earlier probe.
            command
                .env_remove("PARAKEET_DEVICE")
                .env_remove("SBOBINO_PARAKEET_FORCE_CPU");
        }
        TranscriptionComputeDevice::Auto => {}
    }
}

fn configure_probe_command(command: &mut tokio::process::Command, executable: &Path) {
    let mut path_entries = executable
        .parent()
        .map(PathBuf::from)
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(existing) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&existing));
    }
    if let Ok(path) = std::env::join_paths(path_entries) {
        command.env("PATH", path);
    }

    // Managed macOS binaries carry their dependent libraries beside the bin
    // directory.  Keep the probe identical to the production launch path.
    #[cfg(target_os = "macos")]
    if let Some(runtime_root) = executable.parent().and_then(Path::parent) {
        let lib_dir = runtime_root.join("lib");
        if lib_dir.is_dir() {
            command
                .env("DYLD_LIBRARY_PATH", &lib_dir)
                .env("DYLD_FALLBACK_LIBRARY_PATH", lib_dir);
        }
    }
}

async fn run_runtime_probe_command(
    executable: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<(), RuntimeProbeFailure> {
    run_runtime_probe_command_capture(executable, args, timeout, None)
        .await
        .map(|_| ())
}

async fn run_runtime_probe_command_capture(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    compute: Option<(&TranscriptionEngine, TranscriptionComputeDevice)>,
) -> Result<std::process::Output, RuntimeProbeFailure> {
    if !executable.is_file() {
        return Err(RuntimeProbeFailure {
            reason_code: "runtime_probe_missing".to_string(),
            message: format!(
                "Runtime probe executable is missing at '{}'.",
                executable.display()
            ),
        });
    }

    let mut command = tokio_background_command(executable);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    configure_probe_command(&mut command, executable);
    if let Some((engine, compute_device)) = compute {
        configure_probe_compute_environment(&mut command, engine, compute_device);
    }

    let child = command.spawn().map_err(|error| RuntimeProbeFailure {
        reason_code: "runtime_probe_spawn_failed".to_string(),
        message: format!(
            "Failed to start runtime probe '{}': {error}",
            executable.display()
        ),
    })?;
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| RuntimeProbeFailure {
            reason_code: "runtime_probe_timeout".to_string(),
            message: format!(
                "Runtime probe '{}' timed out after {} seconds.",
                executable.display(),
                timeout.as_secs()
            ),
        })?
        .map_err(|error| RuntimeProbeFailure {
            reason_code: "runtime_probe_wait_failed".to_string(),
            message: format!(
                "Runtime probe '{}' could not be collected: {error}",
                executable.display()
            ),
        })?;

    if output.status.success() {
        return Ok(output);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(RuntimeProbeFailure {
        reason_code: "runtime_probe_failed".to_string(),
        message: if detail.is_empty() {
            format!(
                "Runtime probe '{}' exited with status {}.",
                executable.display(),
                output.status
            )
        } else {
            format!(
                "Runtime probe '{}' failed: {}",
                executable.display(),
                detail
            )
        },
    })
}

fn write_runtime_probe_wav() -> Result<PathBuf, RuntimeProbeFailure> {
    let path = std::env::temp_dir().join(format!(
        "sbobino-runtime-probe-{}-{}.wav",
        std::process::id(),
        DEEP_RUNTIME_PROBE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::create(&path, spec).map_err(|error| RuntimeProbeFailure {
            reason_code: "runtime_probe_fixture_failed".to_string(),
            message: format!("Failed to create runtime probe audio fixture: {error}"),
        })?;
    for _ in 0..16_000 {
        writer
            .write_sample(0_i16)
            .map_err(|error| RuntimeProbeFailure {
                reason_code: "runtime_probe_fixture_failed".to_string(),
                message: format!("Failed to write runtime probe audio fixture: {error}"),
            })?;
    }
    writer.finalize().map_err(|error| RuntimeProbeFailure {
        reason_code: "runtime_probe_fixture_failed".to_string(),
        message: format!("Failed to finalize runtime probe audio fixture: {error}"),
    })?;
    Ok(path)
}

fn parakeet_batch_worker_path(cli_path: &Path) -> PathBuf {
    let file_name = if cfg!(target_os = "windows") {
        "parakeet-batch-json.exe"
    } else {
        "parakeet-batch-json"
    };
    cli_path
        .parent()
        .map(|parent| parent.join(file_name))
        .unwrap_or_else(|| PathBuf::from(file_name))
}

fn write_runtime_probe_manifest(wav_path: &Path) -> Result<PathBuf, RuntimeProbeFailure> {
    let manifest_path = wav_path.with_extension("tsv");
    std::fs::write(
        &manifest_path,
        format!("0\t0.000\t1.000\t0.000\t1.000\t{}\n", wav_path.display()),
    )
    .map_err(|error| RuntimeProbeFailure {
        reason_code: "runtime_probe_fixture_failed".to_string(),
        message: format!("Failed to create Parakeet probe manifest: {error}"),
    })?;
    Ok(manifest_path)
}

fn validate_parakeet_worker_output(
    output: &std::process::Output,
) -> Result<(), RuntimeProbeFailure> {
    let mut rows = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let value = serde_json::from_str::<serde_json::Value>(line).map_err(|error| {
            RuntimeProbeFailure {
                reason_code: "parakeetcpp_smoke_invalid_json".to_string(),
                message: format!("Parakeet short smoke emitted invalid JSON: {error}"),
            }
        })?;
        if value.get("index").and_then(serde_json::Value::as_i64) != Some(0)
            || value.get("result").is_none()
        {
            return Err(RuntimeProbeFailure {
                reason_code: "parakeetcpp_smoke_invalid_json".to_string(),
                message: "Parakeet short smoke returned incomplete chunk metadata.".to_string(),
            });
        }
        rows.push(value);
    }
    if rows.len() != 1 {
        return Err(RuntimeProbeFailure {
            reason_code: "parakeetcpp_smoke_invalid_json".to_string(),
            message: format!(
                "Parakeet short smoke expected exactly one JSON row, got {}.",
                rows.len()
            ),
        });
    }
    Ok(())
}

fn whisper_probe_args(
    model_path: &Path,
    fixture_path: &Path,
    compute_device: TranscriptionComputeDevice,
) -> Vec<String> {
    let mut args = vec![
        "-m".to_string(),
        model_path.to_string_lossy().to_string(),
        "-f".to_string(),
        fixture_path.to_string_lossy().to_string(),
        "-nt".to_string(),
        "-np".to_string(),
    ];
    if compute_device == TranscriptionComputeDevice::Cpu {
        args.extend(["-ng".to_string(), "-nfa".to_string()]);
    }
    args
}

fn parakeet_probe_args(model_path: &Path, manifest_path: &Path) -> Vec<String> {
    vec![
        "--model".to_string(),
        model_path.to_string_lossy().to_string(),
        "--manifest".to_string(),
        manifest_path.to_string_lossy().to_string(),
        "--lang".to_string(),
        "auto".to_string(),
    ]
}

async fn run_deep_runtime_probe(
    engine: &TranscriptionEngine,
    ffmpeg_path: &Path,
    engine_path: &Path,
    model_path: &Path,
    compute_device: TranscriptionComputeDevice,
) -> Result<(), RuntimeProbeFailure> {
    let cache_key =
        runtime_probe_cache_key(engine, ffmpeg_path, engine_path, model_path, compute_device);
    if let Ok(cache) = runtime_probe_cache().lock() {
        if let Some(cached) = cache.get(&cache_key) {
            if cached.checked_at.elapsed() < DEEP_RUNTIME_PROBE_CACHE_TTL {
                return cached.result.clone();
            }
        }
    }

    let fixture = write_runtime_probe_wav()?;
    let mut manifest = None;
    let result = async {
        run_runtime_probe_command(
            ffmpeg_path,
            &[
                "-hide_banner".to_string(),
                "-loglevel".to_string(),
                "error".to_string(),
                "-i".to_string(),
                fixture.to_string_lossy().to_string(),
                "-f".to_string(),
                "null".to_string(),
                "-".to_string(),
            ],
            DEEP_RUNTIME_PROBE_TIMEOUT,
        )
        .await
        .map_err(|mut error| {
            error.reason_code = "ffmpeg_decode_failed".to_string();
            error.message = format!("FFmpeg decode readiness failed: {}", error.message);
            error
        })?;

        let (probe_executable, args) = match engine {
            TranscriptionEngine::WhisperCpp => (
                engine_path.to_path_buf(),
                whisper_probe_args(model_path, &fixture, compute_device),
            ),
            TranscriptionEngine::ParakeetCpp => {
                let worker = parakeet_batch_worker_path(engine_path);
                let probe_manifest = write_runtime_probe_manifest(&fixture)?;
                manifest = Some(probe_manifest.clone());
                (worker, parakeet_probe_args(model_path, &probe_manifest))
            }
        };
        let output = run_runtime_probe_command_capture(
            &probe_executable,
            &args,
            DEEP_RUNTIME_PROBE_TIMEOUT,
            Some((engine, compute_device)),
        )
        .await
        .map_err(|mut error| {
            error.reason_code = if matches!(
                (engine, error.reason_code.as_str()),
                (TranscriptionEngine::ParakeetCpp, "runtime_probe_missing")
            ) {
                "parakeet_worker_missing".to_string()
            } else {
                match engine {
                    TranscriptionEngine::WhisperCpp => "whispercpp_smoke_failed",
                    TranscriptionEngine::ParakeetCpp => "parakeetcpp_smoke_failed",
                }
                .to_string()
            };
            error.message = format!(
                "{} short transcription readiness failed: {}",
                match engine {
                    TranscriptionEngine::WhisperCpp => "Whisper.cpp",
                    TranscriptionEngine::ParakeetCpp => "Parakeet.cpp",
                },
                error.message
            );
            error
        })?;
        if matches!(engine, TranscriptionEngine::ParakeetCpp) {
            validate_parakeet_worker_output(&output)?;
        }
        Ok(())
    }
    .await;
    let _ = tokio::fs::remove_file(&fixture).await;
    if let Some(manifest) = manifest {
        let _ = tokio::fs::remove_file(manifest).await;
    }

    if let Ok(mut cache) = runtime_probe_cache().lock() {
        cache.insert(
            cache_key,
            CachedRuntimeProbe {
                checked_at: Instant::now(),
                result: result.clone(),
            },
        );
    }
    result
}

fn runtime_toolchain_ready(health: &sbobino_infrastructure::RuntimeHealth) -> bool {
    if health.managed_runtime_required {
        // All release binaries (ffmpeg, whisper CLI, whisper stream,
        // parakeet CLI) must be present regardless of which engine is
        // selected, so the user can switch engines at runtime without
        // re-running setup. v2.0.4 commit 4d0a61c made this a single
        // check; the ff5c896 merge reverted to per-engine checks.
        return health.managed_runtime.ffmpeg.available
            && health.managed_runtime.whisper_cli.available
            && health.managed_runtime.whisper_stream.available
            && health.managed_runtime.parakeet_cli.available;
    }

    match health.configured_engine {
        TranscriptionEngine::WhisperCpp => {
            health.ffmpeg_available
                && health.whisper_cli_available
                && health.whisper_stream_available
        }
        TranscriptionEngine::ParakeetCpp => {
            health.ffmpeg_available && health.parakeet_cli_available
        }
    }
}

fn first_managed_runtime_failure(
    managed_runtime: &ManagedRuntimeHealth,
    engine: TranscriptionEngine,
) -> Option<(&'static str, &str, &str)> {
    if !managed_runtime.ffmpeg.available {
        return Some((
            "FFmpeg",
            managed_runtime.ffmpeg.resolved_path.as_str(),
            managed_runtime.ffmpeg.failure_message.as_str(),
        ));
    }
    match engine {
        TranscriptionEngine::WhisperCpp => {
            if !managed_runtime.whisper_cli.available {
                return Some((
                    "Whisper CLI",
                    managed_runtime.whisper_cli.resolved_path.as_str(),
                    managed_runtime.whisper_cli.failure_message.as_str(),
                ));
            }
            if !managed_runtime.whisper_stream.available {
                return Some((
                    "Whisper Stream",
                    managed_runtime.whisper_stream.resolved_path.as_str(),
                    managed_runtime.whisper_stream.failure_message.as_str(),
                ));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            if !managed_runtime.parakeet_cli.available {
                return Some((
                    "Parakeet CLI",
                    managed_runtime.parakeet_cli.resolved_path.as_str(),
                    managed_runtime.parakeet_cli.failure_message.as_str(),
                ));
            }
        }
    }
    None
}

fn runtime_health_response(health: sbobino_infrastructure::RuntimeHealth) -> RuntimeHealthResponse {
    RuntimeHealthResponse {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        host_os: health.host_os,
        host_arch: health.host_arch,
        is_apple_silicon: health.is_apple_silicon,
        preferred_engine: engine_to_wire(&health.preferred_engine).to_string(),
        configured_engine: engine_to_wire(&health.configured_engine).to_string(),
        runtime_source: health.runtime_source,
        managed_runtime_required: health.managed_runtime_required,
        managed_runtime: health.managed_runtime,
        ffmpeg_path: health.ffmpeg_path,
        ffmpeg_resolved: health.ffmpeg_resolved,
        ffmpeg_available: health.ffmpeg_available,
        whisper_cli_path: health.whisper_cli_path,
        whisper_cli_resolved: health.whisper_cli_resolved,
        whisper_cli_available: health.whisper_cli_available,
        whisper_stream_path: health.whisper_stream_path,
        whisper_stream_resolved: health.whisper_stream_resolved,
        whisper_stream_available: health.whisper_stream_available,
        parakeet_cli_path: health.parakeet_cli_path,
        parakeet_cli_resolved: health.parakeet_cli_resolved,
        parakeet_cli_available: health.parakeet_cli_available,
        models_dir_configured: health.models_dir_configured,
        models_dir_resolved: health.models_dir_resolved,
        parakeet_models_dir_configured: health.parakeet_models_dir_configured,
        parakeet_models_dir_resolved: health.parakeet_models_dir_resolved,
        model_filename: health.model_filename,
        model_present: health.model_present,
        parakeet_model_filename: health.parakeet_model_filename,
        parakeet_model_present: health.parakeet_model_present,
        missing_parakeet_models: health.missing_parakeet_models,
        coreml_encoder_present: health.coreml_encoder_present,
        missing_models: health.missing_models,
        missing_encoders: health.missing_encoders,
        pyannote: health.pyannote,
        setup_complete: health.setup_complete,
    }
}

fn is_legacy_whisperkit_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains("whisperkit-cli")
}

fn runtime_toolchain_message(
    health: &sbobino_infrastructure::RuntimeHealth,
    setup_note: Option<&str>,
) -> String {
    if health.managed_runtime_required {
        if let Some((label, path, detail)) =
            first_managed_runtime_failure(&health.managed_runtime, health.configured_engine.clone())
        {
            let mut message = if detail.trim().is_empty() {
                format!("{label} is not runnable at '{path}'.")
            } else {
                format!("{label} verification failed at '{path}': {}", detail.trim())
            };
            if let Some(note) = setup_note {
                message.push(' ');
                message.push_str(note);
            }
            message.push_str(" Repair the local runtime from Settings > Local Models.");
            return message;
        }
    }

    let mut missing = Vec::new();
    if !health.ffmpeg_available {
        missing.push(format!(
            "FFmpeg is not runnable at '{}'.",
            health.ffmpeg_resolved
        ));
    }
    match health.configured_engine {
        TranscriptionEngine::WhisperCpp => {
            if !health.whisper_cli_available {
                missing.push(format!(
                    "Whisper CLI is not runnable at '{}'.",
                    health.whisper_cli_resolved
                ));
            }
            if !health.whisper_stream_available {
                missing.push(format!(
                    "Whisper Stream is not runnable at '{}'.",
                    health.whisper_stream_resolved
                ));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            if !health.parakeet_cli_available {
                missing.push(format!(
                    "Parakeet CLI is not runnable at '{}'.",
                    health.parakeet_cli_resolved
                ));
            }
        }
    }

    let mut message = if missing.is_empty() {
        "Transcription runtime unavailable.".to_string()
    } else {
        missing.join(" ")
    };
    if let Some(note) = setup_note {
        message.push(' ');
        message.push_str(note);
    }
    message.push_str(" Repair the local runtime from Settings > Local Models.");
    message
}

async fn normalize_runtime_settings_for_whisper_cpp(state: &AppState) -> (bool, Option<String>) {
    let mut did_setup = false;
    let mut setup_note = None::<String>;

    match state.settings_service.snapshot().await {
        Ok(mut settings) => {
            let mut changed = false;

            if settings.transcription.engine != TranscriptionEngine::WhisperCpp {
                settings.transcription.engine = TranscriptionEngine::WhisperCpp;
                settings.transcription_engine = TranscriptionEngine::WhisperCpp;
                changed = true;
            }

            let transcription_path = settings.transcription.whisper_cli_path.trim();
            if transcription_path.is_empty() || is_legacy_whisperkit_path(transcription_path) {
                settings.transcription.whisper_cli_path = "whisper-cli".to_string();
                changed = true;
            }

            let legacy_path = settings.whisper_cli_path.trim();
            if legacy_path.is_empty() || is_legacy_whisperkit_path(legacy_path) {
                settings.whisper_cli_path = "whisper-cli".to_string();
                changed = true;
            }

            let transcription_stream_path = settings.transcription.whisperkit_cli_path.trim();
            if transcription_stream_path.is_empty()
                || is_legacy_whisperkit_path(transcription_stream_path)
            {
                settings.transcription.whisperkit_cli_path = "whisper-stream".to_string();
                changed = true;
            }

            let legacy_stream_path = settings.whisperkit_cli_path.trim();
            if legacy_stream_path.is_empty() || is_legacy_whisperkit_path(legacy_stream_path) {
                settings.whisperkit_cli_path = "whisper-stream".to_string();
                changed = true;
            }

            if changed {
                settings.sync_sections_from_legacy();
                settings.sync_legacy_from_sections();
                match state.settings_service.update(settings).await {
                    Ok(_) => {
                        did_setup = true;
                    }
                    Err(error) => {
                        let message =
                            format!("Failed to persist whisper.cpp runtime settings: {error}");
                        warn!("{message}");
                        setup_note = Some(message);
                    }
                }
            }
        }
        Err(error) => {
            let message = format!("Failed to load settings for whisper.cpp runtime setup: {error}");
            warn!("{message}");
            setup_note = Some(message);
        }
    }

    (did_setup, setup_note)
}

#[tauri::command]
pub async fn ensure_transcription_runtime(
    state: State<'_, AppState>,
) -> Result<EnsureRuntimeResponse, CommandError> {
    recover_interrupted_runtime_install(&state)?;
    let health = state
        .runtime_factory
        .runtime_health_preflight()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    if runtime_toolchain_ready(&health) {
        return Ok(EnsureRuntimeResponse {
            ready: true,
            engine: engine_to_wire(&health.configured_engine).to_string(),
            did_setup: false,
            message: match health.configured_engine {
                TranscriptionEngine::WhisperCpp => "Whisper.cpp runtime available.".to_string(),
                TranscriptionEngine::ParakeetCpp => "Parakeet.cpp runtime available.".to_string(),
            },
            ffmpeg_resolved: health.ffmpeg_resolved,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
        });
    }

    let (did_setup, setup_note) = if health.configured_engine == TranscriptionEngine::WhisperCpp {
        normalize_runtime_settings_for_whisper_cpp(&state).await
    } else {
        (false, None)
    };

    let refreshed = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    let ready = runtime_toolchain_ready(&refreshed);
    let message = if ready {
        if did_setup {
            "Whisper.cpp runtime is ready.".to_string()
        } else if refreshed.configured_engine == TranscriptionEngine::ParakeetCpp {
            "Parakeet.cpp runtime available.".to_string()
        } else {
            "Whisper.cpp runtime available.".to_string()
        }
    } else {
        runtime_toolchain_message(&refreshed, setup_note.as_deref())
    };

    Ok(EnsureRuntimeResponse {
        ready,
        engine: engine_to_wire(&refreshed.configured_engine).to_string(),
        did_setup,
        message,
        ffmpeg_resolved: refreshed.ffmpeg_resolved,
        whisper_cli_resolved: refreshed.whisper_cli_resolved,
        whisper_stream_resolved: refreshed.whisper_stream_resolved,
        parakeet_cli_resolved: refreshed.parakeet_cli_resolved,
    })
}

#[tauri::command]
pub async fn get_realtime_start_readiness(
    state: State<'_, AppState>,
    payload: Option<StartPreflightPayload>,
) -> Result<RealtimeStartReadinessResponse, CommandError> {
    recover_interrupted_runtime_install(&state)?;
    eprintln!("[realtime-readiness] command received payload={payload:?}");

    let mut settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;
    let selected_engine = payload
        .as_ref()
        .and_then(|value| value.engine.clone())
        .unwrap_or_else(|| settings.transcription.engine.clone());
    eprintln!("[realtime-readiness] selected_engine={selected_engine:?}");

    if selected_engine == TranscriptionEngine::WhisperCpp {
        let _ = normalize_runtime_settings_for_whisper_cpp(&state).await;
        settings = state
            .runtime_factory
            .load_settings()
            .map_err(|e| CommandError::new("settings", e))?;
    }

    if selected_engine == TranscriptionEngine::ParakeetCpp {
        let selected_language = payload
            .as_ref()
            .and_then(|value| value.language.clone())
            .unwrap_or_else(|| settings.transcription.language.clone());
        let requested_parakeet_model = payload
            .as_ref()
            .and_then(|value| value.parakeet_model.clone())
            .unwrap_or_else(|| settings.transcription.parakeet_model.clone());
        let health = state
            .runtime_factory
            .runtime_health_preflight()
            .map_err(|e| CommandError::new("runtime_health", e))?;
        let models_dir = PathBuf::from(&health.parakeet_models_dir_resolved);
        let selected_parakeet_model = match select_parakeet_live_model(
            &models_dir,
            requested_parakeet_model.clone(),
            selected_language.clone(),
        ) {
            Ok(model) => model,
            Err(error) => {
                let model_filename = requested_parakeet_model.gguf_filename().to_string();
                return Ok(RealtimeStartReadinessResponse {
                    allowed: false,
                    reason_code: "parakeet_realtime_model_missing".to_string(),
                    message: error.message,
                    engine: "parakeet_cpp".to_string(),
                    model_filename: model_filename.clone(),
                    model_path: models_dir
                        .join(model_filename)
                        .to_string_lossy()
                        .to_string(),
                    ffmpeg_resolved: health.ffmpeg_resolved,
                    whisper_stream_resolved: health.whisper_stream_resolved,
                    parakeet_cli_resolved: health.parakeet_cli_resolved,
                    input_device_name: None,
                });
            }
        };
        let model_filename = selected_parakeet_model.gguf_filename().to_string();
        let model_path_buf = models_dir.join(&model_filename);
        let model_path = model_path_buf.to_string_lossy().to_string();
        let parakeet_lib =
            resolve_parakeet_live_library_path(PathBuf::from(&health.parakeet_cli_resolved));

        eprintln!(
            "[realtime-readiness] parakeet health cli_available={} cli={} models_dir={}",
            health.parakeet_cli_available,
            health.parakeet_cli_resolved,
            health.parakeet_models_dir_resolved
        );
        eprintln!(
            "[realtime-readiness] parakeet resolved lib={} model={}",
            parakeet_lib.display(),
            model_path
        );

        if !health.parakeet_cli_available {
            eprintln!("[realtime-readiness] blocked: parakeet_cli_missing");
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_cli_missing".to_string(),
                message: format!(
                    "Parakeet.cpp runtime is not available at {}.",
                    health.parakeet_cli_resolved
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        if !parakeet_lib.exists() {
            eprintln!(
                "[realtime-readiness] blocked: parakeet_live_library_missing path={}",
                parakeet_lib.display()
            );
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_live_library_missing".to_string(),
                message: format!(
                    "Parakeet.cpp live library is missing at {}. Reinstall the local runtime.",
                    parakeet_lib.display()
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        if let Err(error) =
            ParakeetRealtimeEngine::new(parakeet_lib.clone(), models_dir.clone()).validate_library()
        {
            eprintln!(
                "[realtime-readiness] blocked: parakeet_live_library_unloadable path={} error={}",
                parakeet_lib.display(),
                error
            );
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_live_library_unloadable".to_string(),
                message: format!(
                    "Parakeet.cpp live library is not loadable at {}. {}",
                    parakeet_lib.display(),
                    error
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        if !model_path_buf.exists() {
            eprintln!(
                "[realtime-readiness] blocked: parakeet_realtime_model_missing path={}",
                model_path_buf.display()
            );
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_realtime_model_missing".to_string(),
                message: format!(
                    "Parakeet.cpp live requires the selected streaming model. Download '{}' in Local Models.",
                    model_filename
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        let input_device_name = match crate::realtime_audio::probe_input_device_name() {
            Ok(name) => Some(name),
            Err(error) => {
                eprintln!(
                    "[realtime-readiness] blocked: input_device reason={} message={}",
                    error.reason_code, error.message
                );
                return Ok(RealtimeStartReadinessResponse {
                    allowed: false,
                    reason_code: error.reason_code,
                    message: error.message,
                    engine: "parakeet_cpp".to_string(),
                    model_filename,
                    model_path,
                    ffmpeg_resolved: health.ffmpeg_resolved,
                    whisper_stream_resolved: health.whisper_stream_resolved,
                    parakeet_cli_resolved: health.parakeet_cli_resolved,
                    input_device_name: None,
                });
            }
        };

        eprintln!(
            "[realtime-readiness] allowed: parakeet model={} input_device={input_device_name:?}",
            model_filename
        );
        return Ok(RealtimeStartReadinessResponse {
            allowed: true,
            reason_code: "ok".to_string(),
            message: format!(
                "Parakeet.cpp live is ready with '{}' (lang {}).",
                model_filename,
                parakeet_live_target_lang(selected_language)
            ),
            engine: "parakeet_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: health.ffmpeg_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            input_device_name,
        });
    }
    let selected_model = payload
        .as_ref()
        .map(|value| value.model.clone())
        .unwrap_or_else(|| settings.transcription.model.clone());
    let live_health = state
        .runtime_factory
        .live_start_health(selected_model.clone())
        .map_err(|e| CommandError::new("runtime_health", e))?;

    let model_filename = selected_model.ggml_filename().to_string();
    let model_path = PathBuf::from(&live_health.models_dir_resolved)
        .join(&model_filename)
        .to_string_lossy()
        .to_string();

    if !live_health.ffmpeg_available {
        return Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: "ffmpeg_missing".to_string(),
            message: format!(
                "FFmpeg is not runnable at '{}'. Repair the local runtime from Settings > Local Models.",
                live_health.ffmpeg_resolved
            ),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        });
    }

    if !live_health.whisper_stream_available {
        return Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: "whisper_stream_missing".to_string(),
            message: format!(
                "Whisper Stream is not runnable at '{}'. Repair the local runtime from Settings > Local Models.",
                live_health.whisper_stream_resolved
            ),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        });
    }

    if !live_health.model_present {
        return Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: "model_missing".to_string(),
            message: format!(
                "Model file '{}' was not found in '{}'. Download models from Settings > Local Models.",
                model_filename, live_health.models_dir_resolved
            ),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        });
    }

    match probe_input_device_name() {
        Ok(device_name) => Ok(RealtimeStartReadinessResponse {
            allowed: true,
            reason_code: "ok".to_string(),
            message: "Realtime start readiness passed.".to_string(),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: Some(device_name),
        }),
        Err(error) => Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: error.reason_code,
            message: error.message,
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        }),
    }
}

#[tauri::command]
pub async fn get_transcription_start_preflight(
    state: State<'_, AppState>,
    payload: Option<StartPreflightPayload>,
) -> Result<StartPreflightResponse, CommandError> {
    recover_interrupted_runtime_install(&state)?;
    let health = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;
    let compute_device = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?
        .transcription
        .compute_device;

    // Pyannote is optional. Probe its Python import/model load in a detached,
    // bounded task so a broken diarization environment is reported for repair
    // without delaying or blocking the ASR preflight itself.
    schedule_optional_pyannote_probe(&state, &health);

    let requested_engine = payload
        .as_ref()
        .and_then(|value| value.engine.clone())
        .unwrap_or_else(|| health.configured_engine.clone());
    let model_filename = match requested_engine {
        TranscriptionEngine::WhisperCpp => payload
            .as_ref()
            .map(|value| value.model.ggml_filename().to_string())
            .unwrap_or_else(|| health.model_filename.clone()),
        TranscriptionEngine::ParakeetCpp => payload
            .as_ref()
            .and_then(|value| value.parakeet_model.clone())
            .map(|model| model.gguf_filename().to_string())
            .unwrap_or_else(|| health.parakeet_model_filename.clone()),
    };
    let model_dir = match requested_engine {
        TranscriptionEngine::WhisperCpp => &health.models_dir_resolved,
        TranscriptionEngine::ParakeetCpp => &health.parakeet_models_dir_resolved,
    };
    let model_path = PathBuf::from(model_dir)
        .join(&model_filename)
        .to_string_lossy()
        .to_string();
    let engine_wire = engine_to_wire(&requested_engine).to_string();

    if !health.ffmpeg_available {
        let message = if health.managed_runtime_required {
            runtime_toolchain_message(&health, None)
        } else {
            format!(
                "FFmpeg is not runnable at '{}'. Configure FFmpeg path in Settings > Advanced.",
                health.ffmpeg_resolved
            )
        };
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "ffmpeg_missing".to_string(),
            message,
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    if requested_engine == TranscriptionEngine::WhisperCpp && !health.whisper_cli_available {
        let message = if health.managed_runtime_required {
            runtime_toolchain_message(&health, None)
        } else {
            format!(
                "Whisper CLI is not runnable at '{}'. Configure Whisper CLI path in Settings > Local Models.",
                health.whisper_cli_resolved
            )
        };
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "whispercpp_missing".to_string(),
            message,
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    if requested_engine == TranscriptionEngine::ParakeetCpp && !health.parakeet_cli_available {
        let message = format!(
            "Parakeet CLI is not runnable at '{}'. Repair the local runtime from Settings > Local Models.",
            health.parakeet_cli_resolved
        );
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "parakeetcpp_missing".to_string(),
            message,
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    if !PathBuf::from(&model_path).exists() {
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "model_missing".to_string(),
            message: format!(
                "Model file '{}' was not found in '{}'. Download models from Settings > Local Models.",
                model_filename, model_dir
            ),
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    let engine_path = match requested_engine {
        TranscriptionEngine::WhisperCpp => PathBuf::from(&health.whisper_cli_resolved),
        TranscriptionEngine::ParakeetCpp => PathBuf::from(&health.parakeet_cli_resolved),
    };
    if let Err(error) = run_deep_runtime_probe(
        &requested_engine,
        Path::new(&health.ffmpeg_resolved),
        &engine_path,
        Path::new(&model_path),
        compute_device,
    )
    .await
    {
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: error.reason_code,
            message: format!(
                "{} Repair the local runtime or selected model from Settings > Local Models.",
                error.message
            ),
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    Ok(StartPreflightResponse {
        allowed: true,
        reason_code: "ok".to_string(),
        message: match requested_engine {
            TranscriptionEngine::WhisperCpp => "Whisper.cpp preflight passed.".to_string(),
            TranscriptionEngine::ParakeetCpp => "Parakeet.cpp preflight passed.".to_string(),
        },
        engine: engine_wire,
        model_filename,
        model_path,
        whisper_cli_resolved: health.whisper_cli_resolved,
        whisper_stream_resolved: health.whisper_stream_resolved,
        parakeet_cli_resolved: health.parakeet_cli_resolved,
        pyannote: health.pyannote,
    })
}

#[tauri::command]
pub async fn get_transcription_runtime_health(
    state: State<'_, AppState>,
) -> Result<RuntimeHealthResponse, CommandError> {
    recover_interrupted_runtime_install(&state)?;
    let health = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    Ok(runtime_health_response(health))
}

#[tauri::command]
pub async fn get_transcription_runtime_status(
    state: State<'_, AppState>,
) -> Result<RuntimeHealthResponse, CommandError> {
    recover_interrupted_runtime_install(&state)?;
    let health = state
        .runtime_factory
        .runtime_health_preflight()
        .map_err(|e| CommandError::new("runtime_status", e))?;

    Ok(runtime_health_response(health))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sbobino_infrastructure::{ManagedRuntimeBinaryHealth, RuntimeHealth};

    fn available_binary(name: &str) -> ManagedRuntimeBinaryHealth {
        ManagedRuntimeBinaryHealth {
            resolved_path: format!("/tmp/{name}"),
            available: true,
            failure_reason: String::new(),
            failure_message: String::new(),
        }
    }

    fn missing_binary(name: &str) -> ManagedRuntimeBinaryHealth {
        ManagedRuntimeBinaryHealth {
            resolved_path: format!("/tmp/{name}"),
            available: false,
            failure_reason: "missing_file".to_string(),
            failure_message: "Managed runtime binary is missing.".to_string(),
        }
    }

    fn runtime_health_for_managed_runtime(
        configured_engine: TranscriptionEngine,
        parakeet_available: bool,
    ) -> RuntimeHealth {
        let parakeet_cli = if parakeet_available {
            available_binary("parakeet-cli")
        } else {
            missing_binary("parakeet-cli")
        };
        RuntimeHealth {
            host_os: "macos".to_string(),
            host_arch: "aarch64".to_string(),
            is_apple_silicon: true,
            preferred_engine: TranscriptionEngine::WhisperCpp,
            configured_engine,
            runtime_source: "managed_release_asset".to_string(),
            managed_runtime_required: true,
            managed_runtime: ManagedRuntimeHealth {
                source: "managed_release_asset".to_string(),
                ready: parakeet_available,
                ffmpeg: available_binary("ffmpeg"),
                whisper_cli: available_binary("whisper-cli"),
                whisper_stream: available_binary("whisper-stream"),
                parakeet_cli,
            },
            ffmpeg_path: "ffmpeg".to_string(),
            ffmpeg_resolved: "/tmp/ffmpeg".to_string(),
            ffmpeg_available: true,
            whisper_cli_path: "whisper-cli".to_string(),
            whisper_cli_resolved: "/tmp/whisper-cli".to_string(),
            whisper_cli_available: true,
            whisper_stream_path: "whisper-stream".to_string(),
            whisper_stream_resolved: "/tmp/whisper-stream".to_string(),
            whisper_stream_available: true,
            parakeet_cli_path: "parakeet-cli".to_string(),
            parakeet_cli_resolved: "/tmp/parakeet-cli".to_string(),
            parakeet_cli_available: parakeet_available,
            models_dir_configured: "/tmp/models".to_string(),
            models_dir_resolved: "/tmp/models".to_string(),
            parakeet_models_dir_configured: "/tmp/parakeet-models".to_string(),
            parakeet_models_dir_resolved: "/tmp/parakeet-models".to_string(),
            model_filename: "ggml-base.bin".to_string(),
            model_present: true,
            parakeet_model_filename: "tdt-0.6b-v3-q4_k.gguf".to_string(),
            parakeet_model_present: true,
            missing_parakeet_models: Vec::new(),
            coreml_encoder_present: true,
            missing_models: Vec::new(),
            missing_encoders: Vec::new(),
            pyannote: PyannoteRuntimeHealth {
                enabled: false,
                ready: true,
                runtime_installed: false,
                model_installed: false,
                runtime_dir: "/tmp/runtime/pyannote".to_string(),
                arch: "aarch64-apple-darwin".to_string(),
                device: "cpu".to_string(),
                source: "disabled".to_string(),
                reason_code: "disabled".to_string(),
                message: "disabled".to_string(),
            },
            setup_complete: parakeet_available,
        }
    }

    #[test]
    fn managed_runtime_requires_parakeet_even_when_whisper_is_selected() {
        let health = runtime_health_for_managed_runtime(TranscriptionEngine::WhisperCpp, false);

        assert!(!runtime_toolchain_ready(&health));
    }

    #[test]
    fn managed_runtime_is_ready_when_all_release_binaries_are_available() {
        let health = runtime_health_for_managed_runtime(TranscriptionEngine::WhisperCpp, true);

        assert!(runtime_toolchain_ready(&health));
    }

    #[test]
    fn runtime_probe_fixture_is_one_second_pcm16_wav() {
        let path = write_runtime_probe_wav().expect("probe fixture should be created");
        let reader = hound::WavReader::open(&path).expect("probe fixture should be readable");
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.duration(), 16_000);
        drop(reader);
        std::fs::remove_file(path).expect("probe fixture should be cleaned up");
    }

    #[cfg(unix)]
    #[test]
    fn parakeet_worker_output_requires_one_complete_json_row() {
        let output = std::process::Command::new("sh")
            .args([
                "-c",
                "printf '%s\\n' '{\"index\":0,\"result\":{\"text\":\"\"}}'",
            ])
            .output()
            .expect("shell should run");
        validate_parakeet_worker_output(&output).expect("worker row should validate");

        let invalid = std::process::Command::new("sh")
            .args(["-c", "printf '%s\\n' '{\"index\":1}'"])
            .output()
            .expect("shell should run");
        assert!(validate_parakeet_worker_output(&invalid).is_err());
    }

    #[test]
    fn runtime_probe_cache_key_changes_when_model_changes() {
        let temp = tempfile::tempdir().expect("probe tempdir should exist");
        let ffmpeg = temp.path().join("ffmpeg");
        let engine = temp.path().join("engine");
        let model = temp.path().join("model.gguf");
        std::fs::write(&ffmpeg, b"ffmpeg").expect("ffmpeg fixture should write");
        std::fs::write(&engine, b"engine").expect("engine fixture should write");
        std::fs::write(&model, b"model-a").expect("model fixture should write");
        let first = runtime_probe_cache_key(
            &TranscriptionEngine::WhisperCpp,
            &ffmpeg,
            &engine,
            &model,
            TranscriptionComputeDevice::Auto,
        );
        std::fs::write(&model, b"model-b-with-a-different-size")
            .expect("model fixture should update");
        let second = runtime_probe_cache_key(
            &TranscriptionEngine::WhisperCpp,
            &ffmpeg,
            &engine,
            &model,
            TranscriptionComputeDevice::Auto,
        );
        assert_ne!(first, second);
    }

    #[test]
    fn runtime_probe_compute_device_controls_args_and_worker_environment() {
        let model = Path::new("model.gguf");
        let fixture = Path::new("probe.wav");
        let cpu_args = whisper_probe_args(model, fixture, TranscriptionComputeDevice::Cpu);
        assert!(cpu_args.windows(2).any(|pair| pair == ["-ng", "-nfa"]));
        let auto_args = whisper_probe_args(model, fixture, TranscriptionComputeDevice::Auto);
        assert!(!auto_args.iter().any(|arg| arg == "-ng" || arg == "-nfa"));
        let gpu_args = whisper_probe_args(model, fixture, TranscriptionComputeDevice::Gpu);
        assert!(!gpu_args.iter().any(|arg| arg == "-ng" || arg == "-nfa"));

        let mut cpu_command = tokio_background_command("parakeet-batch-json");
        configure_probe_compute_environment(
            &mut cpu_command,
            &TranscriptionEngine::ParakeetCpp,
            TranscriptionComputeDevice::Cpu,
        );
        assert!(cpu_command.as_std().get_envs().any(|(name, value)| {
            name == std::ffi::OsStr::new("PARAKEET_DEVICE")
                && value == Some(std::ffi::OsStr::new("cpu"))
        }));

        let mut gpu_command = tokio_background_command("parakeet-batch-json");
        gpu_command
            .env("PARAKEET_DEVICE", "cpu")
            .env("SBOBINO_PARAKEET_FORCE_CPU", "1");
        configure_probe_compute_environment(
            &mut gpu_command,
            &TranscriptionEngine::ParakeetCpp,
            TranscriptionComputeDevice::Gpu,
        );
        assert!(!gpu_command.as_std().get_envs().any(|(name, value)| {
            (name == std::ffi::OsStr::new("PARAKEET_DEVICE")
                || name == std::ffi::OsStr::new("SBOBINO_PARAKEET_FORCE_CPU"))
                && value.is_some()
        }));

        let mut auto_command = tokio_background_command("parakeet-batch-json");
        auto_command.env("PARAKEET_DEVICE", "inherited");
        configure_probe_compute_environment(
            &mut auto_command,
            &TranscriptionEngine::ParakeetCpp,
            TranscriptionComputeDevice::Auto,
        );
        assert!(auto_command.as_std().get_envs().any(|(name, value)| {
            name == std::ffi::OsStr::new("PARAKEET_DEVICE")
                && value == Some(std::ffi::OsStr::new("inherited"))
        }));
    }

    #[test]
    fn runtime_probe_cache_key_includes_compute_device() {
        let temp = tempfile::tempdir().expect("probe tempdir should exist");
        let ffmpeg = temp.path().join("ffmpeg");
        let engine = temp.path().join("engine");
        let model = temp.path().join("model.gguf");
        std::fs::write(&ffmpeg, b"ffmpeg").expect("ffmpeg fixture should write");
        std::fs::write(&engine, b"engine").expect("engine fixture should write");
        std::fs::write(&model, b"model").expect("model fixture should write");
        let auto = runtime_probe_cache_key(
            &TranscriptionEngine::ParakeetCpp,
            &ffmpeg,
            &engine,
            &model,
            TranscriptionComputeDevice::Auto,
        );
        let cpu = runtime_probe_cache_key(
            &TranscriptionEngine::ParakeetCpp,
            &ffmpeg,
            &engine,
            &model,
            TranscriptionComputeDevice::Cpu,
        );
        assert_ne!(auto, cpu);
    }

    #[tokio::test]
    async fn runtime_recovery_defers_when_provisioning_slot_is_contended() {
        let temp = tempfile::tempdir().expect("recovery tempdir should exist");
        let journal = temp.path().join(".runtime-install-journal.json");
        std::fs::write(&journal, b"not valid json")
            .expect("malformed journal fixture should write");
        let slot = std::sync::Arc::new(tokio::sync::Mutex::new(None));

        // Hold the provisioning mutex across the recovery attempt.  A
        // contended try_lock is an active/unknown transaction, so recovery
        // must return without even parsing or touching the journal.
        let slot_guard = slot.lock().await;
        recover_interrupted_runtime_install_for_slot(&slot, temp.path())
            .expect("recovery should defer while provisioning owns the slot");
        assert!(
            journal.is_file(),
            "deferred recovery must leave the journal untouched"
        );
        drop(slot_guard);

        // Once the slot is observably idle, the same fixture is reached and
        // reports its malformed journal instead of being silently skipped.
        assert!(recover_interrupted_runtime_install_for_slot(&slot, temp.path()).is_err());
    }

    #[test]
    fn runtime_recovery_holds_idle_slot_through_none_to_some_transition() {
        let temp = tempfile::tempdir().expect("recovery tempdir should exist");
        let journal = temp.path().join(".runtime-install-journal.json");
        std::fs::write(&journal, b"not valid json")
            .expect("malformed journal fixture should write");
        let slot = std::sync::Arc::new(tokio::sync::Mutex::new(None));

        // Hold the transaction lock so the recovery thread must remain inside
        // the synchronous recovery call after it acquires the initially idle
        // provisioning slot.  This makes the None -> Some race observable.
        let transaction_guard = crate::commands::provisioning::runtime_install_transaction_lock()
            .lock()
            .expect("runtime transaction lock should be available");
        let recovery_slot = slot.clone();
        let recovery_root = temp.path().to_path_buf();
        let recovery_thread = std::thread::spawn(move || {
            recover_interrupted_runtime_install_for_slot(&recovery_slot, &recovery_root)
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let mut recovery_holds_slot = false;
        while std::time::Instant::now() < deadline {
            if slot.try_lock().is_err() {
                recovery_holds_slot = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(
            recovery_holds_slot,
            "recovery should hold the idle provisioning slot while it runs"
        );

        let (transition_tx, transition_rx) = std::sync::mpsc::channel();
        let transition_slot = slot.clone();
        let transition_thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("transition runtime should build");
            runtime.block_on(async move {
                let mut active = transition_slot.lock().await;
                *active = Some(CancellationToken::new());
                transition_tx
                    .send(())
                    .expect("transition should signal acquisition");
            });
        });

        assert!(
            transition_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "provisioning must not acquire the slot during recovery"
        );
        drop(transaction_guard);

        recovery_thread
            .join()
            .expect("recovery thread should join")
            .expect_err("malformed journal should be reported after the lock is released");
        transition_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("provisioning transition should complete after recovery");
        transition_thread
            .join()
            .expect("transition thread should join");

        let active = slot
            .try_lock()
            .expect("slot should be idle after transition");
        assert!(
            active.is_some(),
            "transition should publish an active operation"
        );
    }
}
