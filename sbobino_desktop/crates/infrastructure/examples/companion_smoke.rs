//! Long-running, isolated companion process for iOS simulator smoke tests.
//!
//! The process owns only the data directory passed with `--data-dir`.  Audio
//! and transcripts uploaded through the companion are therefore stored in the
//! isolated artifact database/vault, while the installed desktop binaries and
//! models are referenced read-only from `~/Library/Application Support`.
//!
//! Pairing JSON is written exactly once to stdout.  Keep stdout as a private
//! pipe when invoking this example; all diagnostics go to stderr.

use sbobino_domain::{AppSettings, ParakeetModel, SpeechModel, TranscriptionEngine};
use sbobino_infrastructure::{companion::CompanionServer, RuntimeTranscriptionFactory};
use std::{
    env,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy)]
enum Engine {
    Whisper,
    Parakeet,
}

impl Engine {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "whisper" | "whisper-cpp" => Ok(Self::Whisper),
            "parakeet" | "parakeet-cpp" => Ok(Self::Parakeet),
            other => Err(format!(
                "engine non valido: {other}; usa whisper oppure parakeet"
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
            Self::Parakeet => "parakeet",
        }
    }
}

struct Options {
    data_dir: PathBuf,
    support_dir: PathBuf,
    engine: Engine,
    lan: bool,
}

fn usage() -> &'static str {
    "Uso: companion_smoke --data-dir DIR [--engine whisper|parakeet] [--support-dir DIR] [--lan]"
}

fn parse_options() -> Result<Option<Options>, String> {
    let mut data_dir = None;
    let mut support_dir = None;
    let mut engine = Engine::Whisper;
    let mut lan = false;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--lan" => lan = true,
            "--data-dir" => {
                data_dir =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        "manca il valore di --data-dir".to_string()
                    })?));
            }
            "--support-dir" => {
                support_dir =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        "manca il valore di --support-dir".to_string()
                    })?));
            }
            "--engine" => {
                engine = Engine::parse(
                    &args
                        .next()
                        .ok_or_else(|| "manca il valore di --engine".to_string())?,
                )?;
            }
            other => return Err(format!("argomento non riconosciuto: {other}")),
        }
    }

    let data_dir = data_dir.ok_or_else(|| format!("--data-dir è obbligatorio. {}", usage()))?;
    let support_dir = support_dir
        .or_else(|| env::var_os("SBOBINO_DESKTOP_SUPPORT_DIR").map(PathBuf::from))
        .or_else(|| {
            env::var_os("HOME").map(|home| {
                PathBuf::from(home).join("Library/Application Support/com.sbobino.desktop")
            })
        })
        .ok_or_else(|| "non riesco a determinare la directory Application Support".to_string())?;

    Ok(Some(Options {
        data_dir,
        support_dir,
        engine,
        lan,
    }))
}

fn require_file(path: &Path, label: &str) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("{label} non trovato: {}", path.display()))
    }
}

fn write_isolated_settings(options: &Options) -> Result<(), String> {
    if options.data_dir.exists() {
        let target = std::fs::canonicalize(&options.data_dir).map_err(|e| e.to_string())?;
        let support = std::fs::canonicalize(&options.support_dir).map_err(|e| e.to_string())?;
        if target == support || target.starts_with(&support) {
            return Err(
                "La directory di prova deve essere separata dai dati desktop personali.".into(),
            );
        }
        if std::fs::read_dir(&target)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        {
            return Err(
                "La directory di prova deve essere nuova o vuota; nessun dato verrà sovrascritto."
                    .into(),
            );
        }
    }
    let bin_dir = options.support_dir.join("bin");
    let models_dir = options.support_dir.join("models");
    let parakeet_models_dir = options.support_dir.join("parakeet-models");
    require_file(&bin_dir.join("ffmpeg"), "FFmpeg")?;

    let mut settings = AppSettings::default();
    settings.transcription.engine = match options.engine {
        Engine::Whisper => TranscriptionEngine::WhisperCpp,
        Engine::Parakeet => TranscriptionEngine::ParakeetCpp,
    };
    settings.transcription.model = SpeechModel::LargeTurbo;
    settings.transcription.parakeet_model = ParakeetModel::Tdt06bV3Q4;
    settings.transcription.ffmpeg_path = bin_dir.join("ffmpeg").display().to_string();
    settings.transcription.whisper_cli_path = bin_dir.join("whisper-cli").display().to_string();
    settings.transcription.parakeet_cli_path = bin_dir.join("parakeet-cli").display().to_string();
    settings.transcription.models_dir = models_dir.display().to_string();
    settings.transcription.parakeet_models_dir = parakeet_models_dir.display().to_string();
    settings.sync_legacy_from_sections();

    match options.engine {
        Engine::Whisper => require_file(
            &models_dir.join(SpeechModel::LargeTurbo.ggml_filename()),
            "Whisper large-v3-turbo Q8",
        )?,
        Engine::Parakeet => require_file(
            &parakeet_models_dir.join(ParakeetModel::Tdt06bV3Q4.gguf_filename()),
            "Parakeet TDT v3 Q4",
        )?,
    }
    require_file(
        &bin_dir.join(match options.engine {
            Engine::Whisper => "whisper-cli",
            Engine::Parakeet => "parakeet-cli",
        }),
        match options.engine {
            Engine::Whisper => "Whisper CLI",
            Engine::Parakeet => "Parakeet CLI",
        },
    )?;

    std::fs::create_dir_all(&options.data_dir).map_err(|error| {
        format!(
            "non posso creare la directory isolata {}: {error}",
            options.data_dir.display()
        )
    })?;
    let settings_path = options.data_dir.join("settings.json");
    let bytes = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("serializzazione impostazioni fallita: {error}"))?;
    std::fs::write(&settings_path, bytes).map_err(|error| {
        format!(
            "non posso scrivere le impostazioni isolate {}: {error}",
            settings_path.display()
        )
    })?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let Some(options) = parse_options()? else {
        eprintln!("{}", usage());
        return Ok(());
    };
    write_isolated_settings(&options)?;

    let factory = Arc::new(RuntimeTranscriptionFactory::new(&options.data_dir, None)?);
    factory
        .build_service()
        .map_err(|error| format!("runtime {0} non pronto: {error}", options.engine.name()))?;
    let engine = options.engine.name();
    let server = CompanionServer::start(
        factory,
        Arc::new(Semaphore::new(1)),
        options.lan,
        env!("CARGO_PKG_VERSION").to_string(),
        Arc::new(|artifact| {
            eprintln!(
                "companion receipt: {}",
                serde_json::json!({
                    "artifact": artifact.id,
                    "engine": artifact.processing_engine,
                    "model": artifact.processing_model,
                    "audio_duration": artifact.audio_duration_seconds,
                    "audio_available": artifact.audio_available,
                    "text_utf8_bytes": artifact.raw_transcript.len()
                })
            );
        }),
    )
    .await?;

    let pairing = serde_json::to_string(&server.pairing)?;
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    writeln!(stdout, "{pairing}")?;
    stdout.flush()?;
    eprintln!(
        "companion attivo: engine={engine}, dati isolati={}, endpoint={}",
        options.data_dir.display(),
        server.pairing.endpoint
    );
    eprintln!("arresta il processo per revocare token, certificato e job attivi");

    // CompanionServer owns the listener and cancels all jobs when dropped.
    // Keeping the example alive makes its stdout a simple pairing pipe for the
    // iOS simulator test harness.
    std::future::pending::<()>().await;
    #[allow(unreachable_code)]
    Ok(())
}
