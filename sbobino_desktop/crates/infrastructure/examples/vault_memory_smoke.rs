//! Opt-in synthetic vault benchmark. Never opens the user's database or Keychain.
//! Compares historical allocation layout, owned-buffer writer and real repository.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sbobino_application::ArtifactRepository;
use sbobino_domain::{ArtifactKind, ArtifactSourceOrigin, TranscriptArtifact};
use sbobino_infrastructure::repositories::sqlite_artifact_repository::SqliteArtifactRepository;
use sbobino_infrastructure::secure_storage::{encrypt_owned_to_file, SecureStorage};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 4 {
        return Err(
            "usage: vault_memory_smoke <legacy|owned|repository> <MiB> <existing-output-directory>"
                .into(),
        );
    }
    let mode = &arguments[1];
    if !matches!(mode.as_str(), "legacy" | "owned" | "repository") {
        return Err("invalid benchmark mode".into());
    }
    let mib: u64 = arguments[2].parse()?;
    if mib == 0 || mib > 2048 || (mode == "legacy" && mib > 512) {
        return Err("synthetic payload must be 1..2048 MiB (legacy capped at 512 MiB)".into());
    }
    let base = PathBuf::from(&arguments[3]);
    if !base.is_dir() {
        return Err("output directory must already exist".into());
    }
    let root = tempfile::Builder::new()
        .prefix("synthetic-vault-")
        .tempdir_in(base)?;
    // This exception is confined to this standalone synthetic-data process and
    // its newly created directory. No production app/service is launched here.
    std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
    let source = root.path().join("synthetic.bin");
    let byte_count = mib * 1024 * 1024;
    let expected_sha256 = write_synthetic_source(&source, byte_count)?;
    let storage = SecureStorage::load_or_create_with_fallback(root.path())?;
    let before_peak_rss = peak_rss_bytes();
    let started = Instant::now();
    let mut repository_saved = false;
    let encrypted_path = if mode == "repository" {
        let repo = SqliteArtifactRepository::new(root.path().join("artifacts.db"))?;
        let mut artifact = TranscriptArtifact::new(
            "synthetic-memory-job",
            "Synthetic vault memory",
            ArtifactKind::File,
            source.to_string_lossy().to_string(),
            ArtifactSourceOrigin::Imported,
            "Synthetic text, not ASR.",
            "Synthetic text, not ASR.",
            String::new(),
            String::new(),
            BTreeMap::new(),
        )?;
        artifact.set_source_external_path(source.to_string_lossy().into_owned());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(repo.save(&artifact))?;
        let loaded = runtime
            .block_on(repo.get_by_id(&artifact.id))?
            .ok_or("artifact missing")?;
        if !loaded.audio_available {
            return Err("repository audio must be available".into());
        }
        repository_saved = true;
        root.path()
            .join("vault/artifacts")
            .join(artifact.id)
            .join("source.enc")
    } else {
        let bytes = std::fs::read(&source)?;
        if format!("{:x}", Sha256::digest(&bytes)) != expected_sha256 {
            return Err("synthetic input checksum mismatch".into());
        }
        let target = root.path().join("source.enc");
        if mode == "legacy" {
            // Reproduces the unchanged historical encrypt_bytes implementation
            // and keeps its input alive, as import_audio_file previously did.
            let ciphertext = storage.encrypt_bytes("synthetic-audio-vault", &bytes)?;
            std::fs::write(&target, ciphertext)?;
            std::fs::File::open(&target)?.sync_all()?;
            std::hint::black_box(&bytes);
        } else {
            encrypt_owned_to_file(&storage, "synthetic-audio-vault", &target, bytes)?;
        }
        target
    };
    let elapsed = started.elapsed().as_secs_f64();
    let observed_peak_rss = peak_rss_bytes();
    let encoded_bytes = std::fs::metadata(encrypted_path)?.len();
    if encoded_bytes != byte_count + 28 {
        return Err("historical vault layout size mismatch".into());
    }
    println!(
        "{}",
        serde_json::json!({
            "mode": mode, "synthetic": true, "payload_bytes": byte_count,
            "source_sha256": expected_sha256, "encrypted_bytes": encoded_bytes,
            "elapsed_seconds": elapsed, "peak_rss_before_bytes": before_peak_rss,
            "peak_rss_bytes": observed_peak_rss, "repository_saved": repository_saved,
            "full_roundtrip_in_this_run": false,
            "scope": "read_hash_encrypt_sync_persist_not_ASR_or_iOS",
            "legacy_sync_matched": true
        })
    );
    // TempDir removes only this process's synthetic files, never user data.
    Ok(())
}

fn write_synthetic_source(
    path: &Path,
    byte_count: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut file = std::fs::File::create(path)?;
    let buffer: Vec<u8> = (0..131_072)
        .map(|index| (index as u8).wrapping_mul(37))
        .collect();
    let mut remaining = byte_count;
    let mut digest = Sha256::new();
    while remaining > 0 {
        let length = remaining.min(buffer.len() as u64) as usize;
        file.write_all(&buffer[..length])?;
        digest.update(&buffer[..length]);
        remaining -= length as u64;
    }
    file.sync_all()?;
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(unix)]
fn peak_rss_bytes() -> Option<u64> {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let value = u64::try_from(usage.ru_maxrss).ok()?;
    #[cfg(target_os = "macos")]
    return Some(value);
    #[cfg(not(target_os = "macos"))]
    return Some(value * 1024);
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> Option<u64> {
    None
}
