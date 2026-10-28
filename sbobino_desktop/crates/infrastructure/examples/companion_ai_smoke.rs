//! Isolated real-provider smoke. Pairing credentials go only to a private file.
use sbobino_domain::{AppSettings, RemoteServiceConfig, RemoteServiceKind};
use sbobino_infrastructure::{companion::CompanionServer, RuntimeTranscriptionFactory};
use std::{io::Write, path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let data_dir = PathBuf::from(args.next().ok_or("data directory required")?);
    let pairing_file = PathBuf::from(args.next().ok_or("private pairing file required")?);
    let provider = args
        .next()
        .ok_or("explicit provider foundation_apple or ollama required")?;
    let model = args.next().unwrap_or_else(|| "qwen3:8b".into());
    let flags: Vec<_> = args.collect();
    if flags
        .iter()
        .any(|flag| !["--lan", "--resume"].contains(&flag.as_str()))
    {
        return Err("Unknown flag".into());
    }
    let lan = flags.iter().any(|flag| flag == "--lan");
    let resume = flags.iter().any(|flag| flag == "--resume");
    let marker = data_dir.join("isolated-ai-smoke-v1");
    if data_dir.exists()
        && std::fs::read_dir(&data_dir)?.next().is_some()
        && !(resume && marker.is_file())
    {
        return Err(
            "Use a new empty isolated profile; existing data will not be overwritten".into(),
        );
    }
    std::fs::create_dir_all(&data_dir)?;
    let mut settings = AppSettings::default();
    let selection = match provider.as_str() {
        "foundation_apple" => {
            settings.ai.providers.foundation_apple.enabled = true;
            "foundation_apple".to_string()
        }
        "ollama" => {
            settings.ai.remote_services = vec![RemoteServiceConfig {
                id: "local-smoke".into(),
                kind: RemoteServiceKind::Ollama,
                label: "Isolated Ollama".into(),
                enabled: true,
                model: Some(model),
                base_url: Some("http://127.0.0.1:11434/v1".into()),
                ..Default::default()
            }];
            "remote:local-smoke".into()
        }
        _ => return Err("Only explicitly selected local providers are supported".into()),
    };
    if !resume {
        std::fs::write(
            data_dir.join("settings.json"),
            serde_json::to_vec(&settings)?,
        )?;
        std::fs::write(
            &marker,
            b"synthetic companion AI profile; safe to resume explicitly\n",
        )?;
    }
    let factory = Arc::new(RuntimeTranscriptionFactory::new(&data_dir, None)?);
    let server = CompanionServer::start(
        factory,
        Arc::new(Semaphore::new(1)),
        lan,
        env!("CARGO_PKG_VERSION").into(),
        Arc::new(|_| {}),
    )
    .await?;
    let status = server.ai_service().select(Some(selection)).await?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(pairing_file)?;
    file.write_all(&serde_json::to_vec(&server.pairing)?)?;
    file.sync_all()?;
    println!(
        "{}",
        serde_json::json!({"endpoint":server.pairing.endpoint,"ai":status})
    );
    std::future::pending::<()>().await;
    Ok(())
}
