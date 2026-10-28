use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};

use sbobino_application::{ApplicationError, SpeakerDiarizationEngine};
use sbobino_domain::SpeakerTurn;

use crate::background_process::tokio_background_command;

#[derive(Clone, Debug)]
pub struct FluidDiarizationEngine {
    helper_path: PathBuf,
    models_dir: PathBuf,
    neural_engine: bool,
    deadline: Duration,
}

#[derive(Deserialize)]
struct Output {
    version: u8,
    backend: String,
    speakers: Vec<Turn>,
}

#[derive(Deserialize)]
struct Turn {
    speaker_id: String,
    speaker_label: Option<String>,
    start_seconds: f32,
    end_seconds: f32,
}

fn failure(message: impl Into<String>) -> ApplicationError {
    ApplicationError::SpeakerDiarization(message.into())
}

impl FluidDiarizationEngine {
    pub fn new(helper_path: PathBuf, models_dir: PathBuf, neural_engine: bool) -> Self {
        Self {
            helper_path,
            models_dir,
            neural_engine,
            deadline: Duration::from_secs(1800),
        }
    }

    fn parse(bytes: &[u8]) -> Result<Vec<SpeakerTurn>, ApplicationError> {
        let output: Output = serde_json::from_slice(bytes)
            .map_err(|e| failure(format!("FluidAudio produced invalid JSON: {e}")))?;
        if output.version != 1 || output.backend != "fluid_offline" {
            return Err(failure("Unsupported FluidAudio helper protocol"));
        }
        output
            .speakers
            .into_iter()
            .map(|turn| {
                if turn.speaker_id.trim().is_empty()
                    || !turn.start_seconds.is_finite()
                    || !turn.end_seconds.is_finite()
                    || turn.start_seconds < 0.0
                    || turn.end_seconds <= turn.start_seconds
                {
                    return Err(failure("FluidAudio returned an invalid speaker turn"));
                }
                Ok(SpeakerTurn {
                    speaker_id: turn.speaker_id,
                    speaker_label: turn.speaker_label.filter(|label| !label.trim().is_empty()),
                    start_seconds: turn.start_seconds,
                    end_seconds: turn.end_seconds,
                })
            })
            .collect()
    }
}

async fn read_limited(
    reader: impl AsyncRead + Unpin,
    limit: u64,
) -> Result<Vec<u8>, ApplicationError> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| failure(format!("Could not read FluidAudio output: {e}")))?;
    if bytes.len() as u64 > limit {
        return Err(failure("FluidAudio helper exceeded its output limit"));
    }
    Ok(bytes)
}

#[async_trait]
impl SpeakerDiarizationEngine for FluidDiarizationEngine {
    async fn diarize(&self, input_wav: &Path) -> Result<Vec<SpeakerTurn>, ApplicationError> {
        let mut child = tokio_background_command(&self.helper_path)
            .arg("offline")
            .arg("--audio-path")
            .arg(input_wav)
            .arg("--models-dir")
            .arg(&self.models_dir)
            .arg("--compute")
            .arg(if self.neural_engine { "ane" } else { "cpu" })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| failure(format!("Could not start FluidAudio helper: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| failure("Missing FluidAudio stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| failure("Missing FluidAudio stderr"))?;
        let work = async {
            tokio::try_join!(
                read_limited(stdout, 8 * 1024 * 1024),
                read_limited(stderr, 64 * 1024),
                async {
                    child
                        .wait()
                        .await
                        .map_err(|e| failure(format!("Could not wait for FluidAudio: {e}")))
                }
            )
        };
        let (stdout, stderr, status) = tokio::time::timeout(self.deadline, work)
            .await
            .map_err(|_| failure("FluidAudio helper timed out"))??;
        if !status.success() {
            return Err(failure(format!(
                "FluidAudio helper failed ({status}): {}",
                String::from_utf8_lossy(&stderr).trim()
            )));
        }
        Self::parse(&stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn protocol_preserves_overlapping_turns_and_rejects_invalid_output() {
        let valid = json!({"version":1,"backend":"fluid_offline","speakers":[
            {"speaker_id":"s1","speaker_label":"Speaker 1","start_seconds":0,"end_seconds":2},
            {"speaker_id":"s2","start_seconds":1,"end_seconds":3}],"metrics":{"elapsed_seconds":1}});
        let turns = FluidDiarizationEngine::parse(&serde_json::to_vec(&valid).unwrap()).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].start_seconds, 1.0);
        for (field, value) in [("version", json!(2)), ("backend", json!("pyannote"))] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(FluidDiarizationEngine::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        for (field, value) in [
            ("speaker_id", json!("")),
            ("start_seconds", json!(-1)),
            ("end_seconds", json!(0)),
        ] {
            let mut bad = valid.clone();
            bad["speakers"][0][field] = value;
            assert!(FluidDiarizationEngine::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
        assert!(FluidDiarizationEngine::parse(b"not JSON").is_err());
    }

    #[cfg(unix)]
    fn helper(script: &str) -> (tempfile::TempDir, FluidDiarizationEngine) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("helper");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let engine = FluidDiarizationEngine::new(path, dir.path().to_path_buf(), false);
        (dir, engine)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn invocation_and_errors_are_real_process_results() {
        let (_dir, engine) = helper(
            r#"[ "$1" = offline ] && [ "$2" = --audio-path ] && [ "$4" = --models-dir ] && [ "$6" = --compute ] && [ "$7" = cpu ] || exit 8
printf '{"version":1,"backend":"fluid_offline","speakers":[]}'"#,
        );
        assert!(engine
            .diarize(Path::new("synthetic.wav"))
            .await
            .unwrap()
            .is_empty());
        let (_dir, engine) = helper("echo 'model missing' >&2; exit 3");
        assert!(engine
            .diarize(Path::new("synthetic.wav"))
            .await
            .unwrap_err()
            .to_string()
            .contains("model missing"));
        let (_dir, engine) = helper("dd if=/dev/zero bs=1048576 count=9 2>/dev/null");
        assert!(engine
            .diarize(Path::new("synthetic.wav"))
            .await
            .unwrap_err()
            .to_string()
            .contains("output limit"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_owned_helper() {
        let (dir, mut engine) = helper("echo $$ > \"$5/pid\"; exec sleep 30");
        // Allow the shell fixture to start under parallel workspace-test load.
        engine.deadline = Duration::from_secs(2);
        assert!(engine
            .diarize(Path::new("synthetic.wav"))
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out"));
        let pid = std::fs::read_to_string(dir.path().join("pid")).unwrap();
        assert_helper_reaped(pid.trim()).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelling_diarization_kills_the_owned_helper() {
        let (dir, engine) = helper("echo $$ > \"$5/pid\"; exec sleep 30");
        let task = tokio::spawn(async move { engine.diarize(Path::new("synthetic.wav")).await });
        let pid_file = dir.path().join("pid");
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                    if pid.trim().parse::<u32>().is_ok() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("helper must start before cancellation");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_helper_reaped(pid.trim()).await;
    }

    #[cfg(unix)]
    async fn assert_helper_reaped(pid: &str) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = tokio::process::Command::new("kill")
                    .args(["-0", pid])
                    .stderr(Stdio::null())
                    .status()
                    .await
                    .unwrap();
                if !status.success() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        })
        .await
        .expect("cancelled or timed-out helper must be reaped");
    }
}
