use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::WhisperOptions;

#[derive(Debug, Clone)]
pub struct WhisperCppEngine {
    binary_path: String,
    models_dir: String,
}

#[derive(Default)]
struct TranscriptCollector {
    lines: Vec<String>,
}

struct ParsedCliLine {
    text: String,
    end_seconds: Option<f32>,
}

impl WhisperCppEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
        }
    }

    fn model_path(&self, model_filename: &str) -> PathBuf {
        Path::new(&self.models_dir).join(model_filename)
    }

    fn validate_model_exists(&self, model_filename: &str) -> Result<PathBuf, ApplicationError> {
        let model_path = self.model_path(model_filename);
        if model_path.exists() {
            return Ok(model_path);
        }

        let download_url =
            format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{model_filename}");
        Err(ApplicationError::SpeechToText(format!(
            "model file not found at {}. Download it from {}",
            model_path.display(),
            download_url
        )))
    }

    fn parse_timecode_seconds(value: &str) -> Option<f32> {
        let mut parts = value.trim().split(':');
        let hh = parts.next()?.parse::<f32>().ok()?;
        let mm = parts.next()?.parse::<f32>().ok()?;
        let ss = parts.next()?.parse::<f32>().ok()?;
        Some((hh * 3600.0) + (mm * 60.0) + ss)
    }

    fn parse_cli_line(raw_line: &str) -> Option<ParsedCliLine> {
        let cleaned = raw_line
            .replace("\u{001b}[2K", "")
            .replace("\u{001b}[0m", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "")
            .trim()
            .to_string();

        if cleaned.is_empty() {
            return None;
        }

        const NOISE_PREFIXES: [&str; 10] = [
            "init:",
            "main:",
            "whisper_",
            "ggml_",
            "system_info:",
            "output_",
            "sampling_",
            "encode",
            "decode",
            "progress",
        ];

        if NOISE_PREFIXES
            .iter()
            .any(|prefix| cleaned.starts_with(prefix))
        {
            return None;
        }

        let mut end_seconds = None;

        let without_timestamp = if cleaned.starts_with('[') {
            match cleaned.find(']') {
                Some(end_index) => {
                    let bracket_content = cleaned[1..end_index].trim();
                    if bracket_content.contains("-->") {
                        if let Some((_, end_value)) = bracket_content.split_once("-->") {
                            end_seconds = Self::parse_timecode_seconds(end_value.trim());
                        }
                        cleaned[end_index + 1..].trim().to_string()
                    } else {
                        cleaned
                    }
                }
                None => cleaned,
            }
        } else {
            cleaned
        };

        let normalized = without_timestamp.trim().to_string();
        if normalized.is_empty() {
            return None;
        }

        Some(ParsedCliLine {
            text: normalized,
            end_seconds,
        })
    }

    fn collect_line(
        collector: &Arc<Mutex<TranscriptCollector>>,
        emit_partial: &Arc<dyn Fn(String) + Send + Sync>,
        line: String,
    ) {
        if let Ok(mut state) = collector.lock() {
            state.lines.push(line.clone());
        }

        emit_partial(line);
    }

    async fn consume_stream<R>(
        reader: R,
        collector: Arc<Mutex<TranscriptCollector>>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<Vec<String>, ApplicationError>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = BufReader::new(reader);
        let mut chunk = [0_u8; 4096];
        let mut pending = Vec::<u8>::new();
        let mut raw_lines = Vec::<String>::new();

        loop {
            let read = reader.read(&mut chunk).await.map_err(|e| {
                ApplicationError::SpeechToText(format!("failed to read whisper-cli stream: {e}"))
            })?;

            if read == 0 {
                break;
            }

            pending.extend_from_slice(&chunk[..read]);
            let mut start = 0_usize;
            let mut index = 0_usize;

            while index < pending.len() {
                if pending[index] == b'\n' || pending[index] == b'\r' {
                    if index > start {
                        let raw = String::from_utf8_lossy(&pending[start..index]).to_string();
                        raw_lines.push(raw.clone());
                        if let Some(parsed_line) = Self::parse_cli_line(&raw) {
                            if let Some(end_seconds) = parsed_line.end_seconds {
                                emit_progress_seconds(end_seconds);
                            }
                            Self::collect_line(&collector, &emit_partial, parsed_line.text);
                        }
                    }

                    index += 1;
                    while index < pending.len()
                        && (pending[index] == b'\n' || pending[index] == b'\r')
                    {
                        index += 1;
                    }
                    start = index;
                    continue;
                }
                index += 1;
            }

            if start > 0 {
                pending.drain(..start);
            }
        }

        if !pending.is_empty() {
            let raw = String::from_utf8_lossy(&pending).to_string();
            raw_lines.push(raw.clone());
            if let Some(parsed_line) = Self::parse_cli_line(&raw) {
                if let Some(end_seconds) = parsed_line.end_seconds {
                    emit_progress_seconds(end_seconds);
                }
                Self::collect_line(&collector, &emit_partial, parsed_line.text);
            }
        }

        Ok(raw_lines)
    }

    fn normalized_options(options: &WhisperOptions) -> WhisperOptions {
        let mut normalized = options.clone();

        normalized.temperature = normalized.temperature.clamp(0.0, 1.0);
        normalized.entropy_threshold = normalized.entropy_threshold.clamp(0.0, 10.0);
        normalized.logprob_threshold = normalized.logprob_threshold.clamp(-10.0, 0.0);
        normalized.word_threshold = normalized.word_threshold.clamp(0.0, 1.0);
        normalized.best_of = normalized.best_of.clamp(1, 20);
        normalized.beam_size = normalized.beam_size.clamp(1, 20);
        normalized.threads = normalized.threads.clamp(1, 32);
        normalized.processors = normalized.processors.clamp(1, 16);

        normalized
    }

    async fn transcribe_with_cli(
        &self,
        input_wav: &Path,
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<String, ApplicationError> {
        let output_base = std::env::temp_dir().join(format!(
            "sbobino-whisper-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        ));
        let output_txt_path = output_base.with_extension("txt");

        let mut command = Command::new(&self.binary_path);

        // Homebrew-installed whisper-cli links against @rpath/libggml.0.dylib but
        // ships with no embedded rpath. We resolve this by setting DYLD_LIBRARY_PATH
        // to the sibling libexec/lib directory where the dylibs actually live.
        if let Some(binary_dir) = Path::new(&self.binary_path)
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let libexec_lib = binary_dir.join("../libexec/lib");
            let sibling_lib = binary_dir.join("../lib");

            let mut dyld_paths = Vec::new();
            if libexec_lib.exists() {
                dyld_paths.push(libexec_lib.to_string_lossy().to_string());
            }
            if sibling_lib.exists() {
                dyld_paths.push(sibling_lib.to_string_lossy().to_string());
            }
            // Also preserve any existing DYLD_LIBRARY_PATH
            if let Ok(existing) = std::env::var("DYLD_LIBRARY_PATH") {
                dyld_paths.push(existing);
            }
            if !dyld_paths.is_empty() {
                command.env("DYLD_LIBRARY_PATH", dyld_paths.join(":"));
            }
        }

        command
            .kill_on_drop(true)
            .arg("-m")
            .arg(model_path)
            .arg("-f")
            .arg(input_wav);

        let options = Self::normalized_options(options);

        command
            .arg("-t")
            .arg(options.threads.to_string())
            .arg("-p")
            .arg(options.processors.to_string())
            .arg("-tp")
            .arg(options.temperature.to_string())
            .arg("-et")
            .arg(options.entropy_threshold.to_string())
            .arg("-lpt")
            .arg(options.logprob_threshold.to_string())
            .arg("-wt")
            .arg(options.word_threshold.to_string());

        if language_code != "auto" {
            command.arg("-l").arg(language_code);
        }

        if options.translate_to_english {
            command.arg("-tr");
        }
        if options.no_context {
            command.arg("-mc").arg("0");
        }
        if options.split_on_word {
            command.arg("-sow");
        }
        if options.beam_size > 1 {
            command.arg("-bs").arg(options.beam_size.to_string());
        } else if options.best_of > 1 {
            command.arg("-bo").arg(options.best_of.to_string());
        }

        command
            .arg("-otxt")
            .arg("-of")
            .arg(&output_base)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = command.spawn().map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "whisper-cli failed to start at '{}': {e}. Configure Whisper CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisper-cli stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisper-cli stderr pipe".to_string())
        })?;

        let collected = Arc::new(Mutex::new(TranscriptCollector::default()));

        let stdout_emit = emit_partial.clone();
        let stdout_progress = emit_progress_seconds.clone();
        let stdout_collector = collected.clone();
        let stdout_task = tokio::spawn(async move {
            Self::consume_stream(stdout, stdout_collector, stdout_emit, stdout_progress).await
        });

        let stderr_emit = emit_partial.clone();
        let stderr_progress = emit_progress_seconds.clone();
        let stderr_collector = collected.clone();
        let stderr_task = tokio::spawn(async move {
            Self::consume_stream(stderr, stderr_collector, stderr_emit, stderr_progress).await
        });

        let status = match timeout(Duration::from_secs(900), child.wait()).await {
            Ok(wait_result) => wait_result.map_err(|e| {
                ApplicationError::SpeechToText(format!("failed to wait for whisper-cli: {e}"))
            })?,
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ApplicationError::SpeechToText(
                    "whisper-cli timed out after 900s".to_string(),
                ));
            }
        };

        let _stdout_lines = stdout_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("stdout reader task failed: {e}"))
        })??;

        let stderr_lines = stderr_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("stderr reader task failed: {e}"))
        })??;
        let stderr_output = stderr_lines.join("\n");

        let transcript_lines = if let Ok(state) = collected.lock() {
            state.lines.clone()
        } else {
            Vec::new()
        };

        if !status.success() {
            return Err(ApplicationError::SpeechToText(format!(
                "whisper-cli failed: {}",
                stderr_output.trim()
            )));
        }

        let transcript_from_file = match fs::read_to_string(&output_txt_path).await {
            Ok(content) => {
                let cleaned = content.trim().to_string();
                if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                }
            }
            Err(_) => None,
        };

        let transcript =
            transcript_from_file.unwrap_or_else(|| transcript_lines.join("\n").trim().to_string());

        let _ = fs::remove_file(&output_txt_path).await;

        if transcript.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "whisper-cli produced empty output".to_string(),
            ));
        }

        Ok(transcript)
    }
}

#[async_trait]
impl SpeechToTextEngine for WhisperCppEngine {
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_code: &str,
        options: &WhisperOptions,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<String, ApplicationError> {
        let model_path = self.validate_model_exists(model_filename)?;
        self.transcribe_with_cli(
            input_wav,
            &model_path,
            language_code,
            options,
            emit_partial,
            emit_progress_seconds,
        )
        .await
    }
}
