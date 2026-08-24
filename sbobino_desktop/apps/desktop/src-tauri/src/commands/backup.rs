use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::{Emitter, State};
use uuid::Uuid;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use sbobino_application::{ApplicationError, ArtifactQuery};
use sbobino_domain::{AppSettings, ArtifactChatMessage, PersonalizationEntry, TranscriptArtifact};
use sbobino_infrastructure::{
    repositories::sqlite_artifact_repository::SqliteArtifactRepository,
    secure_storage::{decrypt_file_with_password, encrypt_file_with_password},
};

use crate::{error::CommandError, state::AppState};

const BACKUP_FORMAT_VERSION: u32 = 3;
const BACKUP_BATCH_SIZE: usize = 500;

#[derive(Debug, Deserialize)]
pub struct ExportAppBackupPayload {
    pub destination_path: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct ExportAppBackupResponse {
    pub path: String,
    pub artifact_count: usize,
    pub deleted_artifact_count: usize,
    pub audio_file_count: usize,
    pub personalization_entry_count: usize,
    pub exported_at: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportAppBackupPayload {
    pub backup_path: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct ImportAppBackupResponse {
    pub artifact_count: usize,
    pub deleted_artifact_count: usize,
    pub personalization_entry_count: usize,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PortableBackupManifest {
    format_version: u32,
    app_version: String,
    created_at: String,
    artifact_count: usize,
    deleted_artifact_count: usize,
    audio_file_count: usize,
    #[serde(default)]
    personalization_entry_count: usize,
    includes_settings_secrets: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PortableBackupRecord {
    artifact: TranscriptArtifact,
    is_deleted: bool,
    deleted_at: Option<String>,
    source_external_path: Option<String>,
    whisper_options_json: Option<String>,
    diarization_settings_json: Option<String>,
    ai_provider_snapshot_json: Option<String>,
    source_fingerprint_json: Option<String>,
    audio_entry_name: Option<String>,
    #[serde(default)]
    chat_messages: Vec<ArtifactChatMessage>,
    #[serde(default)]
    chat_summary: Option<String>,
}

impl PortableBackupRecord {
    fn from_artifact(
        artifact: &TranscriptArtifact,
        is_deleted: bool,
        deleted_at: Option<String>,
        audio_entry_name: Option<String>,
        chat_messages: Vec<ArtifactChatMessage>,
        chat_summary: Option<String>,
    ) -> Self {
        Self {
            artifact: artifact.clone(),
            is_deleted,
            deleted_at,
            source_external_path: artifact.source_external_path.clone(),
            whisper_options_json: artifact.whisper_options_json.clone(),
            diarization_settings_json: artifact.diarization_settings_json.clone(),
            ai_provider_snapshot_json: artifact.ai_provider_snapshot_json.clone(),
            source_fingerprint_json: artifact.source_fingerprint_json.clone(),
            audio_entry_name,
            chat_messages,
            chat_summary,
        }
    }

    fn into_artifact(self) -> TranscriptArtifact {
        let mut artifact = self.artifact;
        artifact.source_external_path = self.source_external_path;
        artifact.whisper_options_json = self.whisper_options_json;
        artifact.diarization_settings_json = self.diarization_settings_json;
        artifact.ai_provider_snapshot_json = self.ai_provider_snapshot_json;
        artifact.source_fingerprint_json = self.source_fingerprint_json;
        artifact
    }
}

#[derive(Debug)]
struct PreparedImport {
    settings: AppSettings,
    records: Vec<PortableBackupRecord>,
    personalization_entries: Vec<PersonalizationEntry>,
    temp_root: PathBuf,
    staging_dir: PathBuf,
}

#[derive(Debug)]
struct SwappedStorage {
    rollback_dir: PathBuf,
}

#[tauri::command]
pub async fn export_app_backup(
    state: State<'_, AppState>,
    payload: ExportAppBackupPayload,
) -> Result<ExportAppBackupResponse, CommandError> {
    ensure_backup_idle(&state).await?;
    validate_backup_password(&payload.password)?;

    let destination_path = normalized_path(&payload.destination_path, "backup destination")?;
    let settings = state
        .settings_service
        .get()
        .await
        .map_err(CommandError::from)?;
    let active_artifacts = collect_all_artifacts(&state, false).await?;
    let deleted_artifacts = collect_all_artifacts(&state, true).await?;
    let personalization_entries = state
        .artifact_service
        .list_personalization_entries()
        .await
        .map_err(CommandError::from)?;

    let mut records = Vec::with_capacity(active_artifacts.len() + deleted_artifacts.len());
    let mut audio_entries: Vec<(String, Vec<u8>)> = Vec::new();

    for artifact in &active_artifacts {
        let audio_entry_name = if artifact.audio_available {
            let bytes = state
                .artifact_service
                .read_audio_bytes(&artifact.id)
                .await
                .map_err(CommandError::from)?
                .ok_or_else(|| {
                    CommandError::from(ApplicationError::Persistence(format!(
                        "artifact {} reports audio availability but no vault audio could be read",
                        artifact.id
                    )))
                })?;
            let entry_name = backup_audio_entry_name(artifact);
            audio_entries.push((entry_name.clone(), bytes));
            Some(entry_name)
        } else {
            None
        };

        let chat_messages = state
            .artifact_service
            .list_chat_messages(&artifact.id)
            .await
            .map_err(CommandError::from)?;
        let chat_summary = state
            .artifact_service
            .load_chat_summary(&artifact.id)
            .await
            .map_err(CommandError::from)?;
        records.push(PortableBackupRecord::from_artifact(
            artifact,
            false,
            None,
            audio_entry_name,
            chat_messages,
            chat_summary,
        ));
    }

    for artifact in &deleted_artifacts {
        let audio_entry_name = if artifact.audio_available {
            let bytes = state
                .artifact_service
                .read_audio_bytes(&artifact.id)
                .await
                .map_err(CommandError::from)?
                .ok_or_else(|| {
                    CommandError::from(ApplicationError::Persistence(format!(
                        "deleted artifact {} reports audio availability but no vault audio could be read",
                        artifact.id
                    )))
                })?;
            let entry_name = backup_audio_entry_name(artifact);
            audio_entries.push((entry_name.clone(), bytes));
            Some(entry_name)
        } else {
            None
        };

        let chat_messages = state
            .artifact_service
            .list_chat_messages(&artifact.id)
            .await
            .map_err(CommandError::from)?;
        let chat_summary = state
            .artifact_service
            .load_chat_summary(&artifact.id)
            .await
            .map_err(CommandError::from)?;
        records.push(PortableBackupRecord::from_artifact(
            artifact,
            true,
            Some(artifact.updated_at.to_rfc3339()),
            audio_entry_name,
            chat_messages,
            chat_summary,
        ));
    }

    let exported_at = Utc::now().to_rfc3339();
    let manifest = PortableBackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: exported_at.clone(),
        artifact_count: records.iter().filter(|record| !record.is_deleted).count(),
        deleted_artifact_count: records.iter().filter(|record| record.is_deleted).count(),
        audio_file_count: audio_entries.len(),
        personalization_entry_count: personalization_entries.len(),
        includes_settings_secrets: true,
    };
    let artifact_count = manifest.artifact_count;
    let deleted_artifact_count = manifest.deleted_artifact_count;
    let audio_file_count = manifest.audio_file_count;
    let personalization_entry_count = manifest.personalization_entry_count;

    let destination_for_task = destination_path.clone();
    let password = payload.password;
    let manifest_for_task = manifest.clone();
    tokio::task::spawn_blocking(move || {
        let temp_root = temp_work_dir("backup-export")?;
        let zip_path = temp_root.join("portable-backup.zip");
        let result = (|| -> Result<(), ApplicationError> {
            write_backup_zip(
                &zip_path,
                &manifest_for_task,
                &settings,
                &records,
                &personalization_entries,
                &audio_entries,
            )?;
            encrypt_file_with_password(&zip_path, &destination_for_task, &password)?;
            Ok(())
        })();
        let _ = remove_path_if_exists(&temp_root);
        result
    })
    .await
    .map_err(|e| {
        CommandError::from(ApplicationError::Persistence(format!(
            "backup export join error: {e}"
        )))
    })?
    .map_err(CommandError::from)?;

    Ok(ExportAppBackupResponse {
        path: destination_path.display().to_string(),
        artifact_count,
        deleted_artifact_count,
        audio_file_count,
        personalization_entry_count,
        exported_at,
    })
}

#[tauri::command]
pub async fn import_app_backup(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: ImportAppBackupPayload,
) -> Result<ImportAppBackupResponse, CommandError> {
    ensure_backup_idle(&state).await?;
    validate_backup_password(&payload.password)?;

    let previous_settings = state
        .settings_service
        .get()
        .await
        .map_err(CommandError::from)?;
    let backup_path = normalized_path(&payload.backup_path, "backup path")?;
    let password = payload.password;
    let data_dir = state.runtime_factory.data_dir().to_path_buf();
    let secure_storage_root = data_dir.clone();

    let prepared = tokio::task::spawn_blocking(move || {
        prepare_import(&backup_path, &password, &secure_storage_root)
    })
    .await
    .map_err(|e| {
        CommandError::from(ApplicationError::Persistence(format!(
            "backup import preparation join error: {e}"
        )))
    })?
    .map_err(CommandError::from)?;

    let staging_dir = prepared.staging_dir.clone();
    let swapped =
        tokio::task::spawn_blocking(move || swap_storage_with_rollback(&data_dir, &staging_dir))
            .await
            .map_err(|e| {
                CommandError::from(ApplicationError::Persistence(format!(
                    "backup storage swap join error: {e}"
                )))
            })?
            .map_err(CommandError::from)?;

    let imported_settings = prepared.settings.clone();
    let imported_at = Utc::now().to_rfc3339();
    let artifact_count = prepared
        .records
        .iter()
        .filter(|record| !record.is_deleted)
        .count();
    let deleted_artifact_count = prepared
        .records
        .iter()
        .filter(|record| record.is_deleted)
        .count();
    let personalization_entry_count = prepared.personalization_entries.len();

    match state.settings_service.update(imported_settings).await {
        Ok(updated) => {
            let _ = app.emit("settings://updated", updated.redacted_clone());
            let cleanup_root = prepared.temp_root.clone();
            let rollback_dir = swapped.rollback_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = remove_path_if_exists(&cleanup_root);
                let _ = remove_path_if_exists(&rollback_dir);
            })
            .await;

            Ok(ImportAppBackupResponse {
                artifact_count,
                deleted_artifact_count,
                personalization_entry_count,
                imported_at,
            })
        }
        Err(settings_error) => {
            let data_dir = state.runtime_factory.data_dir().to_path_buf();
            let rollback_dir = swapped.rollback_dir.clone();
            let restore_result = tokio::task::spawn_blocking(move || {
                restore_storage_from_rollback(&data_dir, &rollback_dir)
            })
            .await
            .map_err(|e| {
                CommandError::from(ApplicationError::Persistence(format!(
                    "backup rollback join error: {e}"
                )))
            })?;
            if let Err(restore_error) = restore_result {
                return Err(CommandError::from(ApplicationError::Persistence(format!(
                    "backup import failed while saving settings ({settings_error}) and storage rollback also failed ({restore_error})"
                ))));
            }

            let _ = state.settings_service.update(previous_settings).await;
            let cleanup_root = prepared.temp_root.clone();
            let rollback_dir = swapped.rollback_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = remove_path_if_exists(&cleanup_root);
                let _ = remove_path_if_exists(&rollback_dir);
            })
            .await;

            Err(CommandError::from(settings_error))
        }
    }
}

async fn ensure_backup_idle(state: &AppState) -> Result<(), CommandError> {
    if !state.transcription_tasks.lock().await.is_empty() {
        return Err(CommandError::from(ApplicationError::Validation(
            "finish or cancel active transcriptions before exporting or importing a backup"
                .to_string(),
        )));
    }
    let realtime_engine = state.realtime.engine.lock().await.clone();
    if realtime_engine.is_running().await {
        return Err(CommandError::from(ApplicationError::Validation(
            "stop the realtime recorder before exporting or importing a backup".to_string(),
        )));
    }
    Ok(())
}

fn validate_backup_password(password: &str) -> Result<(), CommandError> {
    if password.trim().len() < 8 {
        return Err(CommandError::from(ApplicationError::Validation(
            "backup password must be at least 8 characters long".to_string(),
        )));
    }
    Ok(())
}

fn normalized_path(value: &str, label: &str) -> Result<PathBuf, CommandError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CommandError::from(ApplicationError::Validation(format!(
            "{label} cannot be empty"
        ))));
    }
    Ok(PathBuf::from(trimmed))
}

async fn collect_all_artifacts(
    state: &AppState,
    deleted: bool,
) -> Result<Vec<TranscriptArtifact>, CommandError> {
    let mut offset = 0;
    let mut collected = Vec::new();

    loop {
        let query = ArtifactQuery {
            kind: None,
            query: None,
            limit: Some(BACKUP_BATCH_SIZE),
            offset: Some(offset),
        };
        let batch = if deleted {
            state
                .artifact_service
                .list_deleted(query)
                .await
                .map_err(CommandError::from)?
        } else {
            state
                .artifact_service
                .list(query)
                .await
                .map_err(CommandError::from)?
        };
        let batch_len = batch.len();
        if batch_len == 0 {
            break;
        }
        collected.extend(batch);
        offset += batch_len;
        if batch_len < BACKUP_BATCH_SIZE {
            break;
        }
    }

    Ok(collected)
}

fn backup_audio_entry_name(artifact: &TranscriptArtifact) -> String {
    let extension = artifact
        .source_external_path
        .as_deref()
        .and_then(|value| Path::new(value).extension())
        .and_then(|value| value.to_str())
        .or_else(|| {
            Path::new(&artifact.source_label)
                .extension()
                .and_then(|value| value.to_str())
        })
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| "bin".to_string());
    format!("audio/{}.{}", artifact.id, extension)
}

fn write_backup_zip(
    zip_path: &Path,
    manifest: &PortableBackupManifest,
    settings: &AppSettings,
    records: &[PortableBackupRecord],
    personalization_entries: &[PersonalizationEntry],
    audio_entries: &[(String, Vec<u8>)],
) -> Result<(), ApplicationError> {
    let file = File::create(zip_path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to create backup archive {}: {e}",
            zip_path.display()
        ))
    })?;
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o600);

    write_zip_json(&mut archive, "manifest.json", manifest, options)?;
    write_zip_json(&mut archive, "settings.json", settings, options)?;
    write_zip_json(&mut archive, "artifacts.json", records, options)?;
    write_zip_json(
        &mut archive,
        "personalization.json",
        personalization_entries,
        options,
    )?;
    for (entry_name, bytes) in audio_entries {
        archive.start_file(entry_name, options).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to add backup audio entry {entry_name}: {e}"
            ))
        })?;
        archive.write_all(bytes).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to write backup audio entry {entry_name}: {e}"
            ))
        })?;
    }

    archive.finish().map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to finalize backup archive {}: {e}",
            zip_path.display()
        ))
    })?;
    Ok(())
}

fn write_zip_json<T: Serialize + ?Sized>(
    archive: &mut ZipWriter<File>,
    entry_name: &str,
    value: &T,
    options: SimpleFileOptions,
) -> Result<(), ApplicationError> {
    archive.start_file(entry_name, options).map_err(|e| {
        ApplicationError::Persistence(format!("failed to create backup entry {entry_name}: {e}"))
    })?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to serialize backup entry {entry_name}: {e}"
        ))
    })?;
    archive.write_all(&bytes).map_err(|e| {
        ApplicationError::Persistence(format!("failed to write backup entry {entry_name}: {e}"))
    })
}

fn prepare_import(
    backup_path: &Path,
    password: &str,
    secure_storage_root: &Path,
) -> Result<PreparedImport, ApplicationError> {
    let temp_root = temp_work_dir("backup-import")?;
    let decrypted_zip = temp_root.join("portable-backup.zip");
    decrypt_file_with_password(backup_path, &decrypted_zip, password)?;

    let file = File::open(&decrypted_zip).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to open decrypted backup {}: {e}",
            decrypted_zip.display()
        ))
    })?;
    let mut archive = ZipArchive::new(file).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to open backup archive {}: {e}",
            decrypted_zip.display()
        ))
    })?;

    let manifest: PortableBackupManifest = read_zip_json(&mut archive, "manifest.json")?;
    if !matches!(manifest.format_version, 1 | 2 | BACKUP_FORMAT_VERSION) {
        return Err(ApplicationError::Validation(format!(
            "unsupported backup format version {}",
            manifest.format_version
        )));
    }

    let settings: AppSettings = read_zip_json(&mut archive, "settings.json")?;
    let records: Vec<PortableBackupRecord> = read_zip_json(&mut archive, "artifacts.json")?;
    let personalization_entries: Vec<PersonalizationEntry> = if manifest.format_version >= 3 {
        read_zip_json(&mut archive, "personalization.json")?
    } else {
        Vec::new()
    };
    let staging_dir = temp_root.join("staging");
    fs::create_dir_all(&staging_dir).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to create backup staging directory {}: {e}",
            staging_dir.display()
        ))
    })?;
    copy_local_master_key_if_present(secure_storage_root, &staging_dir)?;
    let repo = SqliteArtifactRepository::new(staging_dir.join("artifacts.db"))?;

    for record in &records {
        let audio_bytes = if let Some(entry_name) = record.audio_entry_name.as_deref() {
            let mut entry = archive.by_name(entry_name).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to read backup audio entry {entry_name}: {e}"
                ))
            })?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to load backup audio entry {entry_name}: {e}"
                ))
            })?;
            Some(bytes)
        } else {
            None
        };

        let mut artifact = record.clone().into_artifact();
        if record.is_deleted && record.deleted_at.is_none() {
            artifact.touch();
        }

        repo.import_backup_artifact(
            &artifact,
            audio_bytes.as_deref(),
            record.audio_entry_name.as_deref(),
            record.is_deleted,
            record.deleted_at.as_deref(),
        )?;
        for message in &record.chat_messages {
            repo.import_backup_chat_message(message)?;
        }
        if let Some(summary) = record.chat_summary.as_deref() {
            repo.import_backup_chat_summary(&artifact.id, summary)?;
        }
    }
    for entry in &personalization_entries {
        repo.import_backup_personalization_entry(entry)?;
    }

    Ok(PreparedImport {
        settings,
        records,
        personalization_entries,
        temp_root,
        staging_dir,
    })
}

fn copy_local_master_key_if_present(
    source_root: &Path,
    destination_root: &Path,
) -> Result<(), ApplicationError> {
    let relative_key_path = Path::new(".sbobino-secure-storage").join("master_key_v1");
    let source = source_root.join(&relative_key_path);
    if !source.exists() {
        return Ok(());
    }
    let destination = destination_root.join(relative_key_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            ApplicationError::Persistence(format!(
                "failed to prepare backup staging secure storage {}: {error}",
                parent.display()
            ))
        })?;
    }
    fs::copy(&source, &destination).map_err(|error| {
        ApplicationError::Persistence(format!(
            "failed to copy the local encryption key into backup staging: {error}"
        ))
    })?;
    Ok(())
}

fn read_zip_json<T: DeserializeOwned>(
    archive: &mut ZipArchive<File>,
    entry_name: &str,
) -> Result<T, ApplicationError> {
    let mut entry = archive.by_name(entry_name).map_err(|e| {
        ApplicationError::Persistence(format!("failed to open backup entry {entry_name}: {e}"))
    })?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|e| {
        ApplicationError::Persistence(format!("failed to read backup entry {entry_name}: {e}"))
    })?;
    serde_json::from_slice(&bytes).map_err(|e| {
        ApplicationError::Persistence(format!("failed to parse backup entry {entry_name}: {e}"))
    })
}

fn swap_storage_with_rollback(
    data_dir: &Path,
    staging_dir: &Path,
) -> Result<SwappedStorage, ApplicationError> {
    fs::create_dir_all(data_dir).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to create app data directory {}: {e}",
            data_dir.display()
        ))
    })?;

    let current_db = data_dir.join("artifacts.db");
    let current_vault = data_dir.join("vault");
    let staging_db = staging_dir.join("artifacts.db");
    let staging_vault = staging_dir.join("vault");
    if !staging_db.exists() {
        return Err(ApplicationError::Persistence(format!(
            "backup staging database is missing at {}",
            staging_db.display()
        )));
    }

    let rollback_dir = temp_work_dir("backup-rollback")?;
    let rollback_vault = rollback_dir.join("vault");

    let result = (|| -> Result<(), ApplicationError> {
        for current_path in sqlite_related_paths(&current_db) {
            if current_path.exists() {
                let file_name = current_path
                    .file_name()
                    .ok_or_else(|| {
                        ApplicationError::Persistence(
                            "sqlite path is missing a file name".to_string(),
                        )
                    })?
                    .to_os_string();
                fs::rename(&current_path, rollback_dir.join(file_name)).map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to move existing sqlite file {} into rollback storage: {e}",
                        current_path.display()
                    ))
                })?;
            }
        }
        if current_vault.exists() {
            fs::rename(&current_vault, &rollback_vault).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to move existing audio vault {} into rollback storage: {e}",
                    current_vault.display()
                ))
            })?;
        }

        for staged_path in sqlite_related_paths(&staging_db) {
            if staged_path.exists() {
                let file_name = staged_path
                    .file_name()
                    .ok_or_else(|| {
                        ApplicationError::Persistence(
                            "sqlite staging path is missing a file name".to_string(),
                        )
                    })?
                    .to_os_string();
                fs::rename(&staged_path, data_dir.join(file_name)).map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to move staged sqlite file {} into app storage: {e}",
                        staged_path.display()
                    ))
                })?;
            }
        }
        if staging_vault.exists() {
            fs::rename(&staging_vault, &current_vault).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to move staged audio vault {} into app storage: {e}",
                    staging_vault.display()
                ))
            })?;
        } else {
            fs::create_dir_all(&current_vault).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to create app audio vault {}: {e}",
                    current_vault.display()
                ))
            })?;
        }
        Ok(())
    })();

    if let Err(error) = result {
        let _ = restore_storage_from_rollback(data_dir, &rollback_dir);
        return Err(error);
    }

    Ok(SwappedStorage { rollback_dir })
}

fn restore_storage_from_rollback(
    data_dir: &Path,
    rollback_dir: &Path,
) -> Result<(), ApplicationError> {
    let current_db = data_dir.join("artifacts.db");
    let current_vault = data_dir.join("vault");

    for current_path in sqlite_related_paths(&current_db) {
        let _ = remove_path_if_exists(&current_path);
    }
    let _ = remove_path_if_exists(&current_vault);

    for rollback_path in sqlite_related_paths(&rollback_dir.join("artifacts.db")) {
        if rollback_path.exists() {
            let file_name = rollback_path
                .file_name()
                .ok_or_else(|| {
                    ApplicationError::Persistence(
                        "rollback sqlite path is missing a file name".to_string(),
                    )
                })?
                .to_os_string();
            fs::rename(&rollback_path, data_dir.join(file_name)).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to restore sqlite file {} from rollback storage: {e}",
                    rollback_path.display()
                ))
            })?;
        }
    }

    let rollback_vault = rollback_dir.join("vault");
    if rollback_vault.exists() {
        fs::rename(&rollback_vault, &current_vault).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to restore audio vault {} from rollback storage: {e}",
                rollback_vault.display()
            ))
        })?;
    }

    Ok(())
}

fn sqlite_related_paths(base_db_path: &Path) -> [PathBuf; 3] {
    [
        base_db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", base_db_path.display())),
        PathBuf::from(format!("{}-shm", base_db_path.display())),
    ]
}

fn temp_work_dir(prefix: &str) -> Result<PathBuf, ApplicationError> {
    let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).map_err(|e| {
        ApplicationError::Persistence(format!(
            "failed to create temporary backup directory {}: {e}",
            path.display()
        ))
    })?;
    Ok(path)
}

fn remove_path_if_exists(path: &Path) -> Result<(), ApplicationError> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::metadata(path).map_err(|e| {
        ApplicationError::Persistence(format!("failed to inspect path {}: {e}", path.display()))
    })?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to remove directory {}: {e}",
                path.display()
            ))
        })
    } else {
        fs::remove_file(path).map_err(|e| {
            ApplicationError::Persistence(format!("failed to remove file {}: {e}", path.display()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        backup_audio_entry_name, prepare_import, restore_storage_from_rollback,
        swap_storage_with_rollback, write_backup_zip, PortableBackupManifest, PortableBackupRecord,
        BACKUP_FORMAT_VERSION,
    };
    use chrono::Utc;
    use tempfile::tempdir;

    use sbobino_application::ArtifactRepository;
    use sbobino_domain::{
        AiProvider, AppSettings, ArtifactAudioBackfillStatus, ArtifactChatMessage, ArtifactKind,
        ArtifactSourceOrigin, PersonalizationEntry, PersonalizationEntryKind, RemoteServiceConfig,
        RemoteServiceKind, TranscriptArtifact,
    };
    use sbobino_infrastructure::{
        repositories::sqlite_artifact_repository::SqliteArtifactRepository,
        secure_storage::encrypt_file_with_password,
    };

    fn enable_local_secure_storage_for_tests() {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
    }

    fn sample_settings() -> AppSettings {
        let mut settings = AppSettings::default();
        settings.ai.active_provider = AiProvider::Gemini;
        settings.ai.providers.gemini.api_key = Some("gemini-secret".to_string());
        settings.ai.providers.gemini.has_api_key = true;
        settings.ai.remote_services.push(RemoteServiceConfig {
            id: "google-primary".to_string(),
            kind: RemoteServiceKind::Google,
            label: "Google".to_string(),
            enabled: true,
            api_key: Some("remote-secret".to_string()),
            has_api_key: true,
            model: Some("gemini-2.5-flash-lite".to_string()),
            base_url: Some("https://generativelanguage.googleapis.com".to_string()),
        });
        settings.ai.active_remote_service_id = Some("google-primary".to_string());
        settings.sync_legacy_from_sections();
        settings
    }

    fn sample_artifact(id_hint: &str, source_label: &str, deleted: bool) -> TranscriptArtifact {
        let mut artifact = TranscriptArtifact::new(
            format!("job-{id_hint}"),
            format!("Artifact {id_hint}"),
            ArtifactKind::File,
            source_label.to_string(),
            ArtifactSourceOrigin::Imported,
            format!("raw transcript {id_hint}"),
            format!("optimized transcript {id_hint}"),
            format!("summary {id_hint}"),
            format!("faq {id_hint}"),
            std::collections::BTreeMap::new(),
        )
        .expect("artifact");
        artifact.audio_available = true;
        artifact.audio_backfill_status = ArtifactAudioBackfillStatus::Imported;
        artifact.processing_engine = Some("whisper_cpp".to_string());
        artifact.processing_model = Some("ggml-base.bin".to_string());
        artifact.processing_language = Some("it".to_string());
        artifact.audio_duration_seconds = Some(12.5);
        artifact.audio_byte_size = Some(2048);
        artifact.parent_artifact_id = Some("parent-artifact".to_string());
        artifact.metadata.insert(
            "timeline_v2".to_string(),
            "{\"version\":2,\"segments\":[]}".to_string(),
        );
        artifact.metadata.insert(
            "emotion_analysis_v1".to_string(),
            "{\"overview\":{\"primary_emotions\":[\"calm\"]}}".to_string(),
        );
        artifact.metadata.insert(
            "emotion_analysis_generated_at".to_string(),
            Utc::now().to_rfc3339(),
        );
        artifact.whisper_options_json = Some("{\"threads\":4}".to_string());
        artifact.diarization_settings_json = Some("{\"enabled\":true}".to_string());
        artifact.ai_provider_snapshot_json = Some("{\"provider\":\"gemini\"}".to_string());
        artifact.source_fingerprint_json = Some("{\"sha256\":\"abc123\"}".to_string());
        if deleted {
            artifact.source_origin = ArtifactSourceOrigin::Trimmed;
        }
        artifact
    }

    #[tokio::test]
    async fn portable_backup_prepare_import_roundtrip_restores_artifacts_audio_and_settings() {
        enable_local_secure_storage_for_tests();
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("backup.zip");
        let backup_path = temp.path().join("backup.sbobino-backup");

        let settings = sample_settings();
        let active = sample_artifact("active", "meeting-audio.wav", false);
        let deleted = sample_artifact("deleted", "trimmed-audio.m4a", true);
        let active_audio = b"active-audio-bytes".to_vec();
        let deleted_audio = b"deleted-audio-bytes".to_vec();
        let active_chat = vec![ArtifactChatMessage::new(
            active.id.clone(),
            "user",
            "What was decided?",
            "typed",
            "complete",
        )];
        let records = vec![
            PortableBackupRecord::from_artifact(
                &active,
                false,
                None,
                Some(backup_audio_entry_name(&active)),
                active_chat,
                Some("user: What was decided?".to_string()),
            ),
            PortableBackupRecord::from_artifact(
                &deleted,
                true,
                Some(deleted.updated_at.to_rfc3339()),
                Some(backup_audio_entry_name(&deleted)),
                Vec::new(),
                None,
            ),
        ];
        let manifest = PortableBackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            app_version: "test".to_string(),
            created_at: Utc::now().to_rfc3339(),
            artifact_count: 1,
            deleted_artifact_count: 1,
            audio_file_count: 2,
            personalization_entry_count: 0,
            includes_settings_secrets: true,
        };
        let audio_entries = vec![
            (backup_audio_entry_name(&active), active_audio.clone()),
            (backup_audio_entry_name(&deleted), deleted_audio.clone()),
        ];
        let personalization_entries = vec![PersonalizationEntry::new(
            PersonalizationEntryKind::Correction,
            "Sbobinoo",
            Some("Sbobino".to_string()),
            Some("en".to_string()),
        )];
        let manifest = PortableBackupManifest {
            personalization_entry_count: personalization_entries.len(),
            ..manifest
        };

        write_backup_zip(
            &zip_path,
            &manifest,
            &settings,
            &records,
            &personalization_entries,
            &audio_entries,
        )
        .expect("write backup zip");
        encrypt_file_with_password(&zip_path, &backup_path, "test-password")
            .expect("encrypt backup");

        let live_storage_root = temp.path().join("live-storage");
        let live_repo = SqliteArtifactRepository::new(live_storage_root.join("artifacts.db"))
            .expect("live repo should establish the active encryption key");
        let prepared = prepare_import(&backup_path, "test-password", &live_storage_root)
            .expect("prepare import");
        assert_eq!(
            prepared.settings.ai.providers.gemini.api_key.as_deref(),
            Some("gemini-secret")
        );
        assert_eq!(prepared.records.len(), 2);
        assert_eq!(prepared.personalization_entries, personalization_entries);

        let repo = SqliteArtifactRepository::new(prepared.staging_dir.join("artifacts.db"))
            .expect("staging repo");
        let loaded_active = repo
            .get_by_id(&active.id)
            .await
            .expect("query active")
            .expect("active artifact");
        assert_eq!(loaded_active.title, active.title);
        assert_eq!(
            loaded_active.whisper_options_json,
            active.whisper_options_json
        );
        assert_eq!(
            loaded_active.diarization_settings_json,
            active.diarization_settings_json
        );
        assert_eq!(
            loaded_active.ai_provider_snapshot_json,
            active.ai_provider_snapshot_json
        );
        assert_eq!(
            loaded_active.source_fingerprint_json,
            active.source_fingerprint_json
        );
        assert_eq!(
            repo.read_audio_bytes(&active.id)
                .await
                .expect("read active audio"),
            Some(active_audio)
        );
        let restored_chat = repo
            .list_chat_messages(&active.id)
            .await
            .expect("list restored chat");
        assert_eq!(restored_chat.len(), 1);
        assert_eq!(restored_chat[0].text, "What was decided?");
        assert_eq!(
            repo.load_chat_summary(&active.id)
                .await
                .expect("load restored summary")
                .as_deref(),
            Some("user: What was decided?")
        );

        let deleted_entries = repo
            .list_deleted(None, None, 10, 0)
            .await
            .expect("list deleted");
        assert_eq!(deleted_entries.len(), 1);
        assert_eq!(deleted_entries[0].id, deleted.id);
        assert_eq!(
            repo.read_audio_bytes(&deleted.id)
                .await
                .expect("read deleted audio"),
            Some(deleted_audio)
        );
        assert_eq!(
            repo.list_personalization_entries()
                .await
                .expect("list restored personalization"),
            personalization_entries
        );

        let swapped = swap_storage_with_rollback(&live_storage_root, &prepared.staging_dir)
            .expect("swap prepared storage into live root");
        let live_loaded = live_repo
            .get_by_id(&active.id)
            .await
            .expect("live repository should decrypt imported artifact")
            .expect("imported artifact should exist");
        assert_eq!(live_loaded.raw_transcript, active.raw_transcript);
        assert_eq!(
            live_repo
                .list_personalization_entries()
                .await
                .expect("live repository should decrypt imported personalization"),
            personalization_entries
        );
        let _ = super::remove_path_if_exists(&swapped.rollback_dir);
    }

    #[test]
    fn storage_swap_and_rollback_restore_previous_files() {
        let temp = tempdir().expect("tempdir");
        let data_dir = temp.path().join("data");
        let staging_dir = temp.path().join("staging");
        std::fs::create_dir_all(data_dir.join("vault")).expect("create data vault");
        std::fs::create_dir_all(staging_dir.join("vault")).expect("create staging vault");

        std::fs::write(data_dir.join("artifacts.db"), b"old-db").expect("write old db");
        std::fs::write(data_dir.join("artifacts.db-wal"), b"old-wal").expect("write old wal");
        std::fs::write(data_dir.join("vault").join("old.txt"), b"old-vault")
            .expect("write old vault");

        std::fs::write(staging_dir.join("artifacts.db"), b"new-db").expect("write new db");
        std::fs::write(staging_dir.join("artifacts.db-wal"), b"new-wal").expect("write new wal");
        std::fs::write(staging_dir.join("vault").join("new.txt"), b"new-vault")
            .expect("write new vault");

        let swapped =
            swap_storage_with_rollback(&data_dir, &staging_dir).expect("swap should succeed");
        assert_eq!(
            std::fs::read(data_dir.join("artifacts.db")).expect("read swapped db"),
            b"new-db"
        );
        assert!(
            swapped.rollback_dir.join("artifacts.db").exists(),
            "rollback db should exist"
        );

        restore_storage_from_rollback(&data_dir, &swapped.rollback_dir)
            .expect("rollback restore should succeed");
        assert_eq!(
            std::fs::read(data_dir.join("artifacts.db")).expect("read restored db"),
            b"old-db"
        );
        assert!(
            data_dir.join("vault").join("old.txt").exists(),
            "old vault should be restored"
        );
    }
}
