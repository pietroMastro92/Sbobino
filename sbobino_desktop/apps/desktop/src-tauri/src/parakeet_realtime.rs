use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_float, c_int};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BuildStreamError, SampleFormat, Stream, StreamConfig,
};
use libloading::Library;

use sbobino_application::{ApplicationError, RealtimeDelta, RealtimeDeltaKind};
use sbobino_domain::TimedSegment;

type ParakeetCtx = std::ffi::c_void;
type ParakeetStream = std::ffi::c_void;

type LoadFn = unsafe extern "C" fn(*const c_char) -> *mut ParakeetCtx;
type FreeFn = unsafe extern "C" fn(*mut ParakeetCtx);
type StreamBeginFn = unsafe extern "C" fn(*mut ParakeetCtx) -> *mut ParakeetStream;
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
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    ) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().map_err(lock_error)?;
        if state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is already running".to_string(),
            ));
        }

        let api = self.load_api()?;
        let model_path = self.models_dir.join(model_filename);
        if !model_path.exists() {
            return Err(ApplicationError::SpeechToText(format!(
                "Parakeet realtime model file not found at {}",
                model_path.display()
            )));
        }

        let session_dir = create_session_dir()?;
        let saved_audio_path = session_dir.join("parakeet-live.wav");
        let transcript = Arc::new(Mutex::new(String::new()));
        let segments = Arc::new(Mutex::new(Vec::new()));
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let paused = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let (startup_tx, startup_rx) = mpsc::channel();
        let model_for_thread = model_path.clone();
        let audio_for_thread = saved_audio_path.clone();
        let transcript_for_thread = transcript.clone();
        let segments_for_thread = segments.clone();
        let diagnostics_for_thread = diagnostics.clone();
        let paused_for_thread = paused.clone();

        let worker = thread::spawn(move || {
            let result = run_parakeet_capture(
                api,
                model_for_thread,
                audio_for_thread,
                shutdown_rx,
                startup_tx.clone(),
                paused_for_thread,
                transcript_for_thread,
                segments_for_thread,
                diagnostics_for_thread,
                emit_delta,
            );
            if let Err(error) = result {
                let _ = startup_tx.send(Err(error));
            }
        });

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

    fn load_api(&self) -> Result<ParakeetApi, ApplicationError> {
        if !self.lib_path.exists() {
            return Err(ApplicationError::SpeechToText(format!(
                "Parakeet live library not found at {}. Reinstall the local runtime.",
                self.lib_path.display()
            )));
        }

        // ggml Metal residency sets can assert during in-process teardown on
        // some Apple Silicon/macOS combinations. Keep Metal enabled, but use
        // the upstream opt-out for residency-set bookkeeping in live mode.
        std::env::set_var("GGML_METAL_NO_RESIDENCY", "1");

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
    audio_path: PathBuf,
    shutdown_rx: mpsc::Receiver<()>,
    startup_tx: mpsc::Sender<Result<(), ApplicationError>>,
    paused: Arc<AtomicBool>,
    transcript: Arc<Mutex<String>>,
    segments: Arc<Mutex<Vec<TimedSegment>>>,
    diagnostics: Arc<Mutex<Vec<String>>>,
    emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
) -> Result<(), ApplicationError> {
    let model_c = CString::new(model_path.to_string_lossy().as_bytes()).map_err(|_| {
        ApplicationError::SpeechToText("Parakeet model path contains a NUL byte".to_string())
    })?;
    let ctx = unsafe { (api.load)(model_c.as_ptr()) };
    if ctx.is_null() {
        return Err(ApplicationError::SpeechToText(format!(
            "failed to load Parakeet realtime model {}",
            model_path.display()
        )));
    }

    let stream = unsafe { (api.stream_begin)(ctx) };
    if stream.is_null() {
        let detail = last_error(&api, ctx);
        unsafe { (api.free)(ctx) };
        return Err(ApplicationError::SpeechToText(format!(
            "failed to start Parakeet realtime stream for {}: {}",
            model_path.display(),
            if detail.is_empty() {
                "model is not a cache-aware streaming model"
            } else {
                detail.as_str()
            }
        )));
    }

    let result = run_capture_loop(
        &api,
        ctx,
        stream,
        &audio_path,
        shutdown_rx,
        startup_tx,
        paused,
        transcript,
        segments,
        diagnostics,
        emit_delta,
    );

    unsafe {
        (api.stream_free)(stream);
        (api.free)(ctx);
    }

    result
}

fn run_capture_loop(
    api: &ParakeetApi,
    ctx: *mut ParakeetCtx,
    stream: *mut ParakeetStream,
    audio_path: &Path,
    shutdown_rx: mpsc::Receiver<()>,
    startup_tx: mpsc::Sender<Result<(), ApplicationError>>,
    paused: Arc<AtomicBool>,
    transcript: Arc<Mutex<String>>,
    segments: Arc<Mutex<Vec<TimedSegment>>>,
    diagnostics: Arc<Mutex<Vec<String>>>,
    emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
) -> Result<(), ApplicationError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| {
        ApplicationError::SpeechToText("No audio input device is available.".to_string())
    })?;
    let supported_config = device.default_input_config().map_err(|error| {
        ApplicationError::SpeechToText(format!(
            "failed to read default microphone config for Parakeet live: {error}"
        ))
    })?;
    let config = supported_config.config();
    let sample_rate = config.sample_rate.0;
    let channels = usize::from(config.channels.max(1));
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
        ApplicationError::SpeechToText(format!("Parakeet live microphone setup failed: {error}"))
    })?;

    input_stream.play().map_err(|error| {
        ApplicationError::SpeechToText(format!("Parakeet live microphone start failed: {error}"))
    })?;

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(audio_path, spec).map_err(|error| {
        ApplicationError::SpeechToText(format!(
            "failed to create Parakeet live WAV at {}: {error}",
            audio_path.display()
        ))
    })?;

    let mut resampler = LinearResampler::new(sample_rate, 16_000);
    let mut processed_samples: u64 = 0;
    let mut assembler = ParakeetLiveAssembler::new(transcript, segments, emit_delta);
    let _stream_guard = input_stream;
    let _ = startup_tx.send(Ok(()));

    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        if let Ok(mut slot) = last_capture_error.lock() {
            if let Some(error) = slot.take() {
                if let Ok(mut items) = diagnostics.lock() {
                    items.push(error);
                }
            }
        }

        let chunk = match audio_rx.recv_timeout(Duration::from_millis(60)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if paused.load(Ordering::Relaxed) {
            continue;
        }

        let pcm_16k = resampler.push(&chunk);
        if pcm_16k.is_empty() {
            continue;
        }
        processed_samples = processed_samples.saturating_add(pcm_16k.len() as u64);
        let current_seconds = processed_samples as f32 / 16_000.0;
        write_pcm_i16(&mut writer, &pcm_16k)?;

        let mut eou = 0;
        let ptr = unsafe {
            (api.stream_feed)(
                stream,
                pcm_16k.as_ptr(),
                pcm_16k.len() as c_int,
                &mut eou as *mut c_int,
            )
        };
        let delta = take_c_string(api, ptr).ok_or_else(|| {
            ApplicationError::SpeechToText(format!(
                "Parakeet live stream feed failed: {}",
                last_error(api, ctx)
            ))
        })?;
        assembler.push_delta(&delta, current_seconds);
        if eou != 0 {
            assembler.finish_segment(current_seconds);
        }
    }

    let tail = resampler.finish();
    if !tail.is_empty() {
        processed_samples = processed_samples.saturating_add(tail.len() as u64);
        let current_seconds = processed_samples as f32 / 16_000.0;
        write_pcm_i16(&mut writer, &tail)?;
        let mut eou = 0;
        let ptr = unsafe {
            (api.stream_feed)(
                stream,
                tail.as_ptr(),
                tail.len() as c_int,
                &mut eou as *mut c_int,
            )
        };
        let delta = take_c_string(api, ptr).ok_or_else(|| {
            ApplicationError::SpeechToText(format!(
                "Parakeet live stream final feed failed: {}",
                last_error(api, ctx)
            ))
        })?;
        assembler.push_delta(&delta, current_seconds);
    }

    let ptr = unsafe { (api.stream_finalize)(stream) };
    let delta = take_c_string(api, ptr).ok_or_else(|| {
        ApplicationError::SpeechToText(format!(
            "Parakeet live stream finalize failed: {}",
            last_error(api, ctx)
        ))
    })?;
    let final_seconds = processed_samples as f32 / 16_000.0;
    assembler.push_delta(&delta, final_seconds);
    assembler.finish_segment(final_seconds);
    writer.finalize().map_err(|error| {
        ApplicationError::SpeechToText(format!(
            "failed to finalize Parakeet live WAV at {}: {error}",
            audio_path.display()
        ))
    })?;
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
        }
    }

    fn push_delta(&mut self, delta: &str, current_seconds: f32) {
        let display = normalize_parakeet_live_delta(delta);
        if display.is_empty() {
            return;
        }

        if self.current_start_seconds.is_none() {
            self.current_start_seconds = Some(self.last_segment_end_seconds.min(current_seconds));
        }
        self.current_end_seconds = Some(current_seconds.max(self.last_segment_end_seconds));
        if !self.current_text.is_empty()
            && !self
                .current_text
                .chars()
                .last()
                .is_some_and(char::is_whitespace)
        {
            self.current_text.push(' ');
        }
        self.current_text.push_str(&display);

        (self.emit_delta)(RealtimeDelta {
            kind: RealtimeDeltaKind::UpdatePreview,
            text: self.current_text.clone(),
            start_seconds: self.current_start_seconds,
            end_seconds: self.current_end_seconds,
        });
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
        });

        self.current_text.clear();
        self.current_start_seconds = None;
        self.current_end_seconds = None;
        self.last_segment_end_seconds = end_seconds;
    }
}

fn normalize_parakeet_live_delta(delta: &str) -> String {
    delta
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
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

        let drop_count = self.position.floor() as usize;
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
        assembler.push_delta("mondo", 1.2);
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
}
