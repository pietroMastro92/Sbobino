use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, DiarizationProgress, SpeakerDiarizationEngine};
use sbobino_domain::SpeakerTurn;

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
const PROGRESS_PREFIX: &str = "SBOBINO_DIARIZATION_PROGRESS ";

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
    async fn diarize(
        &self,
        input_wav: &Path,
        emit_progress: Arc<dyn Fn(DiarizationProgress) + Send + Sync>,
    ) -> Result<Vec<SpeakerTurn>, ApplicationError> {
        let mut command = Command::new(&self.python_path);
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
            .arg("--batch-size")
            .arg(Self::adaptive_batch_size(input_wav).to_string())
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
        let worker_threads = std::thread::available_parallelism()
            .map(|count| if count.get() >= 8 { 4 } else { 2 })
            .unwrap_or(2)
            .to_string();
        command.env("OMP_NUM_THREADS", &worker_threads);
        command.env("MKL_NUM_THREADS", &worker_threads);
        command.env("OPENBLAS_NUM_THREADS", &worker_threads);
        command.env("VECLIB_MAXIMUM_THREADS", &worker_threads);
        command.env("TOKENIZERS_PARALLELISM", "false");

        let mut child = command.spawn().map_err(|error| {
            ApplicationError::SpeakerDiarization(format!(
                "failed to start pyannote helper with '{}': {error}",
                self.python_path
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeakerDiarization("pyannote stdout was unavailable".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeakerDiarization("pyannote stderr was unavailable".to_string())
        })?;
        let stdout_task = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let mut stdout = stdout;
            stdout.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let last_progress = Arc::new(Mutex::new(None::<DiarizationProgress>));
        let progress_emit = emit_progress.clone();
        let progress_last = last_progress.clone();
        let progress_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut diagnostics = Vec::new();
            let mut last_percentage = 0u8;
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(payload) = line.strip_prefix(PROGRESS_PREFIX) {
                    if let Ok(mut progress) = serde_json::from_str::<DiarizationProgress>(payload) {
                        progress.percentage = progress.percentage.max(last_percentage).min(100);
                        last_percentage = progress.percentage;
                        if let Ok(mut last) = progress_last.lock() {
                            *last = Some(progress.clone());
                        }
                        progress_emit(progress);
                        continue;
                    }
                }
                if diagnostics.len() < 100 {
                    diagnostics.push(line);
                }
            }
            diagnostics.join("\n")
        });
        let heartbeat_done = Arc::new(AtomicBool::new(false));
        let heartbeat_emit = emit_progress.clone();
        let heartbeat_last = last_progress.clone();
        let heartbeat_done_ref = heartbeat_done.clone();
        let heartbeat_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            while !heartbeat_done_ref.load(Ordering::Relaxed) {
                interval.tick().await;
                if heartbeat_done_ref.load(Ordering::Relaxed) {
                    break;
                }
                let progress = heartbeat_last.lock().ok().and_then(|last| last.clone());
                if let Some(progress) = progress {
                    heartbeat_emit(progress);
                }
            }
        });
        let memory_guard_tripped = Arc::new(AtomicBool::new(false));
        let memory_task = child
            .id()
            .and_then(|pid| Self::configured_max_rss_kb().map(|max_rss_kb| (pid, max_rss_kb)))
            .map(|(pid, max_rss_kb)| {
                let tripped = memory_guard_tripped.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(2));
                    loop {
                        interval.tick().await;
                        let Some(rss_kb) = Self::process_rss_kb(pid).await else {
                            break;
                        };
                        if rss_kb > max_rss_kb {
                            tripped.store(true, Ordering::Relaxed);
                            let _ = Command::new("kill")
                                .arg("-TERM")
                                .arg(pid.to_string())
                                .status()
                                .await;
                            break;
                        }
                    }
                })
            });

        let status = match timeout(Duration::from_secs(1800), child.wait()).await {
            Ok(result) => result.map_err(|error| {
                ApplicationError::SpeakerDiarization(format!(
                    "failed to wait for pyannote helper: {error}"
                ))
            })?,
            Err(_) => {
                let _ = child.kill().await;
                heartbeat_done.store(true, Ordering::Relaxed);
                heartbeat_task.abort();
                if let Some(memory_task) = memory_task {
                    memory_task.abort();
                }
                return Err(ApplicationError::SpeakerDiarization(
                    "pyannote helper timed out after 1800s".to_string(),
                ));
            }
        };
        heartbeat_done.store(true, Ordering::Relaxed);
        heartbeat_task.abort();
        if let Some(memory_task) = memory_task {
            memory_task.abort();
        }
        let stdout = stdout_task
            .await
            .map_err(|error| {
                ApplicationError::SpeakerDiarization(format!(
                    "failed to join pyannote stdout: {error}"
                ))
            })?
            .map_err(|error| {
                ApplicationError::SpeakerDiarization(format!(
                    "failed to read pyannote stdout: {error}"
                ))
            })?;
        let stderr = progress_task.await.map_err(|error| {
            ApplicationError::SpeakerDiarization(format!(
                "failed to join pyannote progress: {error}"
            ))
        })?;

        if !status.success() {
            if memory_guard_tripped.load(Ordering::Relaxed) {
                return Err(ApplicationError::SpeakerDiarization(
                    "pyannote memory guard stopped speaker detection before it could exhaust system memory; the transcript remains available".to_string(),
                ));
            }
            let stderr = stderr.trim().to_string();
            return Err(ApplicationError::SpeakerDiarization(if stderr.is_empty() {
                format!("pyannote helper exited with status {status}")
            } else {
                format!("pyannote helper failed: {stderr}")
            }));
        }

        Self::parse_turns(&stdout)
    }
}

impl PyannoteSpeakerDiarizationEngine {
    fn adaptive_batch_size(input_wav: &Path) -> u32 {
        match Self::wav_duration_seconds(input_wav) {
            Some(seconds) if seconds >= 7_200.0 => 8,
            Some(seconds) if seconds >= 3_600.0 => 16,
            Some(_) => 32,
            None => 16,
        }
    }

    fn wav_duration_seconds(input_wav: &Path) -> Option<f32> {
        let reader = hound::WavReader::open(input_wav).ok()?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate.max(1) as f32;
        let channels = spec.channels.max(1) as f32;
        Some((reader.duration() as f32 / channels) / sample_rate)
    }

    fn configured_max_rss_kb() -> Option<u64> {
        const DEFAULT_MAX_RSS_MB: u64 = 12 * 1024;
        let configured = std::env::var("SBOBINO_PYANNOTE_MAX_RSS_MB")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_MAX_RSS_MB);
        if configured == 0 {
            None
        } else {
            Some(configured.saturating_mul(1024))
        }
    }

    async fn process_rss_kb(pid: u32) -> Option<u64> {
        let output = Command::new("ps")
            .arg("-o")
            .arg("rss=")
            .arg("-p")
            .arg(pid.to_string())
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()
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
