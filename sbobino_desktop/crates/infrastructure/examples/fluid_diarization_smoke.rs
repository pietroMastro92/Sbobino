//! Native factory/adapter proof on public or synthetic WAV; no microphone or app profile.
use std::path::PathBuf;

use sbobino_domain::{settings::SpeakerDiarizationBackend, AppSettings};
use sbobino_infrastructure::{
    fluid_diarization_runtime::{fluid_bundle_health, HostPlatform},
    RuntimeTranscriptionFactory,
};

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err(
            "usage: fluid_diarization_smoke <bundle resources> <public WAV> <new isolated profile>"
                .into(),
        );
    }
    let resources = PathBuf::from(&args[0]);
    let profile = PathBuf::from(&args[2]);
    if profile.exists() {
        return Err("profile already exists; refusing to touch existing data".into());
    }
    if std::env::var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS").as_deref() != Ok("1") {
        return Err(
            "the synthetic proof requires isolated local secrets, without Keychain access".into(),
        );
    }
    let health = fluid_bundle_health(Some(&resources), &HostPlatform::current());
    if !health.ready {
        return Err(health.message);
    }
    let factory = RuntimeTranscriptionFactory::new(&profile, Some(resources))?;
    let mut settings = AppSettings::default();
    settings.transcription.speaker_diarization.enabled = true;
    settings.transcription.speaker_diarization.backend = SpeakerDiarizationBackend::Fluid;
    settings.transcription.speaker_diarization.device = "cpu".into();
    let diarizer = factory
        .build_speaker_diarizer(&settings)?
        .ok_or("no diarizer selected")?;
    let turns = diarizer
        .diarize(&PathBuf::from(&args[1]))
        .await
        .map_err(|e| e.to_string())?;
    if turns.is_empty() {
        return Err("public speech fixture produced no speaker turns".into());
    }
    println!("{}", serde_json::to_string(&serde_json::json!({
        "backend": "fluid", "health": health, "speakers": turns,
        "scope": "native bundled file diarization via real app factory; not ASR, Live, UI or multilingual quality"
    })).map_err(|e| e.to_string())?);
    Ok(())
}
