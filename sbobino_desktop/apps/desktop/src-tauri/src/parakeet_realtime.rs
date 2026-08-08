use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_float, c_int};
use std::panic::{self, AssertUnwindSafe};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BuildStreamError, SampleFormat, Stream, StreamConfig,
};
use libloading::Library;

use sbobino_application::{ApplicationError, RealtimeDelta, RealtimeDeltaKind};
use sbobino_domain::{LanguageCode, TimedSegment};

use crate::realtime_audio::{classify_input_error, RealtimeInputLevelEvent};

type ParakeetCtx = std::ffi::c_void;
type ParakeetStream = std::ffi::c_void;
type ParakeetLiveDeltaPiece = (String, Option<String>, bool);
type ParakeetLiveDeltaPieces = Vec<ParakeetLiveDeltaPiece>;

type LoadFn = unsafe extern "C" fn(*const c_char) -> *mut ParakeetCtx;
type FreeFn = unsafe extern "C" fn(*mut ParakeetCtx);
type StreamBeginFn = unsafe extern "C" fn(*mut ParakeetCtx) -> *mut ParakeetStream;
type StreamBeginLangFn =
    unsafe extern "C" fn(*mut ParakeetCtx, *const c_char) -> *mut ParakeetStream;
type StreamFeedFn =
    unsafe extern "C" fn(*mut ParakeetStream, *const c_float, c_int, *mut c_int) -> *mut c_char;
type StreamFinalizeFn = unsafe extern "C" fn(*mut ParakeetStream) -> *mut c_char;
type StreamFreeFn = unsafe extern "C" fn(*mut ParakeetStream);
type FreeStringFn = unsafe extern "C" fn(*mut c_char);
type LastErrorFn = unsafe extern "C" fn(*mut ParakeetCtx) -> *const c_char;

#[derive(Clone)]
struct ParakeetApi {
    _library: Arc<Library>,
    load: LoadFn,
    free: FreeFn,
    stream_begin: StreamBeginFn,
    stream_begin_lang: Option<StreamBeginLangFn>,
    stream_feed: StreamFeedFn,
    stream_finalize: StreamFinalizeFn,
    stream_free: StreamFreeFn,
    free_string: FreeStringFn,
    last_error: LastErrorFn,
}

#[derive(Default)]
struct ParakeetRealtimeState {
    running: bool,
    paused: Arc<AtomicBool>,
    shutdown_tx: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
    transcript: Arc<Mutex<String>>,
    segments: Arc<Mutex<Vec<TimedSegment>>>,
    diagnostics: Arc<Mutex<Vec<String>>>,
    saved_audio_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ParakeetRealtimeStopResult {
    pub transcript: String,
    pub segments: Vec<TimedSegment>,
    pub saved_audio_path: Option<PathBuf>,
}

struct ParakeetCaptureSession {
    target_lang: String,
    audio_path: PathBuf,
    shutdown_rx: mpsc::Receiver<()>,
    startup_tx: mpsc::Sender<Result<(), ApplicationError>>,
    paused: Arc<AtomicBool>,
    transcript: Arc<Mutex<String>>,
    segments: Arc<Mutex<Vec<TimedSegment>>>,
    diagnostics: Arc<Mutex<Vec<String>>>,
    emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    emit_input_level: Arc<dyn Fn(RealtimeInputLevelEvent) + Send + Sync>,
}

#[derive(Clone)]
pub struct ParakeetRealtimeEngine {
    lib_path: PathBuf,
    models_dir: PathBuf,
    state: Arc<Mutex<ParakeetRealtimeState>>,
}

impl ParakeetRealtimeEngine {
    pub fn new(lib_path: PathBuf, models_dir: PathBuf) -> Self {
        Self {
            lib_path,
            models_dir,
            state: Arc::new(Mutex::new(ParakeetRealtimeState {
                paused: Arc::new(AtomicBool::new(false)),
                ..ParakeetRealtimeState::default()
            })),
        }
    }

    pub async fn start(
        &self,
        model_filename: &str,
        target_lang: &str,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
        emit_input_level: Arc<dyn Fn(RealtimeInputLevelEvent) + Send + Sync>,
    ) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().map_err(lock_error)?;
        if state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is already running".to_string(),
            ));
        }

        eprintln!(
            "[parakeet-live] start requested: lib_path={} models_dir={} model={} target_lang={}",
            self.lib_path.display(),
            self.models_dir.display(),
            model_filename,
            target_lang,
        );

        let api = match self.load_api() {
            Ok(api) => {
                eprintln!("[parakeet-live] FFI library loaded successfully");
                api
            }
            Err(error) => {
                eprintln!("[parakeet-live] FFI library load FAILED: {error}");
                return Err(error);
            }
        };
        let model_path = self.models_dir.join(model_filename);
        if !model_path.exists() {
            let error = ApplicationError::SpeechToText(format!(
                "Parakeet realtime model file not found at {}. Download '{}' from Settings > Local Models.",
                model_path.display(),
                model_filename,
            ));
            eprintln!("[parakeet-live] model file missing: {error}");
            return Err(error);
        }
        eprintln!("[parakeet-live] model resolved at {}", model_path.display());

        let session_dir = create_session_dir()?;
        let saved_audio_path = session_dir.join("parakeet-live.wav");
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let paused = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::channel();
        let model_for_thread = model_path.clone();
        let target_lang_for_thread = target_lang.to_string();
        let audio_for_thread = saved_audio_path.clone();
        let transcript_for_thread = transcript.clone();
        let segments_for_thread = segments.clone();
        let diagnostics_for_thread = diagnostics.clone();
        let paused_for_thread = paused.clone();

        emit_parakeet_input_level(
            emit_input_level.as_ref(),
            "connecting",
            0.0,
            "Connecting to the microphone...",
        );

        let startup_tx_for_panic = startup_tx.clone();
        let worker = thread::Builder::new()
            .name("parakeet-realtime-capture".to_string())
            .spawn(move || {
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    run_parakeet_capture(
                        api,
                        model_for_thread,
                        ParakeetCaptureSession {
                            target_lang: target_lang_for_thread,
                            audio_path: audio_for_thread,
                            shutdown_rx,
                            startup_tx: startup_tx.clone(),
                            paused: paused_for_thread,
                            transcript: transcript_for_thread,
                            segments: segments_for_thread,
                            diagnostics: diagnostics_for_thread,
                            emit_delta,
                            emit_input_level,
                        },
                    )
                }));

                match result {
                    Ok(Ok(())) => {
                        eprintln!("[parakeet-live] capture loop exited cleanly");
                    }
                    Ok(Err(error)) => {
                        eprintln!("[parakeet-live] capture loop returned error: {error}");
                        let _ = startup_tx.send(Err(error));
                    }
                    Err(panic_payload) => {
                        let detail = if let Some(message) = panic_payload.downcast_ref::<&str>() {
                            (*message).to_string()
                        } else if let Some(message) = panic_payload.downcast_ref::<String>() {
                            message.clone()
                        } else {
                            "unknown panic payload".to_string()
                        };
                        eprintln!("[parakeet-live] capture thread PANICKED: {detail}");
                        let _ = startup_tx_for_panic.send(Err(ApplicationError::SpeechToText(
                            format!("Parakeet live capture thread panicked: {detail}"),
                        )));
                    }
                }
            })
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to spawn Parakeet realtime capture thread: {error}"
                ))
            })?;

        match startup_rx.recv_timeout(Duration::from_secs(90)) {
            Ok(Ok(())) => {
                state.running = true;
                state.paused = paused;
                state.shutdown_tx = Some(shutdown_tx);
                state.worker = Some(worker);
                state.transcript = transcript;
                state.segments = segments;
                state.diagnostics = diagnostics;
                state.saved_audio_path = Some(saved_audio_path);
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = shutdown_tx.send(());
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = shutdown_tx.send(());
                let _ = worker.join();
                Err(ApplicationError::SpeechToText(
                    "Parakeet realtime startup timed out while waiting for microphone capture."
                        .to_string(),
                ))
            }
        }
    }

    pub async fn pause(&self) -> Result<(), ApplicationError> {
        let state = self.state.lock().map_err(lock_error)?;
        if !state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is not running".to_string(),
            ));
        }
        state.paused.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), ApplicationError> {
        let state = self.state.lock().map_err(lock_error)?;
        if !state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is not running".to_string(),
            ));
        }
        state.paused.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub async fn stop(&self) -> Result<ParakeetRealtimeStopResult, ApplicationError> {
        let worker = {
            let mut state = self.state.lock().map_err(lock_error)?;
            if let Some(tx) = state.shutdown_tx.take() {
                let _ = tx.send(());
            }
            state.worker.take()
        };

        if let Some(worker) = worker {
            let _ = worker.join();
        }

        let mut state = self.state.lock().map_err(lock_error)?;
        state.running = false;
        state.paused.store(false, Ordering::Relaxed);
        let transcript = state
            .transcript
            .lock()
            .map_err(|_| {
                ApplicationError::SpeechToText("Parakeet transcript lock poisoned".to_string())
            })?
            .trim()
            .to_string();
        let segments = state
            .segments
            .lock()
            .map_err(|_| {
                ApplicationError::SpeechToText("Parakeet segment lock poisoned".to_string())
            })?
            .clone();
        Ok(ParakeetRealtimeStopResult {
            transcript,
            segments,
            saved_audio_path: state.saved_audio_path.clone(),
        })
    }

    pub async fn is_running(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.running)
            .unwrap_or(false)
    }

    pub async fn snapshot_diagnostics(&self) -> Vec<String> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.diagnostics.lock().ok().map(|items| items.clone()))
            .unwrap_or_default()
    }

    pub async fn seed_buffer(&self, text: &str) {
        if let Ok(state) = self.state.lock() {
            if let Ok(mut transcript) = state.transcript.lock() {
                *transcript = text.trim().to_string();
            }
            if let Ok(mut segments) = state.segments.lock() {
                segments.clear();
            }
        }
    }

    pub async fn reset(&self) {
        if let Ok(state) = self.state.lock() {
            if let Ok(mut transcript) = state.transcript.lock() {
                transcript.clear();
            }
            if let Ok(mut segments) = state.segments.lock() {
                segments.clear();
            }
            if let Ok(mut diagnostics) = state.diagnostics.lock() {
                diagnostics.clear();
            }
        }
    }

    pub fn validate_library(&self) -> Result<(), ApplicationError> {
        self.load_api().map(|_| ())
    }

    fn load_api(&self) -> Result<ParakeetApi, ApplicationError> {
        if !self.lib_path.exists() {
            return Err(ApplicationError::SpeechToText(format!(
                "Parakeet live library not found at {}. Reinstall the local runtime.",
                self.lib_path.display()
            )));
        }

        // Keep Metal enabled, but disable ggml Metal features that can make the
        // packaged runtime diverge from the dev runtime on Apple Silicon. CPU
        // fallback is intentionally opt-in only because it can make the system
        // unusably slow for realtime transcription.
        for (name, value) in Self::safe_metal_environment() {
            std::env::set_var(name, value);
        }
        if let Some(device) = Self::parakeet_device_override() {
            std::env::set_var("PARAKEET_DEVICE", device);
        }

        let library = unsafe { Library::new(&self.lib_path) }.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to load Parakeet live library {}: {error}",
                self.lib_path.display()
            ))
        })?;
        let library = Arc::new(library);

        unsafe {
            Ok(ParakeetApi {
                load: *library
                    .get(b"parakeet_capi_load\0")
                    .map_err(map_symbol_error)?,
                free: *library
                    .get(b"parakeet_capi_free\0")
                    .map_err(map_symbol_error)?,
                stream_begin: *library
                    .get(b"parakeet_capi_stream_begin\0")
                    .map_err(map_symbol_error)?,
                stream_begin_lang: library
                    .get(b"parakeet_capi_stream_begin_lang\0")
                    .map(|symbol: libloading::Symbol<StreamBeginLangFn>| *symbol)
                    .ok(),
                stream_feed: *library
                    .get(b"parakeet_capi_stream_feed\0")
                    .map_err(map_symbol_error)?,
                stream_finalize: *library
                    .get(b"parakeet_capi_stream_finalize\0")
                    .map_err(map_symbol_error)?,
                stream_free: *library
                    .get(b"parakeet_capi_stream_free\0")
                    .map_err(map_symbol_error)?,
                free_string: *library
                    .get(b"parakeet_capi_free_string\0")
                    .map_err(map_symbol_error)?,
                last_error: *library
                    .get(b"parakeet_capi_last_error\0")
                    .map_err(map_symbol_error)?,
                _library: library,
            })
        }
    }

    fn parakeet_device_override() -> Option<&'static str> {
        if Self::truthy_env("SBOBINO_PARAKEET_FORCE_CPU") {
            return Some("cpu");
        }
        if Self::truthy_env("SBOBINO_PARAKEET_FORCE_METAL") {
            return None;
        }
        None
    }

    fn safe_metal_environment() -> &'static [(&'static str, &'static str)] {
        &[
            ("GGML_METAL_NO_RESIDENCY", "1"),
            ("GGML_METAL_SHARED_BUFFERS_DISABLE", "1"),
            ("GGML_METAL_CONCURRENCY_DISABLE", "1"),
        ]
    }

    fn truthy_env(name: &str) -> bool {
        std::env::var(name)
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    }
}

fn create_session_dir() -> Result<PathBuf, ApplicationError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let session_dir = std::env::temp_dir().join(format!(
        "sbobino-parakeet-live-{timestamp}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&session_dir).map_err(|error| {
        ApplicationError::SpeechToText(format!(
            "failed to create Parakeet realtime session directory at {}: {error}",
            session_dir.display()
        ))
    })?;
    Ok(session_dir)
}

fn map_symbol_error(error: libloading::Error) -> ApplicationError {
    ApplicationError::SpeechToText(format!(
        "Parakeet live library is missing a required C API symbol: {error}"
    ))
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ApplicationError {
    ApplicationError::SpeechToText("Parakeet realtime state lock poisoned".to_string())
}

fn last_error(api: &ParakeetApi, ctx: *mut ParakeetCtx) -> String {
    unsafe {
        let ptr = (api.last_error)(ctx);
        if ptr.is_null() {
            return String::new();
        }
        CStr::from_ptr(ptr).to_string_lossy().to_string()
    }
}

fn take_c_string(api: &ParakeetApi, ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(ptr).to_string_lossy().to_string() };
    unsafe { (api.free_string)(ptr) };
    Some(text)
}

fn run_parakeet_capture(
    api: ParakeetApi,
    model_path: PathBuf,
    mut session: ParakeetCaptureSession,
) -> Result<(), ApplicationError> {
    eprintln!(
        "[parakeet-live] run_parakeet_capture: model={} target_lang={} audio={}",
        model_path.display(),
        session.target_lang,
        session.audio_path.display(),
    );
    let model_c = CString::new(model_path.to_string_lossy().as_bytes()).map_err(|_| {
        ApplicationError::SpeechToText("Parakeet model path contains a NUL byte".to_string())
    })?;
    let ctx = unsafe { (api.load)(model_c.as_ptr()) };
    if ctx.is_null() {
        eprintln!(
            "[parakeet-live] parakeet_capi_load returned null for {}",
            model_path.display()
        );
        return Err(ApplicationError::SpeechToText(format!(
            "failed to load Parakeet realtime model {}. The model file may be corrupt or incompatible with the installed libparakeet.",
            model_path.display()
        )));
    }
    eprintln!("[parakeet-live] parakeet_capi_load OK");

    let mut stream = begin_parakeet_stream(&api, ctx, &session.target_lang)?;
    if stream.is_null() {
        let detail = last_error(&api, ctx);
        eprintln!("[parakeet-live] parakeet_capi_stream_begin returned null: {detail}");
        unsafe { (api.free)(ctx) };
        let hint = if model_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("tdt-"))
            .unwrap_or(false)
        {
            " This TDT model is for file transcription. Use the NVIDIA Nemotron live model for live transcription."
        } else {
            ""
        };
        return Err(ApplicationError::SpeechToText(format!(
            "failed to start Parakeet realtime stream for {}.{hint} Detail: {}",
            model_path.display(),
            if detail.is_empty() {
                "model is not a cache-aware streaming model"
            } else {
                detail.as_str()
            }
        )));
    }
    eprintln!("[parakeet-live] parakeet_capi_stream_begin OK");

    let result = run_capture_loop(&api, ctx, &mut stream, &mut session);

    unsafe {
        (api.stream_free)(stream);
        (api.free)(ctx);
    }

    result
}

fn begin_parakeet_stream(
    api: &ParakeetApi,
    ctx: *mut ParakeetCtx,
    target_lang: &str,
) -> Result<*mut ParakeetStream, ApplicationError> {
    let lang = target_lang.trim();
    let stream = if lang.is_empty() || lang == "en" {
        unsafe { (api.stream_begin)(ctx) }
    } else {
        let Some(stream_begin_lang) = api.stream_begin_lang else {
            return Err(ApplicationError::SpeechToText(
                "Parakeet local runtime is outdated for live transcription. Repair it from Settings > Local Models.".to_string(),
            ));
        };
        let lang_c = CString::new(lang).map_err(|_| {
            ApplicationError::SpeechToText(
                "Parakeet target language contains a NUL byte".to_string(),
            )
        })?;
        unsafe { stream_begin_lang(ctx, lang_c.as_ptr()) }
    };
    if stream.is_null() {
        return Err(ApplicationError::SpeechToText(format!(
            "failed to start Parakeet realtime stream for target language '{}'. Detail: {}",
            if lang.is_empty() { "default" } else { lang },
            last_error(api, ctx)
        )));
    }
    Ok(stream)
}

fn mean_abs_input_level(samples: impl Iterator<Item = f32>) -> f32 {
    let mut sum_squares = 0.0_f32;
    let mut peak = 0.0_f32;
    let mut count = 0_u32;
    for sample in samples {
        let abs = sample.abs();
        peak = peak.max(abs);
        sum_squares += sample * sample;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }

    let rms = (sum_squares / count as f32).sqrt().max(0.000_001);
    // Speech captured by macOS input devices is often around -45..-20 dBFS.
    // Map that range aggressively so the live waveform remains visible while
    // still showing silence as a low baseline.
    let rms_db = 20.0 * rms.log10();
    let rms_level = ((rms_db + 58.0) / 38.0).clamp(0.0, 1.0);
    let peak_level = (peak * 9.0).clamp(0.0, 1.0);
    (rms_level * 0.72 + peak_level * 0.28).clamp(0.0, 1.0)
}

fn emit_parakeet_input_level(
    emit_input_level: &(dyn Fn(RealtimeInputLevelEvent) + Send + Sync),
    state: &str,
    level: f32,
    message: impl Into<String>,
) {
    emit_input_level(RealtimeInputLevelEvent {
        state: state.to_string(),
        level: level.clamp(0.0, 1.0),
        message: message.into(),
    });
}

fn run_capture_loop(
    api: &ParakeetApi,
    ctx: *mut ParakeetCtx,
    stream: &mut *mut ParakeetStream,
    session: &mut ParakeetCaptureSession,
) -> Result<(), ApplicationError> {
    eprintln!("[parakeet-live] run_capture_loop: probing default input device");
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(device) => {
            eprintln!("[parakeet-live] default input device resolved");
            device
        }
        None => {
            eprintln!("[parakeet-live] no audio input device available");
            let message = "No audio input device is available. Connect a microphone and grant Sbobino microphone access in System Settings > Privacy & Security > Microphone.";
            emit_parakeet_input_level(
                session.emit_input_level.as_ref(),
                "unavailable",
                0.0,
                message,
            );
            return Err(ApplicationError::SpeechToText(message.to_string()));
        }
    };
    let device_name = device
        .name()
        .unwrap_or_else(|_| "Default microphone".to_string());
    let supported_config = device.default_input_config().map_err(|error| {
        let input_error = classify_input_error(&error.to_string());
        emit_parakeet_input_level(
            session.emit_input_level.as_ref(),
            &input_error.state,
            0.0,
            input_error.message.clone(),
        );
        ApplicationError::SpeechToText(format!(
            "failed to read default microphone config for Parakeet live: {error}. {}",
            input_error.message
        ))
    })?;
    let config = supported_config.config();
    let sample_rate = config.sample_rate.0;
    let channels = usize::from(config.channels.max(1));
    eprintln!(
        "[parakeet-live] input config: sample_rate={} channels={}",
        sample_rate, channels
    );
    let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>();
    let last_capture_error = Arc::new(Mutex::new(None::<String>));
    let input_stream = build_input_stream(
        &device,
        &config,
        supported_config.sample_format(),
        channels,
        audio_tx,
        last_capture_error.clone(),
    )
    .map_err(|error| {
        let input_error = classify_input_error(&error.to_string());
        emit_parakeet_input_level(
            session.emit_input_level.as_ref(),
            &input_error.state,
            0.0,
            input_error.message.clone(),
        );
        ApplicationError::SpeechToText(format!(
            "Parakeet live microphone setup failed: {error}. {}",
            input_error.message
        ))
    })?;

    input_stream.play().map_err(|error| {
        let input_error = classify_input_error(&error.to_string());
        emit_parakeet_input_level(
            session.emit_input_level.as_ref(),
            &input_error.state,
            0.0,
            input_error.message.clone(),
        );
        ApplicationError::SpeechToText(format!(
            "Parakeet live microphone start failed: {error}. {}",
            input_error.message
        ))
    })?;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&session.audio_path, spec).map_err(|error| {
        ApplicationError::SpeechToText(format!(
            "failed to create Parakeet live WAV at {}: {error}",
            session.audio_path.display()
        ))
    })?;

    const PARAKEET_LIVE_FEED_SAMPLES: usize = 16_000;
    const PARAKEET_LIVE_FORCE_SEGMENT_SECONDS: f32 = 12.0;
    const PARAKEET_LIVE_NO_DELTA_RESTART_SECONDS: f32 = 10.0;

    let mut resampler = LinearResampler::new(sample_rate, 16_000);
    let mut captured_samples: u64 = 0;
    let mut fed_samples: u64 = 0;
    let mut pending_feed = Vec::<f32>::with_capacity(PARAKEET_LIVE_FEED_SAMPLES * 2);
    let mut last_delta_seconds = 0.0_f32;
    let mut last_stream_restart_seconds = 0.0_f32;
    let mut assembler = ParakeetLiveAssembler::new(
        session.transcript.clone(),
        session.segments.clone(),
        session.emit_delta.clone(),
    );
    let mut last_input_level_emit = Instant::now();
    let mut pending_input_level = 0.0_f32;
    let _stream_guard = input_stream;
    emit_parakeet_input_level(
        session.emit_input_level.as_ref(),
        "running",
        0.0,
        format!("Using {device_name}"),
    );
    let _ = session.startup_tx.send(Ok(()));

    loop {
        if session.shutdown_rx.try_recv().is_ok() {
            break;
        }

        if let Ok(mut slot) = last_capture_error.lock() {
            if let Some(error) = slot.take() {
                if let Ok(mut items) = session.diagnostics.lock() {
                    items.push(error);
                }
            }
        }

        let chunk = match audio_rx.recv_timeout(Duration::from_millis(60)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if session.paused.load(Ordering::Relaxed) {
            emit_parakeet_input_level(
                session.emit_input_level.as_ref(),
                "paused",
                0.0,
                "Microphone preview paused.",
            );
            continue;
        }

        pending_input_level = pending_input_level.max(mean_abs_input_level(chunk.iter().copied()));
        if last_input_level_emit.elapsed() >= Duration::from_millis(45) {
            emit_parakeet_input_level(
                session.emit_input_level.as_ref(),
                "running",
                pending_input_level,
                format!("Using {device_name}"),
            );
            pending_input_level = 0.0;
            last_input_level_emit = Instant::now();
        }

        let pcm_16k = resampler.push(&chunk);
        if pcm_16k.is_empty() {
            continue;
        }
        captured_samples = captured_samples.saturating_add(pcm_16k.len() as u64);
        write_pcm_i16(&mut writer, &pcm_16k)?;
        pending_feed.extend_from_slice(&pcm_16k);

        while pending_feed.len() >= PARAKEET_LIVE_FEED_SAMPLES {
            let feed_chunk = pending_feed
                .drain(..PARAKEET_LIVE_FEED_SAMPLES)
                .collect::<Vec<_>>();
            fed_samples = fed_samples.saturating_add(feed_chunk.len() as u64);
            let current_seconds = fed_samples as f32 / 16_000.0;
            let outcome = feed_parakeet_live_chunk(
                api,
                ctx,
                *stream,
                &feed_chunk,
                current_seconds,
                &mut assembler,
            )?;
            if outcome.had_delta {
                last_delta_seconds = current_seconds;
            }
            if outcome.eou
                || assembler.should_force_finish_segment(
                    current_seconds,
                    PARAKEET_LIVE_FORCE_SEGMENT_SECONDS,
                )
            {
                assembler.finish_segment(current_seconds);
            }
            if current_seconds - last_delta_seconds >= PARAKEET_LIVE_NO_DELTA_RESTART_SECONDS
                && current_seconds - last_stream_restart_seconds
                    >= PARAKEET_LIVE_NO_DELTA_RESTART_SECONDS
            {
                if assembler.has_current_text() {
                    assembler.finish_segment(current_seconds);
                }
                eprintln!(
                    "[parakeet-live] no transcript delta for {:.1}s; restarting C API stream",
                    current_seconds - last_delta_seconds
                );
                restart_parakeet_live_stream(api, ctx, stream, &session.target_lang)?;
                last_stream_restart_seconds = current_seconds;
                last_delta_seconds = current_seconds;
            }
        }
    }

    let tail = resampler.finish();
    if !tail.is_empty() {
        captured_samples = captured_samples.saturating_add(tail.len() as u64);
        write_pcm_i16(&mut writer, &tail)?;
        pending_feed.extend_from_slice(&tail);
    }

    if !pending_feed.is_empty() {
        fed_samples = fed_samples.saturating_add(pending_feed.len() as u64);
        let current_seconds = fed_samples as f32 / 16_000.0;
        let outcome = feed_parakeet_live_chunk(
            api,
            ctx,
            *stream,
            &pending_feed,
            current_seconds,
            &mut assembler,
        )?;
        if outcome.eou
            || assembler
                .should_force_finish_segment(current_seconds, PARAKEET_LIVE_FORCE_SEGMENT_SECONDS)
        {
            assembler.finish_segment(current_seconds);
        }
        pending_feed.clear();
    }

    let ptr = unsafe { (api.stream_finalize)(*stream) };
    let delta = take_c_string(api, ptr).ok_or_else(|| {
        ApplicationError::SpeechToText(format!(
            "Parakeet live stream finalize failed: {}",
            last_error(api, ctx)
        ))
    })?;
    let final_seconds = captured_samples.max(fed_samples) as f32 / 16_000.0;
    assembler.push_delta(&delta, final_seconds);
    assembler.finish_segment(final_seconds);
    writer.finalize().map_err(|error| {
        ApplicationError::SpeechToText(format!(
            "failed to finalize Parakeet live WAV at {}: {error}",
            session.audio_path.display()
        ))
    })?;
    Ok(())
}

struct ParakeetLiveFeedOutcome {
    eou: bool,
    had_delta: bool,
}

fn feed_parakeet_live_chunk(
    api: &ParakeetApi,
    ctx: *mut ParakeetCtx,
    stream: *mut ParakeetStream,
    samples: &[f32],
    current_seconds: f32,
    assembler: &mut ParakeetLiveAssembler,
) -> Result<ParakeetLiveFeedOutcome, ApplicationError> {
    let mut eou = 0;
    let ptr = unsafe {
        (api.stream_feed)(
            stream,
            samples.as_ptr(),
            samples.len() as c_int,
            &mut eou as *mut c_int,
        )
    };
    let delta = take_c_string(api, ptr).ok_or_else(|| {
        ApplicationError::SpeechToText(format!(
            "Parakeet live stream feed failed: {}",
            last_error(api, ctx)
        ))
    })?;
    let had_delta = assembler.push_delta(&delta, current_seconds);
    Ok(ParakeetLiveFeedOutcome {
        eou: eou != 0,
        had_delta,
    })
}

fn restart_parakeet_live_stream(
    api: &ParakeetApi,
    ctx: *mut ParakeetCtx,
    stream: &mut *mut ParakeetStream,
    target_lang: &str,
) -> Result<(), ApplicationError> {
    unsafe {
        if !stream.is_null() {
            (api.stream_free)(*stream);
        }
    }
    *stream = begin_parakeet_stream(api, ctx, target_lang)?;
    Ok(())
}

struct ParakeetLiveAssembler {
    transcript: Arc<Mutex<String>>,
    segments: Arc<Mutex<Vec<TimedSegment>>>,
    emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    current_text: String,
    current_start_seconds: Option<f32>,
    current_end_seconds: Option<f32>,
    last_segment_end_seconds: f32,
    current_language_code: Option<String>,
    current_language_confidence: Option<f32>,
    pending_marker_fragment: String,
}

impl ParakeetLiveAssembler {
    fn new(
        transcript: Arc<Mutex<String>>,
        segments: Arc<Mutex<Vec<TimedSegment>>>,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    ) -> Self {
        Self {
            transcript,
            segments,
            emit_delta,
            current_text: String::new(),
            current_start_seconds: None,
            current_end_seconds: None,
            last_segment_end_seconds: 0.0,
            current_language_code: None,
            current_language_confidence: None,
            pending_marker_fragment: String::new(),
        }
    }

    fn push_delta(&mut self, delta: &str, current_seconds: f32) -> bool {
        let combined = format!("{}{}", self.pending_marker_fragment, delta);
        self.pending_marker_fragment.clear();
        let (pieces, pending_fragment) = normalize_parakeet_live_delta_pieces(&combined);
        if let Some(fragment) = pending_fragment {
            self.pending_marker_fragment = fragment;
        }
        let mut had_text = false;
        for (display, detected_language, finish_after) in pieces {
            if let Some(language) = detected_language {
                if self.has_current_text()
                    && self.current_language_code.as_deref() != Some(language.as_str())
                {
                    self.finish_segment(current_seconds);
                }
                self.current_language_code = Some(language);
                self.current_language_confidence = None;
            }
            let display_text = display.trim();
            if display_text.is_empty() {
                continue;
            }

            if self.current_start_seconds.is_none() {
                self.current_start_seconds =
                    Some(self.last_segment_end_seconds.min(current_seconds));
            }
            self.current_end_seconds = Some(current_seconds.max(self.last_segment_end_seconds));
            let starts_new_word = display.chars().next().is_some_and(char::is_whitespace);
            if starts_new_word
                && !self.current_text.is_empty()
                && !self
                    .current_text
                    .chars()
                    .last()
                    .is_some_and(char::is_whitespace)
            {
                self.current_text.push(' ');
            }
            self.current_text.push_str(display_text);

            (self.emit_delta)(RealtimeDelta {
                kind: RealtimeDeltaKind::UpdatePreview,
                text: self.current_text.clone(),
                start_seconds: self.current_start_seconds,
                end_seconds: self.current_end_seconds,
                language_code: None,
                language_confidence: None,
            });
            had_text = true;
            if finish_after {
                self.finish_segment(current_seconds);
            }
        }
        had_text
    }

    fn has_current_text(&self) -> bool {
        !self.current_text.trim().is_empty()
    }

    fn should_force_finish_segment(&self, current_seconds: f32, max_seconds: f32) -> bool {
        let Some(start_seconds) = self.current_start_seconds else {
            return false;
        };
        self.has_current_text() && current_seconds - start_seconds >= max_seconds
    }

    fn finish_segment(&mut self, current_seconds: f32) {
        let text = self.current_text.trim().to_string();
        if text.is_empty() {
            return;
        }

        let start_seconds = self
            .current_start_seconds
            .filter(|value| value.is_finite())
            .unwrap_or(self.last_segment_end_seconds.min(current_seconds));
        let end_seconds = self
            .current_end_seconds
            .filter(|value| value.is_finite())
            .unwrap_or(current_seconds)
            .max(start_seconds);

        let segment = TimedSegment {
            text: text.clone(),
            start_seconds: Some(start_seconds),
            end_seconds: Some(end_seconds),
            speaker_id: None,
            speaker_label: None,
            language_code: self.current_language_code.clone(),
            language_confidence: self.current_language_confidence,
            words: Vec::new(),
        };

        if let Ok(mut transcript) = self.transcript.lock() {
            if !transcript.trim().is_empty() && !transcript.ends_with('\n') {
                transcript.push('\n');
            }
            transcript.push_str(&text);
        }
        if let Ok(mut segments) = self.segments.lock() {
            segments.push(segment);
        }

        (self.emit_delta)(RealtimeDelta {
            kind: RealtimeDeltaKind::AppendFinal,
            text,
            start_seconds: Some(start_seconds),
            end_seconds: Some(end_seconds),
            language_code: self.current_language_code.clone(),
            language_confidence: self.current_language_confidence,
        });

        self.current_text.clear();
        self.current_start_seconds = None;
        self.current_end_seconds = None;
        self.last_segment_end_seconds = end_seconds;
        self.current_language_code = None;
        self.current_language_confidence = None;
    }
}

#[cfg(test)]
fn normalize_parakeet_live_delta(delta: &str) -> String {
    normalize_parakeet_live_delta_with_language(delta).0
}

#[cfg(test)]
fn normalize_parakeet_live_delta_with_language(
    delta: &str,
) -> (String, Option<String>, Option<String>) {
    let (pieces, pending_fragment) = normalize_parakeet_live_delta_pieces(delta);
    let raw_display = pieces
        .iter()
        .map(|(text, _, _)| text.as_str())
        .collect::<String>();
    let display = if delta.chars().next().is_some_and(char::is_whitespace) {
        let trimmed = raw_display.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            format!(" {trimmed}")
        }
    } else {
        raw_display.trim().to_string()
    };
    let detected_language = pieces
        .iter()
        .rev()
        .find_map(|(_, language, _)| language.clone());
    (display, detected_language, pending_fragment)
}

fn normalize_parakeet_live_delta_pieces(delta: &str) -> (ParakeetLiveDeltaPieces, Option<String>) {
    let mut pieces = ParakeetLiveDeltaPieces::new();
    let mut current = String::new();
    let mut current_language: Option<String> = None;
    let mut pending_fragment = None;
    let mut remaining = delta;

    let push_piece = |pieces: &mut ParakeetLiveDeltaPieces,
                      raw: &str,
                      language: &Option<String>,
                      finish_after: bool| {
        let text = raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if text.is_empty() {
            return;
        }
        let starts_new_word = raw.chars().next().is_some_and(char::is_whitespace);
        let display = if starts_new_word {
            format!(" {text}")
        } else {
            text
        };
        pieces.push((display, language.clone(), finish_after));
    };

    while let Some(start) = remaining.find('<') {
        current.push_str(&remaining[..start]);
        let after_start = &remaining[start..];
        let Some(end) = after_start.find('>') else {
            let had_trailing_space = current.chars().last().is_some_and(char::is_whitespace);
            push_piece(&mut pieces, &current, &current_language, false);
            current.clear();
            if after_start.chars().all(|value| {
                value == '<' || value.is_ascii_alphanumeric() || value == '-' || value == '_'
            }) {
                pending_fragment = Some(after_start.to_string());
            } else {
                if had_trailing_space {
                    current.push(' ');
                }
                current.push_str(after_start);
            }
            remaining = "";
            break;
        };
        let candidate = &after_start[..=end];
        if is_parakeet_language_marker(candidate) {
            let marker = candidate
                .trim_matches(|value: char| {
                    !value.is_ascii_alphanumeric() && value != '-' && value != '_'
                })
                .trim_matches(['<', '>'])
                .trim_matches('|');
            let normalized = LanguageCode::try_from_code(marker)
                .ok()
                .filter(|language| !language.is_auto() && language.as_code() != "und")
                .map(|language| language.as_code().to_string());
            let text_before_marker = current.split_whitespace().collect::<Vec<_>>().join(" ");
            let trailing_is_empty = after_start[end + 1..].chars().all(char::is_whitespace);
            let marker_is_suffix = !text_before_marker.is_empty()
                && (trailing_is_empty || !current.chars().last().is_some_and(char::is_whitespace));
            if marker_is_suffix {
                let language_for_previous = current_language.clone().or(normalized);
                push_piece(&mut pieces, &current, &language_for_previous, true);
                current.clear();
                current_language = None;
            } else {
                let had_trailing_space = current.chars().last().is_some_and(char::is_whitespace);
                push_piece(&mut pieces, &current, &current_language, false);
                current.clear();
                if had_trailing_space {
                    current.push(' ');
                }
                current_language = normalized;
            }
            remaining = trim_marker_trailing_punctuation(&after_start[end + 1..]);
        } else {
            current.push('<');
            remaining = &after_start[1..];
        }
    }
    current.push_str(remaining);
    push_piece(&mut pieces, &current, &current_language, false);
    (pieces, pending_fragment)
}

fn trim_marker_trailing_punctuation(text: &str) -> &str {
    let mut remaining = text;
    loop {
        let Some(next) = remaining.chars().next() else {
            return remaining;
        };
        if !matches!(next, '.' | ',' | ';' | ':' | '!' | '?') {
            return remaining;
        }
        let after_next = &remaining[next.len_utf8()..];
        if after_next
            .chars()
            .next()
            .is_some_and(|value| !value.is_whitespace())
        {
            return remaining;
        }
        remaining = after_next;
    }
}

fn is_parakeet_language_marker(token: &str) -> bool {
    let trimmed = token.trim_matches(|c: char| {
        matches!(
            c,
            '.' | ',' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\''
        )
    });
    let Some(inner) = trimmed
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    else {
        return false;
    };
    let inner = inner
        .strip_prefix('|')
        .and_then(|value| value.strip_suffix('|'))
        .unwrap_or(inner);
    let parts = inner.split(['-', '_']).collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 3 {
        return false;
    }
    let primary = parts[0];
    LanguageCode::try_from_code(inner).is_ok()
        && primary.len() >= 2
        && primary.len() <= 3
        && primary.chars().all(|c| c.is_ascii_alphabetic())
        && parts.iter().skip(1).all(|part| {
            (part.len() == 2 || part.len() == 4) && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

fn build_input_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    channels: usize,
    audio_tx: mpsc::Sender<Vec<f32>>,
    last_error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, BuildStreamError> {
    match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| {
                let _ = audio_tx.send(mix_to_mono(data, channels));
            },
            move |error| store_capture_error(&last_error, error.to_string()),
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                let samples = data
                    .iter()
                    .map(|sample| *sample as f32 / i16::MAX as f32)
                    .collect::<Vec<_>>();
                let _ = audio_tx.send(mix_to_mono(&samples, channels));
            },
            move |error| store_capture_error(&last_error, error.to_string()),
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                let samples = data
                    .iter()
                    .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect::<Vec<_>>();
                let _ = audio_tx.send(mix_to_mono(&samples, channels));
            },
            move |error| store_capture_error(&last_error, error.to_string()),
            None,
        ),
        _ => Err(BuildStreamError::StreamConfigNotSupported),
    }
}

fn store_capture_error(slot: &Arc<Mutex<Option<String>>>, detail: String) {
    if let Ok(mut value) = slot.lock() {
        *value = Some(detail);
    }
}

fn mix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

fn write_pcm_i16(
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    samples: &[f32],
) -> Result<(), ApplicationError> {
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = (clamped * i16::MAX as f32) as i16;
        writer.write_sample(value).map_err(|error| {
            ApplicationError::SpeechToText(format!("failed to write Parakeet live WAV: {error}"))
        })?;
    }
    Ok(())
}

struct LinearResampler {
    input_rate: u32,
    output_rate: u32,
    position: f64,
    carry: Vec<f32>,
}

impl LinearResampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate,
            position: 0.0,
            carry: Vec::new(),
        }
    }

    fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        if self.input_rate == self.output_rate && self.carry.is_empty() {
            return samples.to_vec();
        }

        self.carry.extend_from_slice(samples);
        if self.carry.len() < 2 {
            return Vec::new();
        }

        let step = self.input_rate as f64 / self.output_rate as f64;
        let capacity =
            samples.len().saturating_mul(self.output_rate as usize) / self.input_rate as usize + 8;
        let mut out = Vec::with_capacity(capacity);
        while self.position + 1.0 < self.carry.len() as f64 {
            let i = self.position.floor() as usize;
            let frac = (self.position - i as f64) as f32;
            let sample = self.carry[i] + (self.carry[i + 1] - self.carry[i]) * frac;
            out.push(sample);
            self.position += step;
        }

        let drop_count = (self.position.floor() as usize).min(self.carry.len().saturating_sub(1));
        if drop_count > 0 {
            self.carry.drain(0..drop_count);
            self.position -= drop_count as f64;
        }
        out
    }

    fn finish(&mut self) -> Vec<f32> {
        if let Some(last) = self.carry.last().copied() {
            self.push(&[last])
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parakeet_realtime_metal_safety_env_keeps_metal_enabled() {
        let env = ParakeetRealtimeEngine::safe_metal_environment();
        assert!(env.contains(&("GGML_METAL_NO_RESIDENCY", "1")));
        assert!(env.contains(&("GGML_METAL_SHARED_BUFFERS_DISABLE", "1")));
        assert!(env.contains(&("GGML_METAL_CONCURRENCY_DISABLE", "1")));
        assert!(
            !env.iter().any(|(name, _)| *name == "PARAKEET_DEVICE"),
            "Metal safety must not force the CPU backend"
        );
    }

    #[test]
    fn live_assembler_emits_preview_then_final_segment() {
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let mut assembler = ParakeetLiveAssembler::new(
            transcript.clone(),
            segments.clone(),
            Arc::new(move |delta| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(delta);
            }),
        );

        assembler.push_delta("ciao", 0.4);
        assembler.push_delta(" mondo", 1.2);
        assembler.finish_segment(1.4);

        assert_eq!(
            transcript
                .lock()
                .expect("transcript lock poisoned")
                .as_str(),
            "ciao mondo"
        );
        let segments = segments.lock().expect("segments lock poisoned");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "ciao mondo");
        assert_eq!(segments[0].start_seconds, Some(0.0));
        assert_eq!(segments[0].end_seconds, Some(1.2));

        let emitted = emitted.lock().expect("emitted lock poisoned");
        assert_eq!(emitted.len(), 3);
        assert!(matches!(emitted[0].kind, RealtimeDeltaKind::UpdatePreview));
        assert_eq!(emitted[0].text, "ciao");
        assert!(matches!(emitted[1].kind, RealtimeDeltaKind::UpdatePreview));
        assert_eq!(emitted[1].text, "ciao mondo");
        assert!(matches!(emitted[2].kind, RealtimeDeltaKind::AppendFinal));
        assert_eq!(emitted[2].text, "ciao mondo");
    }

    #[test]
    fn live_assembler_handles_fragmented_markers_and_labels_only_final_text() {
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let mut assembler = ParakeetLiveAssembler::new(
            transcript,
            segments.clone(),
            Arc::new(move |delta| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(delta);
            }),
        );

        assert!(!assembler.push_delta("<it", 0.2));
        assert!(assembler.push_delta("-IT> Ciao", 0.8));
        assert!(assembler.push_delta(" <en-US> Hello", 1.6));
        assembler.finish_segment(2.0);

        let segments = segments.lock().expect("segments lock poisoned");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].language_code.as_deref(), Some("it"));
        assert_eq!(segments[1].language_code.as_deref(), Some("en"));
        let emitted = emitted.lock().expect("emitted lock poisoned");
        assert!(emitted
            .iter()
            .filter(|delta| matches!(delta.kind, RealtimeDeltaKind::UpdatePreview))
            .all(|delta| delta.language_code.is_none()));
        assert!(emitted
            .iter()
            .filter(|delta| matches!(delta.kind, RealtimeDeltaKind::AppendFinal))
            .all(|delta| delta.language_code.is_some()));
    }

    #[test]
    fn live_assembler_finalizes_suffix_marker_before_following_unknown_text() {
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let mut assembler = ParakeetLiveAssembler::new(
            transcript,
            segments.clone(),
            Arc::new(move |delta| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(delta);
            }),
        );

        assert!(assembler.push_delta("Ciao<it> Hello", 1.0));
        assembler.finish_segment(2.0);

        let segments = segments.lock().expect("segments lock poisoned");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Ciao");
        assert_eq!(segments[0].language_code.as_deref(), Some("it"));
        assert_eq!(segments[1].text, "Hello");
        assert!(segments[1].language_code.is_none());
    }

    #[test]
    fn live_assembler_joins_parakeet_subword_deltas_without_spaces() {
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let mut assembler = ParakeetLiveAssembler::new(
            transcript.clone(),
            segments,
            Arc::new(move |delta| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(delta);
            }),
        );

        assembler.push_delta("the cl", 0.4);
        assembler.push_delta("osest point", 0.8);
        assembler.push_delta(" of", 1.2);
        assembler.finish_segment(1.4);

        assert_eq!(
            transcript
                .lock()
                .expect("transcript lock poisoned")
                .as_str(),
            "the closest point of"
        );
        assert_eq!(
            emitted
                .lock()
                .expect("emitted lock poisoned")
                .last()
                .map(|delta| delta.text.as_str()),
            Some("the closest point of")
        );
    }

    #[test]
    fn live_assembler_forces_segments_without_eou() {
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let mut assembler = ParakeetLiveAssembler::new(
            transcript.clone(),
            segments.clone(),
            Arc::new(move |delta| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(delta);
            }),
        );

        assert!(assembler.push_delta("first sentence", 1.0));
        assert!(!assembler.should_force_finish_segment(6.0, 12.0));
        assert!(assembler.should_force_finish_segment(13.1, 12.0));
        assembler.finish_segment(13.1);
        assert!(assembler.push_delta(" second sentence", 14.0));
        assembler.finish_segment(15.0);

        let segments = segments.lock().expect("segments lock poisoned");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "first sentence");
        assert_eq!(segments[1].text, "second sentence");
        assert_eq!(
            transcript
                .lock()
                .expect("transcript lock poisoned")
                .as_str(),
            "first sentence\nsecond sentence"
        );
    }

    #[test]
    fn live_assembler_ignores_empty_finalize() {
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let emitted: Arc<Mutex<Vec<RealtimeDelta>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let mut assembler = ParakeetLiveAssembler::new(
            transcript,
            segments.clone(),
            Arc::new(move |delta| {
                emitted_ref
                    .lock()
                    .expect("emitted lock poisoned")
                    .push(delta);
            }),
        );

        assembler.finish_segment(2.0);

        assert!(segments.lock().expect("segments lock poisoned").is_empty());
        assert!(emitted.lock().expect("emitted lock poisoned").is_empty());
    }

    #[test]
    fn live_delta_normalization_removes_language_prompt_tokens() {
        assert_eq!(
            normalize_parakeet_live_delta(" <it-IT> Ciao mondo"),
            " Ciao mondo"
        );
        assert_eq!(normalize_parakeet_live_delta("<en> hello"), "hello");
        assert_eq!(normalize_parakeet_live_delta("<|it|> ciao"), "ciao");
        assert_eq!(
            normalize_parakeet_live_delta("ciao<it-IT> mondo"),
            "ciao mondo"
        );
        assert_eq!(
            normalize_parakeet_live_delta("<it_IT>ciao <en-US>."),
            "ciao"
        );
        assert_eq!(normalize_parakeet_live_delta("test < b"), "test < b");
    }

    #[test]
    fn live_input_level_events_match_realtime_waveform_contract() {
        let emitted: Arc<Mutex<Vec<RealtimeInputLevelEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let emitted_ref = emitted.clone();
        let emit = move |event: RealtimeInputLevelEvent| {
            emitted_ref
                .lock()
                .expect("input event lock poisoned")
                .push(event);
        };

        emit_parakeet_input_level(&emit, "connecting", 0.0, "Connecting to the microphone...");
        emit_parakeet_input_level(
            &emit,
            "running",
            mean_abs_input_level([0.25_f32, -0.5, 0.75].into_iter()),
            "Using Default microphone",
        );
        emit_parakeet_input_level(&emit, "paused", 0.0, "Microphone preview paused.");
        emit_parakeet_input_level(&emit, "idle", 0.0, "Microphone preview stopped.");

        let emitted = emitted.lock().expect("input event lock poisoned");
        assert_eq!(emitted[0].state, "connecting");
        assert_eq!(emitted[1].state, "running");
        assert!(emitted[1].level > 0.0);
        assert!(emitted[1].level <= 1.0);
        assert_eq!(emitted[2].state, "paused");
        assert_eq!(emitted[3].state, "idle");
    }

    #[test]
    #[ignore = "requires real libparakeet.dylib, realtime GGUF model, and spoken WAV env vars"]
    fn parakeet_realtime_c_api_streams_real_wav() {
        let lib_path = std::env::var("SBOBINO_PARAKEET_LIB")
            .expect("SBOBINO_PARAKEET_LIB must point to libparakeet.dylib");
        let models_dir = std::env::var("SBOBINO_PARAKEET_MODELS_DIR")
            .expect("SBOBINO_PARAKEET_MODELS_DIR must point to Parakeet models");
        let model = std::env::var("SBOBINO_PARAKEET_REALTIME_MODEL")
            .unwrap_or_else(|_| "realtime_eou_120m-v1-f16.gguf".to_string());
        let audio_path = std::env::var("SBOBINO_PARAKEET_AUDIO")
            .expect("SBOBINO_PARAKEET_AUDIO must point to a spoken WAV");

        let engine =
            ParakeetRealtimeEngine::new(PathBuf::from(lib_path), PathBuf::from(models_dir));
        let api = engine.load_api().expect("libparakeet should load");
        let model_path = engine.models_dir.join(model);
        let model_c = CString::new(model_path.to_string_lossy().as_bytes())
            .expect("model path should not contain NUL");
        let ctx = unsafe { (api.load)(model_c.as_ptr()) };
        assert!(!ctx.is_null(), "Parakeet realtime model should load");
        let stream = unsafe { (api.stream_begin)(ctx) };
        assert!(
            !stream.is_null(),
            "Parakeet realtime stream should begin: {}",
            last_error(&api, ctx)
        );

        let samples = read_test_wav_as_16k_mono(Path::new(&audio_path), 24.0);
        assert!(!samples.is_empty(), "test WAV should produce samples");
        let mut combined = String::new();
        let mut saw_eou = false;
        for chunk in samples.chunks(16_000) {
            let mut eou = 0;
            let ptr = unsafe {
                (api.stream_feed)(stream, chunk.as_ptr(), chunk.len() as c_int, &mut eou)
            };
            if let Some(delta) = take_c_string(&api, ptr) {
                let display = normalize_parakeet_live_delta(&delta);
                if !display.trim().is_empty() {
                    println!("parakeet_realtime_delta={}", display.trim());
                    combined = ParakeetLiveAssembler::join_for_test(&combined, &display);
                }
            }
            saw_eou |= eou != 0;
        }
        let ptr = unsafe { (api.stream_finalize)(stream) };
        if let Some(delta) = take_c_string(&api, ptr) {
            let display = normalize_parakeet_live_delta(&delta);
            if !display.trim().is_empty() {
                println!("parakeet_realtime_final={}", display.trim());
                combined = ParakeetLiveAssembler::join_for_test(&combined, &display);
            }
        }
        unsafe {
            (api.stream_free)(stream);
            (api.free)(ctx);
        }

        assert!(
            !combined.trim().is_empty(),
            "Parakeet realtime C API produced no transcript"
        );
        println!("parakeet_realtime_text={combined}");
        println!("parakeet_realtime_saw_eou={saw_eou}");
    }

    fn read_test_wav_as_16k_mono(path: &Path, max_seconds: f32) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).expect("failed to open test WAV");
        let spec = reader.spec();
        let channels = usize::from(spec.channels.max(1));
        let max_samples = (spec.sample_rate as f32 * max_seconds) as usize * channels;
        let mono = match spec.sample_format {
            hound::SampleFormat::Float => {
                let samples = reader
                    .samples::<f32>()
                    .take(max_samples)
                    .map(|sample| sample.expect("failed to read float sample"))
                    .collect::<Vec<_>>();
                mix_to_mono(&samples, channels)
            }
            hound::SampleFormat::Int if spec.bits_per_sample <= 16 => {
                let samples = reader
                    .samples::<i16>()
                    .take(max_samples)
                    .map(|sample| {
                        sample.expect("failed to read i16 sample") as f32 / i16::MAX as f32
                    })
                    .collect::<Vec<_>>();
                mix_to_mono(&samples, channels)
            }
            hound::SampleFormat::Int => {
                let scale = ((1_i64 << (spec.bits_per_sample.saturating_sub(1))) - 1) as f32;
                let samples = reader
                    .samples::<i32>()
                    .take(max_samples)
                    .map(|sample| sample.expect("failed to read i32 sample") as f32 / scale)
                    .collect::<Vec<_>>();
                mix_to_mono(&samples, channels)
            }
        };
        let mut resampler = LinearResampler::new(spec.sample_rate, 16_000);
        let mut output = resampler.push(&mono);
        output.extend(resampler.finish());
        output
    }

    impl ParakeetLiveAssembler {
        fn join_for_test(left: &str, right: &str) -> String {
            let left = left.trim();
            let starts_new_word = right.chars().next().is_some_and(char::is_whitespace);
            let right = right.trim();
            if left.is_empty() {
                right.to_string()
            } else if right.is_empty() || left.contains(right) {
                left.to_string()
            } else if starts_new_word {
                format!("{left} {right}")
            } else {
                format!("{left}{right}")
            }
        }
    }
}
