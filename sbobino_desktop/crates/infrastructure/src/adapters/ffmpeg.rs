use std::path::Path;

use async_trait::async_trait;
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

    fn build_transcode_command(&self, input: &Path, output: &Path) -> Command {
        let mut command = Command::new(&self.binary_path);
        if let Some(binary_dir) = Path::new(&self.binary_path).parent() {
            let sibling_lib = binary_dir.join("../lib");
            let mut runtime_paths = vec![binary_dir.to_path_buf()];
            if sibling_lib.is_dir() {
                runtime_paths.push(sibling_lib.clone());
            }
            if let Some(existing) = std::env::var_os("PATH") {
                runtime_paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(path) = std::env::join_paths(runtime_paths) {
                command.env("PATH", path);
            }
            #[cfg(target_os = "macos")]
            if sibling_lib.is_dir() {
                command
                    .env("DYLD_LIBRARY_PATH", &sibling_lib)
                    .env("DYLD_FALLBACK_LIBRARY_PATH", &sibling_lib);
            }
        }
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
            .arg("wav")
            .arg(output);
        command
    }
}

#[async_trait]
impl AudioTranscoder for FfmpegAdapter {
    async fn to_wav_mono_16k(&self, input: &Path, output: &Path) -> Result<(), ApplicationError> {
        let mut command = self.build_transcode_command(input, output);

        let output = timeout(Duration::from_secs(300), command.output())
            .await
            .map_err(|_| {
                ApplicationError::AudioTranscoding(
                    "ffmpeg conversion timed out after 300s".to_string(),
                )
            })?
            .map_err(|e| {
                ApplicationError::AudioTranscoding(format!(
                    "ffmpeg process failed to start ({}) : {e}",
                    self.binary_path
                ))
            })?;

        if !output.status.success() {
            return Err(ApplicationError::AudioTranscoding(format!(
                "ffmpeg conversion failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::FfmpegAdapter;
    use std::path::Path;

    #[test]
    fn transcode_command_uses_audio_only_safe_flags() {
        let adapter = FfmpegAdapter::new("ffmpeg".to_string());
        let command = adapter.build_transcode_command(Path::new("in.mp3"), Path::new("out.wav"));
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
    fn transcode_command_prepends_runtime_binary_and_library_directories() {
        let adapter = FfmpegAdapter::new("runtime/bin/ffmpeg.exe".to_string());
        let command = adapter.build_transcode_command(Path::new("in.mp3"), Path::new("out.wav"));
        let path = command
            .as_std()
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
            .expect("PATH should be configured for a packaged FFmpeg binary")
            .to_string_lossy();
        assert!(path.contains("runtime/bin"));
    }
}
