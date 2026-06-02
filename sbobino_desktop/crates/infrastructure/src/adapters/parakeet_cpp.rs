use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{TimedSegment, TimedWord, TranscriptionOutput, WhisperOptions};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;

#[derive(Debug, Clone)]
pub struct ParakeetCppEngine {
    binary_path: String,
    models_dir: String,
}

#[derive(Debug, Deserialize, Default)]
struct ParakeetJsonOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<ParakeetJsonWord>,
    #[serde(default)]
    segments: Vec<ParakeetJsonSegment>,
}

#[derive(Debug, Deserialize, Default)]
struct ParakeetJsonSegment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: Option<f32>,
    #[serde(default)]
    end: Option<f32>,
    #[serde(default)]
    words: Vec<ParakeetJsonWord>,
}

#[derive(Debug, Deserialize, Default)]
struct ParakeetJsonWord {
    #[serde(default, alias = "text")]
    w: String,
    #[serde(default)]
    start: Option<f32>,
    #[serde(default)]
    end: Option<f32>,
    #[serde(default, alias = "confidence")]
    conf: Option<f32>,
}

impl ParakeetCppEngine {
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

        let download_url = format!(
            "https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/{model_filename}"
        );
        Err(ApplicationError::SpeechToText(format!(
            "Parakeet model file not found at {}. Download it from {}",
            model_path.display(),
            download_url
        )))
    }

    fn configure_command_environment(command: &mut Command, binary_path: &str) {
        if let Some(binary_dir) = Path::new(binary_path)
            .canonicalize()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
        {
            let sibling_lib = binary_dir.join("../lib");
            let mut dyld_paths = vec![binary_dir.to_string_lossy().to_string()];
            if sibling_lib.exists() {
                dyld_paths.push(sibling_lib.to_string_lossy().to_string());
            }
            if let Ok(existing) = std::env::var("DYLD_LIBRARY_PATH") {
                dyld_paths.push(existing);
            }
            command.env("DYLD_LIBRARY_PATH", dyld_paths.join(":"));
        }
    }

    fn extract_json_payload(stdout: &str) -> Result<&str, ApplicationError> {
        let start = stdout.find('{').ok_or_else(|| {
            ApplicationError::SpeechToText("parakeet-cli produced no JSON output".to_string())
        })?;
        let end = stdout.rfind('}').ok_or_else(|| {
            ApplicationError::SpeechToText(
                "parakeet-cli produced incomplete JSON output".to_string(),
            )
        })?;
        if end < start {
            return Err(ApplicationError::SpeechToText(
                "parakeet-cli produced malformed JSON output".to_string(),
            ));
        }
        Ok(&stdout[start..=end])
    }

    fn parse_json_output(
        raw_stdout: &str,
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let payload = Self::extract_json_payload(raw_stdout)?;
        let parsed: ParakeetJsonOutput = serde_json::from_str(payload).map_err(|error| {
            ApplicationError::SpeechToText(format!("failed to parse parakeet-cli JSON: {error}"))
        })?;

        let text = parsed.text.trim().to_string();
        let segment_text = parsed
            .segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let text = if text.is_empty() { segment_text } else { text };
        if text.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "parakeet-cli produced empty output".to_string(),
            ));
        }

        let raw_segments = if parsed.segments.is_empty() {
            Self::segments_from_words(text.clone(), parsed.words, total_audio_seconds)
        } else {
            parsed
                .segments
                .into_iter()
                .filter_map(Self::segment_from_json)
                .collect::<Vec<_>>()
        };

        let raw_segments = if raw_segments.is_empty() {
            vec![TimedSegment {
                text: text.clone(),
                start_seconds: None,
                end_seconds: total_audio_seconds,
                speaker_id: None,
                speaker_label: None,
                words: Vec::new(),
            }]
        } else {
            raw_segments
        };

        Ok(TranscriptionOutput {
            text: text.clone(),
            segments: normalize_transcript_segments(&text, &raw_segments, total_audio_seconds),
        })
    }

    fn segments_from_words(
        text: String,
        words: Vec<ParakeetJsonWord>,
        total_audio_seconds: Option<f32>,
    ) -> Vec<TimedSegment> {
        let words = words
            .into_iter()
            .filter_map(Self::word_from_json)
            .collect::<Vec<_>>();

        if words.is_empty() {
            return vec![TimedSegment {
                text,
                start_seconds: None,
                end_seconds: total_audio_seconds,
                speaker_id: None,
                speaker_label: None,
                words,
            }];
        }

        vec![TimedSegment {
            text,
            start_seconds: words.iter().find_map(|word| word.start_seconds),
            end_seconds: words.iter().rev().find_map(|word| word.end_seconds),
            speaker_id: None,
            speaker_label: None,
            words,
        }]
    }

    fn segment_from_json(segment: ParakeetJsonSegment) -> Option<TimedSegment> {
        let text = segment.text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let words = segment
            .words
            .into_iter()
            .filter_map(Self::word_from_json)
            .collect::<Vec<_>>();

        Some(TimedSegment {
            text,
            start_seconds: segment.start.filter(|value| value.is_finite()),
            end_seconds: segment.end.filter(|value| value.is_finite()),
            speaker_id: None,
            speaker_label: None,
            words,
        })
    }

    fn word_from_json(word: ParakeetJsonWord) -> Option<TimedWord> {
        let text = word.w.trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(TimedWord {
            text,
            start_seconds: word.start.filter(|value| value.is_finite()),
            end_seconds: word.end.filter(|value| value.is_finite()),
            confidence: word.conf.filter(|value| value.is_finite()),
        })
    }
}

#[async_trait]
impl SpeechToTextEngine for ParakeetCppEngine {
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        _language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        if options.translate_to_english {
            return Err(ApplicationError::SpeechToText(
                "Parakeet.cpp does not support translate-to-English mode".to_string(),
            ));
        }

        let model_path = self.validate_model_exists(model_filename)?;
        let mut command = Command::new(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(&model_path)
            .arg("--input")
            .arg(input_wav)
            .arg("--json")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = command.output().await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-cli failed to start at '{}': {error}. Configure Parakeet CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-cli failed: {}",
                if stderr.is_empty() {
                    output.status.to_string()
                } else {
                    stderr
                }
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result = Self::parse_json_output(&stdout, total_audio_seconds)?;
        emit_partial(result.text.clone());
        if let Some(total) = total_audio_seconds {
            emit_progress_seconds(total);
        }
        Ok(result)
    }
}
