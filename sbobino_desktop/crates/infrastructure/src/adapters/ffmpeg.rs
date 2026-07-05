use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, AudioTranscoder};

#[derive(Debug, Clone)]
pub struct FfmpegAdapter {
    binary_path: String,
}

impl FfmpegAdapter {
    pub fn new(binary_path: String) -> Self {
        Self { binary_path }
    }

    fn build_transcode_command_with_progress(
        &self,
        input: &Path,
        output: &Path,
        progress: bool,
    ) -> Command {
        let mut command = Command::new(&self.binary_path);
        command
            .kill_on_drop(true)
            .arg("-y")
            .arg("-nostdin")
            .arg("-i")
            .arg(input)
            .arg("-map")
            .arg("0:a:0")
            .arg("-vn")
            .arg("-sn")
            .arg("-dn")
            .arg("-map_metadata")
            .arg("-1")
            .arg("-ar")
            .arg("16000")
            .arg("-ac")
            .arg("1")
            .arg("-c:a")
            .arg("pcm_s16le")
            .arg("-f")
            .arg("wav");
        if progress {
            command.arg("-progress").arg("pipe:2").arg("-nostats");
        }
        command.arg(output);
        command
    }

    fn parse_progress_seconds(line: &str) -> Option<f32> {
        let (key, value) = line.split_once('=')?;
        match key.trim() {
            "out_time_ms" => value
                .trim()
                .parse::<f32>()
                .ok()
                .map(|microseconds| (microseconds / 1_000_000.0).max(0.0)),
            "out_time" => Self::parse_timestamp_seconds(value.trim()),
            _ => None,
        }
    }

    fn parse_duration_seconds(line: &str) -> Option<f32> {
        let marker = "Duration:";
        let start = line.find(marker)? + marker.len();
        let value = line[start..].split(',').next()?.trim();
        Self::parse_timestamp_seconds(value)
    }

    fn parse_timestamp_seconds(value: &str) -> Option<f32> {
        let mut parts = value.split(':');
        let hours = parts.next()?.trim().parse::<f32>().ok()?;
        let minutes = parts.next()?.trim().parse::<f32>().ok()?;
        let seconds = parts.next()?.trim().parse::<f32>().ok()?;
        Some((hours * 3600.0 + minutes * 60.0 + seconds).max(0.0))
    }

    fn is_progress_line(line: &str) -> bool {
        matches!(
            line.split_once('=').map(|(key, _)| key.trim()),
            Some(
                "frame"
                    | "fps"
                    | "stream_0_0_q"
                    | "bitrate"
                    | "total_size"
                    | "out_time_us"
                    | "out_time_ms"
                    | "out_time"
                    | "dup_frames"
                    | "drop_frames"
                    | "speed"
                    | "progress"
            )
        )
    }

    async fn run_transcode(
        &self,
        input: &Path,
        output: &Path,
        emit_progress: Option<Arc<dyn Fn(f32, Option<f32>) + Send + Sync>>,
    ) -> Result<(), ApplicationError> {
        let mut command =
            self.build_transcode_command_with_progress(input, output, emit_progress.is_some());
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().map_err(|e| {
            ApplicationError::AudioTranscoding(format!(
                "ffmpeg process failed to start ({}) : {e}",
                self.binary_path
            ))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::AudioTranscoding("ffmpeg stderr was unavailable".to_string())
        })?;
        let diagnostics = Arc::new(Mutex::new(VecDeque::<String>::new()));
        let diagnostics_ref = diagnostics.clone();
        let progress_task = tokio::spawn(async move {
            let mut total_seconds: Option<f32> = None;
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for raw_part in line.split('\r') {
                    let part = raw_part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    if total_seconds.is_none() {
                        total_seconds = Self::parse_duration_seconds(part);
                    }
                    if let Some(seconds) = Self::parse_progress_seconds(part) {
                        if let Some(emit_progress) = &emit_progress {
                            emit_progress(seconds, total_seconds);
                        }
                    }
                    if !Self::is_progress_line(part) {
                        if let Ok(mut diagnostics) = diagnostics_ref.lock() {
                            if diagnostics.len() >= 80 {
                                diagnostics.pop_front();
                            }
                            diagnostics.push_back(part.to_string());
                        }
                    }
                }
            }
        });

        let status = match timeout(Duration::from_secs(300), child.wait()).await {
            Ok(result) => result.map_err(|e| {
                ApplicationError::AudioTranscoding(format!(
                    "failed to wait for ffmpeg process ({}) : {e}",
                    self.binary_path
                ))
            })?,
            Err(_) => {
                let _ = child.kill().await;
                progress_task.abort();
                return Err(ApplicationError::AudioTranscoding(
                    "ffmpeg conversion timed out after 300s".to_string(),
                ));
            }
        };
        let _ = progress_task.await;

        if !status.success() {
            let diagnostics = diagnostics
                .lock()
                .ok()
                .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join("\n"))
                .unwrap_or_default();
            return Err(ApplicationError::AudioTranscoding(format!(
                "ffmpeg conversion failed: {}",
                if diagnostics.trim().is_empty() {
                    status.to_string()
                } else {
                    diagnostics
                }
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl AudioTranscoder for FfmpegAdapter {
    async fn to_wav_mono_16k(&self, input: &Path, output: &Path) -> Result<(), ApplicationError> {
        self.run_transcode(input, output, None).await
    }

    async fn to_wav_mono_16k_with_progress(
        &self,
        input: &Path,
        output: &Path,
        emit_progress: Arc<dyn Fn(f32, Option<f32>) + Send + Sync>,
    ) -> Result<(), ApplicationError> {
        self.run_transcode(input, output, Some(emit_progress)).await
    }
}

#[cfg(test)]
mod tests {
    use super::FfmpegAdapter;
    use std::path::Path;

    #[test]
    fn transcode_command_uses_audio_only_safe_flags() {
        let adapter = FfmpegAdapter::new("ffmpeg".to_string());
        let command = adapter.build_transcode_command_with_progress(
            Path::new("in.mp3"),
            Path::new("out.wav"),
            false,
        );
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args.windows(2).any(|pair| pair == ["-map", "0:a:0"]));
        assert!(args.contains(&"-vn".to_string()));
        assert!(args.contains(&"-sn".to_string()));
        assert!(args.contains(&"-dn".to_string()));
        assert!(args.windows(2).any(|pair| pair == ["-map_metadata", "-1"]));
        assert!(args.windows(2).any(|pair| pair == ["-c:a", "pcm_s16le"]));
        assert!(args.windows(2).any(|pair| pair == ["-f", "wav"]));
    }

    #[test]
    fn progress_transcode_command_enables_ffmpeg_machine_progress() {
        let adapter = FfmpegAdapter::new("ffmpeg".to_string());
        let command = adapter.build_transcode_command_with_progress(
            Path::new("in.m4a"),
            Path::new("out.wav"),
            true,
        );
        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args.windows(2).any(|pair| pair == ["-progress", "pipe:2"]));
        assert!(args.contains(&"-nostats".to_string()));
    }

    #[test]
    fn parses_ffmpeg_duration_and_progress_seconds() {
        assert_eq!(
            FfmpegAdapter::parse_duration_seconds(
                "  Duration: 02:10:14.00, start: 0.000000, bitrate: 128 kb/s",
            ),
            Some(7814.0),
        );
        assert_eq!(
            FfmpegAdapter::parse_progress_seconds("out_time=00:01:02.500000"),
            Some(62.5),
        );
        assert_eq!(
            FfmpegAdapter::parse_progress_seconds("out_time_ms=2500000"),
            Some(2.5),
        );
    }
}
