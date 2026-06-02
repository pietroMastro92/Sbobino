use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use sbobino_application::ArtifactQuery;
use sbobino_domain::{
    AppSettings, ArtifactKind, ArtifactSourceOrigin, AutomaticImportActivityEntry,
    AutomaticImportActivityLevel, AutomaticImportPreset, AutomaticImportQuarantineItem,
    AutomaticImportSource, AutomaticImportSourceHealth, AutomaticImportSourceStatus, LanguageCode,
    SpeechModel,
};

use crate::{
    commands::transcription::{spawn_transcription_job, StartTranscriptionPayload},
    error::CommandError,
    state::AppState,
};

pub(crate) const IMPORT_SOURCE_ID_METADATA_KEY: &str = "auto_import_source_id";
pub(crate) const IMPORT_SOURCE_LABEL_METADATA_KEY: &str = "auto_import_source_label";
pub(crate) const IMPORT_PRESET_METADATA_KEY: &str = "auto_import_preset";
pub(crate) const IMPORT_WORKSPACE_METADATA_KEY: &str = "workspace_id";
pub(crate) const IMPORT_FOLDER_METADATA_KEY: &str = "auto_import_folder_path";
pub(crate) const IMPORT_FILE_PATH_METADATA_KEY: &str = "auto_import_source_path";
pub(crate) const IMPORT_DETECTED_AT_METADATA_KEY: &str = "auto_import_detected_at";
pub(crate) const IMPORT_SCAN_REASON_METADATA_KEY: &str = "auto_import_scan_reason";
pub(crate) const IMPORT_SCAN_TRIGGER_METADATA_KEY: &str = "auto_import_trigger";
pub(crate) const IMPORT_GENERATE_SUMMARY_METADATA_KEY: &str = "auto_import_generate_summary";
pub(crate) const IMPORT_GENERATE_FAQS_METADATA_KEY: &str = "auto_import_generate_faqs";
pub(crate) const IMPORT_GENERATE_PRESET_OUTPUT_METADATA_KEY: &str =
    "auto_import_generate_preset_output";

#[derive(Debug, Deserialize, Default)]
pub struct ScanAutomaticImportPayload {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAutomaticImportQuarantinePayload {
    pub id: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AutomaticImportQueuedJob {
    pub job_id: String,
    pub source_id: String,
    pub source_label: String,
    pub folder_path: String,
    pub file_path: String,
    pub title: String,
    pub model: SpeechModel,
    pub language: LanguageCode,
    pub workspace_id: Option<String>,
    pub preset: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AutomaticImportScanResponse {
    pub reason: String,
    pub started_at: String,
    pub finished_at: String,
    pub scanned_sources: usize,
    pub scanned_files: usize,
    pub queued_jobs: Vec<AutomaticImportQueuedJob>,
    pub skipped_existing: usize,
    pub skipped_missing_sources: usize,
    pub skipped_unreadable: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
struct DiscoveredImportCandidate {
    source_id: String,
    source_label: String,
    source_preset: AutomaticImportPreset,
    model: SpeechModel,
    language: LanguageCode,
    workspace_id: Option<String>,
    folder_path: String,
    file_path: String,
    title: String,
    fingerprint_json: String,
    fingerprint_key: String,
    enable_ai_post_processing: bool,
    generate_summary: bool,
    generate_faqs: bool,
    generate_preset_output: bool,
}

#[derive(Debug, Default)]
struct ExistingImportIndex {
    fingerprint_jsons: HashSet<String>,
    fingerprint_keys: HashSet<String>,
    source_paths: HashSet<String>,
}

impl ExistingImportIndex {
    fn insert_fingerprint_json(&mut self, fingerprint_json: &str) {
        let _ = self.fingerprint_jsons.insert(fingerprint_json.to_string());
        if let Some(key) = fingerprint_dedupe_key(fingerprint_json) {
            let _ = self.fingerprint_keys.insert(key);
        }
        if let Some(path) = fingerprint_path(fingerprint_json) {
            let _ = self.source_paths.insert(path);
        }
    }

    fn insert_source_path(&mut self, path: &str) {
        let _ = self.source_paths.insert(path.to_string());
    }

    fn matches(&self, candidate: &DiscoveredImportCandidate) -> bool {
        self.fingerprint_jsons.contains(&candidate.fingerprint_json)
            || self.fingerprint_keys.contains(&candidate.fingerprint_key)
            || self.source_paths.contains(&candidate.file_path)
    }
}

#[derive(Debug, Default)]
struct ScanCollection {
    scanned_sources: usize,
    scanned_files: usize,
    skipped_missing_sources: usize,
    skipped_unreadable: usize,
    errors: Vec<String>,
    candidates: Vec<DiscoveredImportCandidate>,
    source_summaries: BTreeMap<String, SourceScanSummary>,
    quarantined_items: Vec<PendingQuarantineItem>,
    successful_paths: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
struct SourceScanSummary {
    source_id: String,
    source_label: String,
    scanned_files: usize,
    queued_jobs: usize,
    skipped_existing: usize,
    unreadable_files: usize,
    last_error: Option<String>,
    health: AutomaticImportSourceHealth,
}

#[derive(Debug, Clone)]
struct PendingQuarantineItem {
    source_id: Option<String>,
    source_label: Option<String>,
    file_path: String,
    fingerprint_key: Option<String>,
    reason: String,
}

#[tauri::command]
pub async fn scan_automatic_import(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<ScanAutomaticImportPayload>,
) -> Result<AutomaticImportScanResponse, CommandError> {
    let reason = payload.and_then(|value| value.reason);
    scan_automatic_import_inner(app, state.inner().clone(), reason).await
}

#[tauri::command]
pub async fn retry_automatic_import_quarantine_item(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: UpdateAutomaticImportQuarantinePayload,
) -> Result<AutomaticImportScanResponse, CommandError> {
    increment_quarantine_retry_count(state.inner(), &payload.id).await?;
    scan_automatic_import_inner(app, state.inner().clone(), Some("retry".to_string())).await
}

#[tauri::command]
pub async fn clear_automatic_import_quarantine_item(
    state: State<'_, AppState>,
    payload: UpdateAutomaticImportQuarantinePayload,
) -> Result<AppSettings, CommandError> {
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let mut automation = settings.automation.clone();
    automation
        .quarantined_items
        .retain(|item| item.id != payload.id.trim());
    let updated = state
        .settings_service
        .update_partial(None, None, Some(automation), None, None, None)
        .await
        .map_err(CommandError::from)?;
    Ok(updated.redacted_clone())
}

async fn scan_automatic_import_inner(
    app: tauri::AppHandle,
    state: AppState,
    reason_override: Option<String>,
) -> Result<AutomaticImportScanResponse, CommandError> {
    let started_at = Utc::now().to_rfc3339();
    let reason = reason_override
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "manual".to_string());

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let trigger = if reason == "startup" {
        "app_start"
    } else {
        "manual_scan"
    };

    if !settings.automation.enabled {
        let finished_at = Utc::now().to_rfc3339();
        return Ok(AutomaticImportScanResponse {
            reason,
            started_at,
            finished_at,
            scanned_sources: 0,
            scanned_files: 0,
            queued_jobs: Vec::new(),
            skipped_existing: 0,
            skipped_missing_sources: 0,
            skipped_unreadable: 0,
            errors: Vec::new(),
        });
    }

    let existing_artifacts = state
        .artifact_service
        .list(ArtifactQuery {
            kind: Some(ArtifactKind::File),
            query: None,
            limit: Some(500),
            offset: Some(0),
        })
        .await
        .map_err(CommandError::from)?;
    let mut existing_index = build_existing_import_index(&existing_artifacts);

    let settings_for_scan = settings.clone();
    let scan_collection =
        tokio::task::spawn_blocking(move || collect_candidates(&settings_for_scan))
            .await
            .map_err(|error| CommandError::new("automatic_import", error.to_string()))?;
    let mut source_summaries = scan_collection.source_summaries.clone();

    let mut queued_jobs = Vec::new();
    let mut skipped_existing = 0usize;

    for candidate in scan_collection.candidates {
        if existing_index.matches(&candidate) {
            skipped_existing += 1;
            if let Some(summary) = source_summaries.get_mut(&candidate.source_id) {
                summary.skipped_existing += 1;
            }
            continue;
        }

        let mut metadata = BTreeMap::new();
        metadata.insert(
            IMPORT_SOURCE_ID_METADATA_KEY.to_string(),
            candidate.source_id.clone(),
        );
        metadata.insert(
            IMPORT_SOURCE_LABEL_METADATA_KEY.to_string(),
            candidate.source_label.clone(),
        );
        metadata.insert(
            IMPORT_PRESET_METADATA_KEY.to_string(),
            automatic_import_preset_str(&candidate.source_preset).to_string(),
        );
        metadata.insert(
            IMPORT_FOLDER_METADATA_KEY.to_string(),
            candidate.folder_path.clone(),
        );
        metadata.insert(
            IMPORT_FILE_PATH_METADATA_KEY.to_string(),
            candidate.file_path.clone(),
        );
        metadata.insert(
            IMPORT_DETECTED_AT_METADATA_KEY.to_string(),
            Utc::now().to_rfc3339(),
        );
        metadata.insert(IMPORT_SCAN_REASON_METADATA_KEY.to_string(), reason.clone());
        metadata.insert(
            IMPORT_SCAN_TRIGGER_METADATA_KEY.to_string(),
            trigger.to_string(),
        );
        metadata.insert(
            IMPORT_GENERATE_SUMMARY_METADATA_KEY.to_string(),
            candidate.generate_summary.to_string(),
        );
        metadata.insert(
            IMPORT_GENERATE_FAQS_METADATA_KEY.to_string(),
            candidate.generate_faqs.to_string(),
        );
        metadata.insert(
            IMPORT_GENERATE_PRESET_OUTPUT_METADATA_KEY.to_string(),
            candidate.generate_preset_output.to_string(),
        );
        if let Some(workspace_id) = candidate.workspace_id.clone() {
            metadata.insert(IMPORT_WORKSPACE_METADATA_KEY.to_string(), workspace_id);
        }

        let response = spawn_transcription_job(
            app.clone(),
            state.clone(),
            StartTranscriptionPayload {
                input_path: candidate.file_path.clone(),
                engine: settings.transcription.engine.clone(),
                model: candidate.model.clone(),
                parakeet_model: settings.transcription.parakeet_model.clone(),
                language: candidate.language.clone(),
                enable_ai: candidate.enable_ai_post_processing,
                whisper_options: settings.transcription.whisper_options.clone(),
                title: Some(candidate.title.clone()),
                parent_id: None,
                source_origin: Some(ArtifactSourceOrigin::Imported),
                metadata,
                source_fingerprint_json: Some(candidate.fingerprint_json.clone()),
            },
        )
        .await?;

        queued_jobs.push(AutomaticImportQueuedJob {
            job_id: response.job_id,
            source_id: candidate.source_id.clone(),
            source_label: candidate.source_label.clone(),
            folder_path: candidate.folder_path.clone(),
            file_path: candidate.file_path.clone(),
            title: candidate.title.clone(),
            model: candidate.model.clone(),
            language: candidate.language.clone(),
            workspace_id: candidate.workspace_id.clone(),
            preset: automatic_import_preset_str(&candidate.source_preset).to_string(),
        });
        if let Some(summary) = source_summaries.get_mut(&candidate.source_id) {
            summary.queued_jobs += 1;
            if !matches!(summary.health, AutomaticImportSourceHealth::Error) {
                summary.health = AutomaticImportSourceHealth::Healthy;
            }
        }
        existing_index.insert_fingerprint_json(&candidate.fingerprint_json);
        existing_index.insert_source_path(&candidate.file_path);
    }

    let finished_at = Utc::now().to_rfc3339();
    persist_scan_state(
        &state,
        &settings,
        &source_summaries,
        &scan_collection.quarantined_items,
        &scan_collection.successful_paths,
        &reason,
        trigger,
        &finished_at,
    )
    .await?;

    Ok(AutomaticImportScanResponse {
        reason,
        started_at,
        finished_at,
        scanned_sources: scan_collection.scanned_sources,
        scanned_files: scan_collection.scanned_files,
        queued_jobs,
        skipped_existing,
        skipped_missing_sources: scan_collection.skipped_missing_sources,
        skipped_unreadable: scan_collection.skipped_unreadable,
        errors: scan_collection.errors,
    })
}

fn collect_candidates(settings: &AppSettings) -> ScanCollection {
    let mut output = ScanCollection::default();
    let allowed_extensions = normalized_allowed_extensions(settings);
    let excluded_directories = normalized_excluded_directories(settings);

    for source in settings
        .automation
        .watched_sources
        .iter()
        .filter(|source| source.enabled)
    {
        output.scanned_sources += 1;
        let entry = output
            .source_summaries
            .entry(source.id.clone())
            .or_insert_with(|| SourceScanSummary {
                source_id: source.id.clone(),
                source_label: if source.label.trim().is_empty() {
                    file_label(&source.folder_path)
                } else {
                    source.label.clone()
                },
                health: AutomaticImportSourceHealth::Idle,
                ..SourceScanSummary::default()
            });
        let folder_path = source.folder_path.trim();
        if folder_path.is_empty() {
            output.skipped_missing_sources += 1;
            let error = format!(
                "Auto-import source '{}' is missing a folder path.",
                source.label
            );
            entry.health = AutomaticImportSourceHealth::Error;
            entry.last_error = Some(error.clone());
            output.errors.push(error);
            continue;
        }

        let root = PathBuf::from(folder_path);
        if !root.is_dir() {
            output.skipped_missing_sources += 1;
            let error = format!(
                "Auto-import source '{}' cannot access {}.",
                source.label, folder_path
            );
            entry.health = AutomaticImportSourceHealth::Error;
            entry.last_error = Some(error.clone());
            output.errors.push(error);
            continue;
        }

        let mut discovered_paths = Vec::new();
        walk_audio_files(
            &root,
            source.recursive,
            &allowed_extensions,
            &excluded_directories,
            &mut discovered_paths,
        );
        discovered_paths.sort();
        entry.scanned_files = discovered_paths.len();

        for file_path in discovered_paths {
            output.scanned_files += 1;
            let normalized_path = normalize_path_string(&file_path);
            match build_candidate(source, &file_path) {
                Ok(candidate) => {
                    output.successful_paths.insert(normalized_path);
                    output.candidates.push(candidate);
                }
                Err(error) => {
                    output.skipped_unreadable += 1;
                    entry.unreadable_files += 1;
                    entry.health = AutomaticImportSourceHealth::Warning;
                    entry.last_error = Some(error.clone());
                    output.quarantined_items.push(PendingQuarantineItem {
                        source_id: Some(source.id.clone()),
                        source_label: Some(entry.source_label.clone()),
                        file_path: normalized_path.clone(),
                        fingerprint_key: quarantine_fingerprint_key(&file_path),
                        reason: error.clone(),
                    });
                    output
                        .errors
                        .push(format!("{} ({})", error, file_path.to_string_lossy()));
                }
            }
        }
        if matches!(entry.health, AutomaticImportSourceHealth::Idle) {
            entry.health = AutomaticImportSourceHealth::Healthy;
        }
    }

    output
}

async fn persist_scan_state(
    state: &AppState,
    settings: &AppSettings,
    source_summaries: &BTreeMap<String, SourceScanSummary>,
    quarantined_updates: &[PendingQuarantineItem],
    successful_paths: &HashSet<String>,
    reason: &str,
    trigger: &str,
    finished_at: &str,
) -> Result<(), CommandError> {
    let mut automation = settings.automation.clone();
    let mut next_statuses = Vec::with_capacity(automation.watched_sources.len());

    for source in automation
        .watched_sources
        .iter()
        .filter(|source| source.enabled)
    {
        let previous = automation
            .source_statuses
            .iter()
            .find(|status| status.source_id == source.id)
            .cloned()
            .unwrap_or_default();
        let status = if let Some(summary) = source_summaries.get(&source.id) {
            let is_error = matches!(summary.health, AutomaticImportSourceHealth::Error);
            AutomaticImportSourceStatus {
                source_id: source.id.clone(),
                source_label: summary.source_label.clone(),
                health: summary.health.clone(),
                last_scan_at: Some(finished_at.to_string()),
                last_success_at: if is_error {
                    previous.last_success_at
                } else {
                    Some(finished_at.to_string())
                },
                last_failure_at: if is_error {
                    Some(finished_at.to_string())
                } else {
                    previous.last_failure_at
                },
                last_error: summary.last_error.clone(),
                last_scan_reason: Some(reason.to_string()),
                last_trigger: Some(trigger.to_string()),
                last_scanned_files: summary.scanned_files as u32,
                last_queued_jobs: summary.queued_jobs as u32,
                last_skipped_existing: summary.skipped_existing as u32,
                watcher_mode: "periodic_scan".to_string(),
            }
        } else {
            AutomaticImportSourceStatus {
                source_id: source.id.clone(),
                source_label: if source.label.trim().is_empty() {
                    file_label(&source.folder_path)
                } else {
                    source.label.clone()
                },
                health: previous.health,
                last_scan_at: previous.last_scan_at,
                last_success_at: previous.last_success_at,
                last_failure_at: previous.last_failure_at,
                last_error: previous.last_error,
                last_scan_reason: previous.last_scan_reason,
                last_trigger: previous.last_trigger,
                last_scanned_files: previous.last_scanned_files,
                last_queued_jobs: previous.last_queued_jobs,
                last_skipped_existing: previous.last_skipped_existing,
                watcher_mode: previous.watcher_mode,
            }
        };
        next_statuses.push(status);
    }

    let mut activity = automation.recent_activity.clone();
    for summary in source_summaries.values() {
        let level = if matches!(summary.health, AutomaticImportSourceHealth::Error) {
            AutomaticImportActivityLevel::Error
        } else if matches!(summary.health, AutomaticImportSourceHealth::Warning) {
            AutomaticImportActivityLevel::Warning
        } else {
            AutomaticImportActivityLevel::Info
        };
        let message = if let Some(error) = summary.last_error.as_ref() {
            format!(
                "{}: {} (queued {}, skipped {}, unreadable {})",
                summary.source_label,
                error,
                summary.queued_jobs,
                summary.skipped_existing,
                summary.unreadable_files
            )
        } else {
            format!(
                "{}: scanned {}, queued {}, skipped {}",
                summary.source_label,
                summary.scanned_files,
                summary.queued_jobs,
                summary.skipped_existing
            )
        };
        activity.push(AutomaticImportActivityEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: finished_at.to_string(),
            source_id: Some(summary.source_id.clone()),
            level,
            message,
        });
    }
    trim_automatic_import_activity(&mut activity);

    let mut quarantined_items = settings.automation.quarantined_items.clone();
    quarantined_items.retain(|item| !successful_paths.contains(&item.file_path));
    for pending in quarantined_updates {
        upsert_quarantined_item(
            &mut quarantined_items,
            pending.source_id.clone(),
            pending.source_label.clone(),
            pending.file_path.clone(),
            pending.fingerprint_key.clone(),
            pending.reason.clone(),
            finished_at,
        );
    }

    automation.source_statuses = next_statuses;
    automation.recent_activity = activity;
    automation.quarantined_items = quarantined_items;
    state
        .settings_service
        .update_partial(None, None, Some(automation), None, None, None)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

async fn increment_quarantine_retry_count(
    state: &AppState,
    quarantine_id: &str,
) -> Result<(), CommandError> {
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let mut automation = settings.automation.clone();
    let now = Utc::now().to_rfc3339();
    if let Some(item) = automation
        .quarantined_items
        .iter_mut()
        .find(|item| item.id == quarantine_id.trim())
    {
        item.retry_count = item.retry_count.saturating_add(1);
        item.last_detected_at = now.clone();
        automation
            .recent_activity
            .push(AutomaticImportActivityEntry {
                id: Uuid::new_v4().to_string(),
                timestamp: now,
                source_id: item.source_id.clone(),
                level: AutomaticImportActivityLevel::Info,
                message: format!(
                    "Retry requested for quarantined file {}",
                    item.source_label
                        .clone()
                        .unwrap_or_else(|| file_label(&item.file_path))
                ),
            });
    }
    trim_automatic_import_activity(&mut automation.recent_activity);
    state
        .settings_service
        .update_partial(None, None, Some(automation), None, None, None)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

fn upsert_quarantined_item(
    items: &mut Vec<AutomaticImportQuarantineItem>,
    source_id: Option<String>,
    source_label: Option<String>,
    file_path: String,
    fingerprint_key: Option<String>,
    reason: String,
    timestamp: &str,
) {
    if let Some(item) = items.iter_mut().find(|item| item.file_path == file_path) {
        item.source_id = source_id;
        item.source_label = source_label;
        item.fingerprint_key = fingerprint_key;
        item.reason = reason;
        item.last_detected_at = timestamp.to_string();
        return;
    }

    items.push(AutomaticImportQuarantineItem {
        id: Uuid::new_v4().to_string(),
        source_id,
        source_label,
        file_path,
        fingerprint_key,
        reason,
        first_detected_at: timestamp.to_string(),
        last_detected_at: timestamp.to_string(),
        retry_count: 0,
    });
}

fn trim_automatic_import_activity(activity: &mut Vec<AutomaticImportActivityEntry>) {
    if activity.len() > 40 {
        let start = activity.len().saturating_sub(40);
        let trimmed = activity.split_off(start);
        *activity = trimmed;
    }
}

pub(crate) async fn record_automatic_import_failure(
    state: &AppState,
    metadata: &BTreeMap<String, String>,
    reason: &str,
) -> Result<(), CommandError> {
    let Some(file_path) = metadata.get(IMPORT_FILE_PATH_METADATA_KEY).cloned() else {
        return Ok(());
    };
    let timestamp = Utc::now().to_rfc3339();
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let mut automation = settings.automation.clone();
    upsert_quarantined_item(
        &mut automation.quarantined_items,
        metadata.get(IMPORT_SOURCE_ID_METADATA_KEY).cloned(),
        metadata.get(IMPORT_SOURCE_LABEL_METADATA_KEY).cloned(),
        file_path.clone(),
        None,
        reason.to_string(),
        &timestamp,
    );
    if let Some(source_id) = metadata.get(IMPORT_SOURCE_ID_METADATA_KEY) {
        if let Some(status) = automation
            .source_statuses
            .iter_mut()
            .find(|status| &status.source_id == source_id)
        {
            status.health = AutomaticImportSourceHealth::Error;
            status.last_failure_at = Some(timestamp.clone());
            status.last_error = Some(reason.to_string());
        }
    }
    automation
        .recent_activity
        .push(AutomaticImportActivityEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: timestamp.clone(),
            source_id: metadata.get(IMPORT_SOURCE_ID_METADATA_KEY).cloned(),
            level: AutomaticImportActivityLevel::Error,
            message: format!(
                "{}: {}",
                metadata
                    .get(IMPORT_SOURCE_LABEL_METADATA_KEY)
                    .cloned()
                    .unwrap_or_else(|| file_label(&file_path)),
                reason
            ),
        });
    trim_automatic_import_activity(&mut automation.recent_activity);
    state
        .settings_service
        .update_partial(None, None, Some(automation), None, None, None)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

pub(crate) async fn record_automatic_import_success(
    state: &AppState,
    metadata: &BTreeMap<String, String>,
) -> Result<(), CommandError> {
    let Some(file_path) = metadata.get(IMPORT_FILE_PATH_METADATA_KEY) else {
        return Ok(());
    };
    let timestamp = Utc::now().to_rfc3339();
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let mut automation = settings.automation.clone();
    automation
        .quarantined_items
        .retain(|item| item.file_path != *file_path);
    if let Some(source_id) = metadata.get(IMPORT_SOURCE_ID_METADATA_KEY) {
        if let Some(status) = automation
            .source_statuses
            .iter_mut()
            .find(|status| &status.source_id == source_id)
        {
            status.health = AutomaticImportSourceHealth::Healthy;
            status.last_success_at = Some(timestamp);
            status.last_error = None;
        }
    }
    state
        .settings_service
        .update_partial(None, None, Some(automation), None, None, None)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

fn build_existing_import_index(
    artifacts: &[sbobino_domain::TranscriptArtifact],
) -> ExistingImportIndex {
    let mut index = ExistingImportIndex::default();
    for artifact in artifacts {
        if let Some(fingerprint) = artifact.source_fingerprint_json.as_deref() {
            index.insert_fingerprint_json(fingerprint);
        }
        if let Some(source_path) = artifact.metadata.get(IMPORT_FILE_PATH_METADATA_KEY) {
            index.insert_source_path(source_path);
        }
    }
    index
}

fn normalized_allowed_extensions(settings: &AppSettings) -> HashSet<String> {
    settings
        .automation
        .allowed_extensions
        .iter()
        .map(|value| value.trim().trim_start_matches('.').to_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalized_excluded_directories(settings: &AppSettings) -> Vec<String> {
    settings
        .automation
        .excluded_folders
        .iter()
        .map(|value| normalize_path_string(Path::new(value)))
        .collect()
}

fn walk_audio_files(
    directory: &Path,
    recursive: bool,
    allowed_extensions: &HashSet<String>,
    excluded_directories: &[String],
    output: &mut Vec<PathBuf>,
) {
    if is_path_excluded(directory, excluded_directories) {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            if recursive {
                walk_audio_files(
                    &path,
                    recursive,
                    allowed_extensions,
                    excluded_directories,
                    output,
                );
            }
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        if allowed_extensions.contains(&extension) {
            output.push(path);
        }
    }
}

fn is_path_excluded(path: &Path, excluded_directories: &[String]) -> bool {
    let normalized = normalize_path_string(path);
    excluded_directories.iter().any(|excluded| {
        normalized == *excluded
            || normalized
                .strip_prefix(excluded)
                .is_some_and(|suffix| suffix.starts_with(std::path::MAIN_SEPARATOR))
    })
}

fn build_candidate(
    source: &AutomaticImportSource,
    file_path: &Path,
) -> Result<DiscoveredImportCandidate, String> {
    let metadata = fs::metadata(file_path)
        .map_err(|error| format!("failed to inspect auto-import candidate: {error}"))?;
    let size_bytes = metadata.len();
    let modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(system_time_to_unix_ms)
        .unwrap_or_default();
    let sha256 = file_sha256(file_path)
        .map_err(|error| format!("failed to fingerprint auto-import candidate: {error}"))?;

    let normalized_path = normalize_path_string(file_path);
    let fingerprint_key = format!("{size_bytes}:{modified_unix_ms}:{sha256}");
    let fingerprint_json = serde_json::json!({
        "path": normalized_path,
        "size_bytes": size_bytes,
        "modified_unix_ms": modified_unix_ms,
        "sha256": sha256,
        "dedupe_key": fingerprint_key,
    })
    .to_string();

    Ok(DiscoveredImportCandidate {
        source_id: source.id.clone(),
        source_label: if source.label.trim().is_empty() {
            file_label(&source.folder_path)
        } else {
            source.label.clone()
        },
        source_preset: source.preset.clone(),
        model: source.model.clone(),
        language: source.language.clone(),
        workspace_id: source.workspace_id.clone(),
        folder_path: source.folder_path.clone(),
        file_path: normalized_path,
        title: file_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_else(|| {
                file_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Imported audio")
            })
            .to_string(),
        fingerprint_json,
        fingerprint_key,
        enable_ai_post_processing: source.enable_ai_post_processing,
        generate_summary: source.post_processing.generate_summary,
        generate_faqs: source.post_processing.generate_faqs,
        generate_preset_output: source.post_processing.generate_preset_output,
    })
}

fn quarantine_fingerprint_key(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    let modified_unix_ms = metadata.modified().ok().and_then(system_time_to_unix_ms)?;
    Some(format!("{}:{modified_unix_ms}", metadata.len()))
}

fn system_time_to_unix_ms(value: SystemTime) -> Option<u128> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

fn normalize_path_string(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_string()
}

fn file_sha256(path: &Path) -> Result<String, std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn fingerprint_dedupe_key(fingerprint_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(fingerprint_json)
        .ok()?
        .get("dedupe_key")?
        .as_str()
        .map(str::to_string)
}

fn fingerprint_path(fingerprint_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(fingerprint_json)
        .ok()?
        .get("path")?
        .as_str()
        .map(str::to_string)
}

fn automatic_import_preset_str(preset: &AutomaticImportPreset) -> &'static str {
    match preset {
        AutomaticImportPreset::General => "general",
        AutomaticImportPreset::Lecture => "lecture",
        AutomaticImportPreset::Meeting => "meeting",
        AutomaticImportPreset::Interview => "interview",
        AutomaticImportPreset::VoiceMemo => "voice_memo",
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use sbobino_domain::AutomaticImportSettings;

    #[test]
    fn collect_candidates_filters_supported_extensions_and_builds_hash_dedupe() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path();
        fs::write(root.join("lecture.m4a"), b"audio").expect("write audio");
        fs::write(root.join("notes.txt"), b"text").expect("write note");

        let mut settings = AppSettings::default();
        settings.automation = AutomaticImportSettings {
            enabled: true,
            run_scan_on_app_start: true,
            scan_interval_minutes: 15,
            allowed_extensions: vec!["m4a".to_string()],
            watched_sources: vec![AutomaticImportSource {
                id: "source_a".to_string(),
                label: "Lectures".to_string(),
                folder_path: root.to_string_lossy().to_string(),
                enabled: true,
                preset: AutomaticImportPreset::Lecture,
                model: SpeechModel::Small,
                language: LanguageCode::It,
                workspace_id: Some("uni".to_string()),
                recursive: true,
                enable_ai_post_processing: false,
                post_processing: sbobino_domain::AutomaticImportPostProcessingSettings::default(),
            }],
            excluded_folders: Vec::new(),
            source_statuses: Vec::new(),
            recent_activity: Vec::new(),
            quarantined_items: Vec::new(),
        };

        let scan = collect_candidates(&settings);
        assert_eq!(scan.scanned_sources, 1);
        assert_eq!(scan.scanned_files, 1);
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].source_id, "source_a");
        assert_eq!(scan.candidates[0].workspace_id.as_deref(), Some("uni"));
        assert_eq!(scan.candidates[0].model, SpeechModel::Small);
        assert_eq!(scan.candidates[0].language, LanguageCode::It);
        assert!(scan.candidates[0].fingerprint_json.contains("\"sha256\""));
        assert!(scan.candidates[0]
            .fingerprint_json
            .contains("\"dedupe_key\""));
    }

    #[test]
    fn collect_candidates_preserves_auto_language_for_source() {
        let temp = tempdir().expect("temp dir");
        let root = temp.path();
        fs::write(root.join("italian.m4a"), b"audio").expect("write audio");

        let mut settings = AppSettings::default();
        settings.automation = AutomaticImportSettings {
            enabled: true,
            run_scan_on_app_start: true,
            scan_interval_minutes: 15,
            allowed_extensions: vec!["m4a".to_string()],
            watched_sources: vec![AutomaticImportSource {
                id: "source_auto".to_string(),
                label: "Italian Auto".to_string(),
                folder_path: root.to_string_lossy().to_string(),
                enabled: true,
                preset: AutomaticImportPreset::General,
                model: SpeechModel::LargeTurbo,
                language: LanguageCode::Auto,
                workspace_id: None,
                recursive: true,
                enable_ai_post_processing: false,
                post_processing: sbobino_domain::AutomaticImportPostProcessingSettings::default(),
            }],
            excluded_folders: Vec::new(),
            source_statuses: Vec::new(),
            recent_activity: Vec::new(),
            quarantined_items: Vec::new(),
        };

        let scan = collect_candidates(&settings);
        assert_eq!(scan.candidates.len(), 1);
        assert_eq!(scan.candidates[0].model, SpeechModel::LargeTurbo);
        assert_eq!(scan.candidates[0].language, LanguageCode::Auto);
    }

    #[test]
    fn existing_index_matches_by_dedupe_key_even_when_path_changes() {
        let original = serde_json::json!({
            "path": "/tmp/old-path.m4a",
            "size_bytes": 100,
            "modified_unix_ms": 42,
            "sha256": "abc",
            "dedupe_key": "100:42:abc"
        })
        .to_string();
        let moved = DiscoveredImportCandidate {
            source_id: "source_a".to_string(),
            source_label: "Lectures".to_string(),
            source_preset: AutomaticImportPreset::Lecture,
            model: SpeechModel::Medium,
            language: LanguageCode::En,
            workspace_id: None,
            folder_path: "/tmp".to_string(),
            file_path: "/tmp/new-path.m4a".to_string(),
            title: "new-path".to_string(),
            fingerprint_json: serde_json::json!({
                "path": "/tmp/new-path.m4a",
                "size_bytes": 100,
                "modified_unix_ms": 42,
                "sha256": "abc",
                "dedupe_key": "100:42:abc"
            })
            .to_string(),
            fingerprint_key: "100:42:abc".to_string(),
            enable_ai_post_processing: false,
            generate_summary: true,
            generate_faqs: true,
            generate_preset_output: true,
        };

        let mut index = ExistingImportIndex::default();
        index.insert_fingerprint_json(&original);
        assert!(index.matches(&moved));
    }
}
