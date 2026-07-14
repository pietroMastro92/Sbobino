use std::ffi::OsString;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, SpeakerDiarizationEngine};
use sbobino_domain::SpeakerTurn;

use crate::background_process::tokio_background_command;

pub const EMBEDDED_HELPER_FILENAME: &str = "pyannote_diarize.py";
const PYTHON_ENV_VARS_TO_CLEAR: &[&str] = &[
    "PYTHONPATH",
    "PYTHONEXECUTABLE",
    "PYTHONHOME",
    "PYTHONNOUSERSITE",
    "PYTHONUSERBASE",
    "PYTHONSTARTUP",
    "PYTHONPLATLIBDIR",
    "PYTHONPYCACHEPREFIX",
    "PYTHONBREAKPOINT",
    "__PYVENV_LAUNCHER__",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "CONDA_DEFAULT_ENV",
];

pub fn embedded_helper_script() -> &'static str {
    include_str!("../../../../scripts/pyannote_diarize.py")
}

#[derive(Debug, Clone)]
pub struct PyannoteSpeakerDiarizationEngine {
    python_path: String,
    python_home: Option<PathBuf>,
    python_path_env: Option<OsString>,
    script_path: String,
    model_path: String,
    device: String,
    path_prepend: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct PyannoteOutput {
    #[serde(default)]
    speakers: Vec<PyannoteSpeakerTurn>,
}

#[derive(Debug, Deserialize)]
struct PyannoteSpeakerTurn {
    speaker_id: String,
    #[serde(default)]
    speaker_label: Option<String>,
    start_seconds: f32,
    end_seconds: f32,
}

impl PyannoteSpeakerDiarizationEngine {
    pub fn new(
        python_path: String,
        python_home: Option<PathBuf>,
        python_path_env: Option<OsString>,
        script_path: String,
        model_path: String,
        device: String,
        path_prepend: Vec<PathBuf>,
    ) -> Self {
        Self {
            python_path,
            python_home,
            python_path_env,
            script_path,
            model_path,
            device,
            path_prepend,
        }
    }

    fn build_path_env(&self) -> Option<OsString> {
        if self.path_prepend.is_empty() {
            return None;
        }

        let mut entries = self.path_prepend.clone();
        if let Some(existing) = std::env::var_os("PATH") {
            entries.extend(std::env::split_paths(&existing));
        }

        std::env::join_paths(entries).ok()
    }

    fn parse_turns(stdout: &[u8]) -> Result<Vec<SpeakerTurn>, ApplicationError> {
        let parsed = serde_json::from_slice::<PyannoteOutput>(stdout).map_err(|error| {
            ApplicationError::SpeakerDiarization(format!(
                "pyannote helper produced invalid JSON: {error}"
            ))
        })?;

        Ok(parsed
            .speakers
            .into_iter()
            .filter(|turn| {
                turn.start_seconds.is_finite()
                    && turn.end_seconds.is_finite()
                    && turn.end_seconds > turn.start_seconds
                    && !turn.speaker_id.trim().is_empty()
            })
            .map(|turn| SpeakerTurn {
                speaker_id: turn.speaker_id.trim().to_string(),
                speaker_label: turn
                    .speaker_label
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                start_seconds: turn.start_seconds.max(0.0),
                end_seconds: turn.end_seconds.max(0.0),
            })
            .collect())
    }
}

#[async_trait]
impl SpeakerDiarizationEngine for PyannoteSpeakerDiarizationEngine {
    async fn diarize(&self, input_wav: &Path) -> Result<Vec<SpeakerTurn>, ApplicationError> {
        let mut command = tokio_background_command(&self.python_path);
        command
            .arg(&self.script_path)
            .arg("--audio-path")
            .arg(input_wav)
            .arg("--model-path")
            .arg(&self.model_path)
            .arg("--device")
            .arg(if self.device.trim().is_empty() {
                "cpu"
            } else {
                self.device.trim()
            })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        if let Some(path_env) = self.build_path_env() {
            command.env("PATH", path_env);
        }
        for key in PYTHON_ENV_VARS_TO_CLEAR {
            command.env_remove(key);
        }
        if let Some(python_home) = &self.python_home {
            command.env("PYTHONHOME", python_home);
        }
        if let Some(python_path_env) = &self.python_path_env {
            command.env("PYTHONPATH", python_path_env);
        }
        command.env("PYTHONNOUSERSITE", "1");

        let child = command.spawn().map_err(|error| {
            ApplicationError::SpeakerDiarization(format!(
                "failed to start pyannote helper with '{}': {error}",
                self.python_path
            ))
        })?;

        let output = timeout(Duration::from_secs(1800), child.wait_with_output())
            .await
            .map_err(|_| {
                ApplicationError::SpeakerDiarization(
                    "pyannote helper timed out after 1800s".to_string(),
                )
            })?
            .map_err(|error| {
                ApplicationError::SpeakerDiarization(format!(
                    "failed to wait for pyannote helper: {error}"
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ApplicationError::SpeakerDiarization(if stderr.is_empty() {
                format!("pyannote helper exited with status {}", output.status)
            } else {
                format!("pyannote helper failed: {stderr}")
            }));
        }

        Self::parse_turns(&output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::PyannoteSpeakerDiarizationEngine;
    use std::path::PathBuf;

    #[test]
    fn parse_turns_discards_invalid_entries() {
        let output = br#"{
          "speakers": [
            {"speaker_id":"speaker_1","speaker_label":"Speaker 1","start_seconds":0.0,"end_seconds":1.2},
            {"speaker_id":"","speaker_label":"Invalid","start_seconds":1.2,"end_seconds":2.0},
            {"speaker_id":"speaker_2","start_seconds":3.0,"end_seconds":2.0}
          ]
        }"#;

        let turns = PyannoteSpeakerDiarizationEngine::parse_turns(output)
            .expect("valid payload should parse");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].speaker_id, "speaker_1");
        assert_eq!(turns[0].speaker_label.as_deref(), Some("Speaker 1"));
    }

    #[test]
    fn build_path_env_prepends_custom_entries() {
        let engine = PyannoteSpeakerDiarizationEngine::new(
            "python3".to_string(),
            None,
            None,
            "helper.py".to_string(),
            "model".to_string(),
            "cpu".to_string(),
            vec![PathBuf::from("/tmp/ffmpeg-bin")],
        );

        let path_env = engine.build_path_env().expect("path should build");
        let entries = std::env::split_paths(&path_env).collect::<Vec<_>>();

        assert_eq!(entries.first(), Some(&PathBuf::from("/tmp/ffmpeg-bin")));
    }
}
