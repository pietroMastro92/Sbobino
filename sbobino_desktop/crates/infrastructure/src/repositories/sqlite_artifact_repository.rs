use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, Row, Transaction,
};
use sha2::{Digest, Sha256};

use sbobino_application::{ApplicationError, ArtifactRepository};
use sbobino_domain::{
    ArtifactAudioBackfillStatus, ArtifactChatMessage, ArtifactKind, ArtifactSourceOrigin,
    PersonalizationEntry, PersonalizationEntryKind, TranscriptArtifact,
};

use crate::secure_storage::{decrypt_from_file, encrypt_to_file, SecureStorage};

const HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY: &str = "has_optimized_transcript";
const EMOTION_ANALYSIS_METADATA_KEY: &str = "emotion_analysis_v1";
const EMOTION_ANALYSIS_GENERATED_AT_METADATA_KEY: &str = "emotion_analysis_generated_at";

#[derive(Debug, Clone)]
pub struct SqliteArtifactRepository {
    db_path: PathBuf,
    vault_root: PathBuf,
    secure_storage: SecureStorage,
}

#[derive(Debug, Clone)]
struct AudioImportResult {
    available: bool,
    backfill_status: ArtifactAudioBackfillStatus,
    encrypted_rel_path: Option<String>,
    extension: Option<String>,
    mime_type: Option<String>,
    sha256: Option<String>,
    byte_size: Option<u64>,
}

#[derive(Debug, Clone)]
struct ArtifactRow {
    id: String,
    job_id: String,
    kind: String,
    title_enc: Vec<u8>,
    source_label_enc: Vec<u8>,
    source_origin: String,
    raw_transcript_enc: Vec<u8>,
    optimized_transcript_enc: Vec<u8>,
    summary_enc: Vec<u8>,
    faqs_enc: Vec<u8>,
    metadata_json_enc: Vec<u8>,
    created_at: String,
    updated_at: String,
    revision: i64,
    audio_available: bool,
    audio_backfill_status: String,
    engine: Option<String>,
    model: Option<String>,
    language: Option<String>,
    parent_artifact_id: Option<String>,
    duration_seconds: Option<f32>,
    byte_size: Option<u64>,
    whisper_options_json_enc: Option<Vec<u8>>,
    diarization_settings_json_enc: Option<Vec<u8>>,
    ai_provider_snapshot_json_enc: Option<Vec<u8>>,
    source_fingerprint_json_enc: Option<Vec<u8>>,
    timeline_v2_json_enc: Option<Vec<u8>>,
    emotion_analysis_json_enc: Option<Vec<u8>>,
    emotion_generated_at: Option<String>,
}

#[derive(Debug, Clone)]
struct LegacyArtifact {
    id: String,
    job_id: String,
    title: String,
    kind: ArtifactKind,
    input_path: String,
    raw_transcript: String,
    optimized_transcript: String,
    summary: String,
    faqs: String,
    metadata_json: String,
    created_at: String,
    updated_at: String,
    is_deleted: bool,
    deleted_at: Option<String>,
}

impl SqliteArtifactRepository {
    pub fn new(db_path: PathBuf) -> Result<Self, ApplicationError> {
        let vault_root = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("vault")
            .join("artifacts");
        let fallback_root = db_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let secure_storage = SecureStorage::load_or_create_with_fallback(&fallback_root)?;
        let repo = Self {
            db_path,
            vault_root,
            secure_storage,
        };
        repo.init_schema()?;
        Ok(repo)
    }

    pub fn import_backup_chat_message(
        &self,
        message: &ArtifactChatMessage,
    ) -> Result<(), ApplicationError> {
        let conn = self.open_connection()?;
        let field = format!("chat_message:{}", message.id);
        let text_enc = self.encrypt_text(&message.artifact_id, &field, &message.text)?;
        conn.execute(
            r#"
            INSERT INTO artifact_chat_messages (
                id, artifact_id, role, text_enc, origin, status, provider, model, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                message.id,
                message.artifact_id,
                message.role,
                text_enc,
                message.origin,
                message.status,
                message.provider,
                message.model,
                message.created_at.to_rfc3339(),
            ],
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to save artifact chat message: {e}"))
        })?;
        Ok(())
    }

    pub fn import_backup_chat_summary(
        &self,
        artifact_id: &str,
        summary: &str,
    ) -> Result<(), ApplicationError> {
        let conn = self.open_connection()?;
        let summary_enc = self.encrypt_text(artifact_id, "chat_summary", summary)?;
        conn.execute(
            r#"
            INSERT INTO artifact_chat_context (artifact_id, summary_enc, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(artifact_id) DO UPDATE SET
                summary_enc = excluded.summary_enc,
                updated_at = excluded.updated_at
            "#,
            params![artifact_id, summary_enc, Utc::now().to_rfc3339()],
        )
        .map_err(|e| ApplicationError::Persistence(format!("failed to save chat summary: {e}")))?;
        Ok(())
    }

    pub fn import_backup_personalization_entry(
        &self,
        entry: &PersonalizationEntry,
    ) -> Result<(), ApplicationError> {
        self.upsert_personalization_entry_sync(entry)
    }

    fn upsert_personalization_entry_sync(
        &self,
        entry: &PersonalizationEntry,
    ) -> Result<(), ApplicationError> {
        if entry.id.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "personalization entry id cannot be empty".to_string(),
            ));
        }
        if entry.source_text.trim().is_empty() {
            return Err(ApplicationError::Validation(
                "personalization source text cannot be empty".to_string(),
            ));
        }
        if entry.kind == PersonalizationEntryKind::Correction
            && entry
                .replacement_text
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(ApplicationError::Validation(
                "personalization correction replacement text cannot be empty".to_string(),
            ));
        }
        let hit_count = i64::try_from(entry.hit_count).map_err(|_| {
            ApplicationError::Validation(
                "personalization entry hit count exceeds SQLite integer range".to_string(),
            )
        })?;
        let source_text_enc =
            self.encrypt_personalization_text(&entry.id, "source_text", &entry.source_text)?;
        let replacement_text_enc = entry
            .replacement_text
            .as_deref()
            .map(|value| self.encrypt_personalization_text(&entry.id, "replacement_text", value))
            .transpose()?;
        let language_code_enc = entry
            .language_code
            .as_deref()
            .map(|value| self.encrypt_personalization_text(&entry.id, "language_code", value))
            .transpose()?;
        let conn = self.open_connection()?;
        conn.execute(
            r#"
            INSERT INTO personalization_entries (
                id, kind, source_text_enc, replacement_text_enc, language_code_enc,
                enabled, hit_count, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                source_text_enc = excluded.source_text_enc,
                replacement_text_enc = excluded.replacement_text_enc,
                language_code_enc = excluded.language_code_enc,
                enabled = excluded.enabled,
                hit_count = excluded.hit_count,
                updated_at = excluded.updated_at
            "#,
            params![
                entry.id,
                entry.kind.as_str(),
                source_text_enc,
                replacement_text_enc,
                language_code_enc,
                if entry.enabled { 1_i64 } else { 0_i64 },
                hit_count,
                entry.created_at.to_rfc3339(),
                entry.updated_at.to_rfc3339(),
            ],
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to upsert personalization entry: {e}"))
        })?;
        Ok(())
    }

    fn init_schema(&self) -> Result<(), ApplicationError> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to create data directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        std::fs::create_dir_all(&self.vault_root).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create audio vault {}: {e}",
                self.vault_root.display()
            ))
        })?;

        let mut conn = self.open_connection()?;

        if self.needs_legacy_migration(&conn)? {
            self.migrate_legacy_plaintext_db(&mut conn)?;
        }

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS transcript_artifacts (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                title_enc BLOB NOT NULL,
                source_label_enc BLOB NOT NULL,
                source_origin TEXT NOT NULL,
                raw_transcript_enc BLOB NOT NULL,
                optimized_transcript_enc BLOB NOT NULL,
                summary_enc BLOB NOT NULL,
                faqs_enc BLOB NOT NULL,
                metadata_json_enc BLOB NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                revision INTEGER NOT NULL DEFAULT 0,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT,
                audio_available INTEGER NOT NULL DEFAULT 0,
                audio_backfill_status TEXT NOT NULL DEFAULT 'pending_backfill'
            );

            CREATE TABLE IF NOT EXISTS artifact_audio (
                artifact_id TEXT PRIMARY KEY,
                encrypted_rel_path TEXT,
                mime_type TEXT,
                file_extension TEXT,
                sha256 TEXT,
                byte_size INTEGER,
                duration_seconds REAL,
                imported_at TEXT,
                backfill_status TEXT NOT NULL DEFAULT 'pending_backfill',
                source_fingerprint_json_enc BLOB,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS artifact_processing (
                artifact_id TEXT PRIMARY KEY,
                engine TEXT,
                model TEXT,
                language TEXT,
                whisper_options_json_enc BLOB,
                diarization_settings_json_enc BLOB,
                ai_provider_snapshot_json_enc BLOB,
                parent_artifact_id TEXT,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS artifact_analysis (
                artifact_id TEXT PRIMARY KEY,
                timeline_v2_json_enc BLOB,
                emotion_analysis_json_enc BLOB,
                emotion_generated_at TEXT,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS artifact_chat_messages (
                id TEXT PRIMARY KEY,
                artifact_id TEXT NOT NULL,
                role TEXT NOT NULL,
                text_enc BLOB NOT NULL,
                origin TEXT NOT NULL,
                status TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS artifact_chat_context (
                artifact_id TEXT PRIMARY KEY,
                summary_enc BLOB NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            -- Personalization is additive to the artifact schema.  User text
            -- stays encrypted with the same per-installation SecureStorage
            -- key; only query-neutral metadata is kept in plaintext.
            CREATE TABLE IF NOT EXISTS personalization_entries (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                source_text_enc BLOB NOT NULL,
                replacement_text_enc BLOB,
                language_code_enc BLOB,
                enabled INTEGER NOT NULL DEFAULT 1,
                hit_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_artifact_chat_messages_artifact
            ON artifact_chat_messages(artifact_id, created_at, id);

            CREATE INDEX IF NOT EXISTS idx_personalization_entries_updated_at
            ON personalization_entries(updated_at DESC, id ASC);

            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_created_at
            ON transcript_artifacts(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_kind
            ON transcript_artifacts(kind);
            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_deleted
            ON transcript_artifacts(is_deleted, deleted_at DESC);
            "#,
        )
        .map_err(|e| ApplicationError::Persistence(format!("failed to initialize schema: {e}")))?;

        Ok(())
    }

    fn open_connection(&self) -> Result<Connection, ApplicationError> {
        let conn = Connection::open(&self.db_path).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to open sqlite database {}: {e}",
                self.db_path.display()
            ))
        })?;
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            PRAGMA secure_delete = ON;
            "#,
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to configure sqlite pragmas: {e}"))
        })?;
        Ok(conn)
    }

    fn needs_legacy_migration(&self, conn: &Connection) -> Result<bool, ApplicationError> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'transcript_artifacts'")
            .map_err(|e| ApplicationError::Persistence(format!("failed to inspect sqlite schema: {e}")))?;
        let exists = stmt
            .query_row([], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to inspect sqlite schema: {e}"))
            })?
            .is_some();
        if !exists {
            return Ok(false);
        }

        let columns = self.legacy_columns(conn)?;
        Ok(!columns.iter().any(|column| column == "title_enc"))
    }

    fn legacy_columns(&self, conn: &Connection) -> Result<Vec<String>, ApplicationError> {
        let mut pragma = conn
            .prepare("PRAGMA table_info(transcript_artifacts)")
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to prepare table_info pragma: {e}"))
            })?;
        let columns = pragma
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| ApplicationError::Persistence(format!("failed to query table_info: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to parse table_info: {e}"))
            })?;
        Ok(columns)
    }

    fn migrate_legacy_plaintext_db(&self, conn: &mut Connection) -> Result<(), ApplicationError> {
        let legacy_columns = self.legacy_columns(conn)?;
        let has_column = |name: &str| legacy_columns.iter().any(|column| column == name);
        let legacy_query = format!(
            r#"
                SELECT id,
                       job_id,
                       {},
                       {},
                       input_path,
                       raw_transcript,
                       optimized_transcript,
                       summary,
                       faqs,
                       metadata_json,
                       created_at,
                       {},
                       {},
                       {}
                FROM transcript_artifacts
            "#,
            if has_column("title") {
                "COALESCE(title, '')"
            } else {
                "''"
            },
            if has_column("kind") {
                "COALESCE(kind, 'file')"
            } else {
                "'file'"
            },
            if has_column("updated_at") {
                "COALESCE(updated_at, created_at)"
            } else {
                "created_at"
            },
            if has_column("is_deleted") {
                "COALESCE(is_deleted, 0)"
            } else {
                "0"
            },
            if has_column("deleted_at") {
                "deleted_at"
            } else {
                "NULL"
            }
        );
        let legacy = {
            let mut stmt = conn.prepare(&legacy_query).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to prepare legacy migration query: {e}"
                ))
            })?;

            let rows = stmt
                .query_map([], |row| {
                    let kind_raw: String = row.get(3)?;
                    Ok(LegacyArtifact {
                        id: row.get(0)?,
                        job_id: row.get(1)?,
                        title: row.get(2)?,
                        kind: if kind_raw.eq_ignore_ascii_case("realtime") {
                            ArtifactKind::Realtime
                        } else {
                            ArtifactKind::File
                        },
                        input_path: row.get(4)?,
                        raw_transcript: row.get(5)?,
                        optimized_transcript: row.get(6)?,
                        summary: row.get(7)?,
                        faqs: row.get(8)?,
                        metadata_json: row.get(9)?,
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
                        is_deleted: row.get::<_, i64>(12)? != 0,
                        deleted_at: row.get(13)?,
                    })
                })
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to read legacy artifacts: {e}"))
                })?;

            rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
                ApplicationError::Persistence(format!("failed to collect legacy artifacts: {e}"))
            })?
        };

        let tx = conn.transaction().map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to start legacy migration transaction: {e}"
            ))
        })?;
        tx.execute_batch(
            r#"
            ALTER TABLE transcript_artifacts RENAME TO transcript_artifacts_legacy_backup;
            "#,
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to rename legacy artifacts table: {e}"))
        })?;

        tx.execute_batch(
            r#"
            CREATE TABLE transcript_artifacts (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                title_enc BLOB NOT NULL,
                source_label_enc BLOB NOT NULL,
                source_origin TEXT NOT NULL,
                raw_transcript_enc BLOB NOT NULL,
                optimized_transcript_enc BLOB NOT NULL,
                summary_enc BLOB NOT NULL,
                faqs_enc BLOB NOT NULL,
                metadata_json_enc BLOB NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                revision INTEGER NOT NULL DEFAULT 0,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT,
                audio_available INTEGER NOT NULL DEFAULT 0,
                audio_backfill_status TEXT NOT NULL DEFAULT 'pending_backfill'
            );

            CREATE TABLE artifact_audio (
                artifact_id TEXT PRIMARY KEY,
                encrypted_rel_path TEXT,
                mime_type TEXT,
                file_extension TEXT,
                sha256 TEXT,
                byte_size INTEGER,
                duration_seconds REAL,
                imported_at TEXT,
                backfill_status TEXT NOT NULL DEFAULT 'pending_backfill',
                source_fingerprint_json_enc BLOB,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            CREATE TABLE artifact_processing (
                artifact_id TEXT PRIMARY KEY,
                engine TEXT,
                model TEXT,
                language TEXT,
                whisper_options_json_enc BLOB,
                diarization_settings_json_enc BLOB,
                ai_provider_snapshot_json_enc BLOB,
                parent_artifact_id TEXT,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );

            CREATE TABLE artifact_analysis (
                artifact_id TEXT PRIMARY KEY,
                timeline_v2_json_enc BLOB,
                emotion_analysis_json_enc BLOB,
                emotion_generated_at TEXT,
                FOREIGN KEY(artifact_id) REFERENCES transcript_artifacts(id) ON DELETE CASCADE
            );
            "#,
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to create migrated schema: {e}"))
        })?;

        for legacy_artifact in legacy {
            let mut metadata: BTreeMap<String, String> =
                serde_json::from_str(&legacy_artifact.metadata_json).unwrap_or_default();

            let mut artifact = TranscriptArtifact::new(
                legacy_artifact.job_id.clone(),
                if legacy_artifact.title.trim().is_empty() {
                    Path::new(&legacy_artifact.input_path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&legacy_artifact.input_path)
                        .to_string()
                } else {
                    legacy_artifact.title.clone()
                },
                legacy_artifact.kind.clone(),
                Path::new(&legacy_artifact.input_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&legacy_artifact.input_path)
                    .to_string(),
                ArtifactSourceOrigin::LegacyExternal,
                legacy_artifact.raw_transcript.clone(),
                legacy_artifact.optimized_transcript.clone(),
                legacy_artifact.summary.clone(),
                legacy_artifact.faqs.clone(),
                BTreeMap::new(),
            )
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to rebuild legacy artifact: {e}"))
            })?;
            artifact.id = legacy_artifact.id.clone();
            artifact.created_at = chrono::DateTime::parse_from_rfc3339(&legacy_artifact.created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            artifact.updated_at = chrono::DateTime::parse_from_rfc3339(&legacy_artifact.updated_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(artifact.created_at);
            artifact.source_external_path = Some(legacy_artifact.input_path.clone());
            artifact.parent_artifact_id = metadata.remove("parent_id");
            artifact.processing_model = metadata.get("model").cloned();
            artifact.processing_language = metadata.get("language").cloned();
            artifact.processing_engine = Some(match legacy_artifact.kind {
                ArtifactKind::Realtime => "whisper_stream".to_string(),
                ArtifactKind::File => "whisper_cpp".to_string(),
            });
            artifact.metadata = metadata;
            let audio = if Path::new(&legacy_artifact.input_path).exists() {
                self.import_audio_file(&artifact.id, Path::new(&legacy_artifact.input_path), true)?
            } else {
                AudioImportResult {
                    available: false,
                    backfill_status: ArtifactAudioBackfillStatus::Missing,
                    encrypted_rel_path: None,
                    extension: None,
                    mime_type: None,
                    sha256: None,
                    byte_size: None,
                }
            };
            self.insert_artifact_tx(
                &tx,
                &artifact,
                &audio,
                legacy_artifact.is_deleted,
                legacy_artifact.deleted_at.as_deref(),
            )?;
        }

        tx.execute_batch("DROP TABLE transcript_artifacts_legacy_backup;")
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to drop legacy backup table: {e}"))
            })?;
        tx.commit().map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to commit legacy migration transaction: {e}"
            ))
        })?;
        Ok(())
    }

    fn encrypted_label(artifact_id: &str, field: &str) -> String {
        format!("artifact:{artifact_id}:{field}")
    }

    fn encrypt_text(
        &self,
        artifact_id: &str,
        field: &str,
        value: &str,
    ) -> Result<Vec<u8>, ApplicationError> {
        self.secure_storage
            .encrypt_bytes(&Self::encrypted_label(artifact_id, field), value.as_bytes())
    }

    fn decrypt_text(
        &self,
        artifact_id: &str,
        field: &str,
        value: &[u8],
    ) -> Result<String, ApplicationError> {
        let bytes = self
            .secure_storage
            .decrypt_bytes(&Self::encrypted_label(artifact_id, field), value)?;
        String::from_utf8(bytes).map_err(|e| {
            ApplicationError::Persistence(format!("failed to decode decrypted UTF-8 text: {e}"))
        })
    }

    fn personalization_encrypted_label(entry_id: &str, field: &str) -> String {
        format!("personalization:{entry_id}:{field}")
    }

    fn encrypt_personalization_text(
        &self,
        entry_id: &str,
        field: &str,
        value: &str,
    ) -> Result<Vec<u8>, ApplicationError> {
        self.secure_storage.encrypt_bytes(
            &Self::personalization_encrypted_label(entry_id, field),
            value.as_bytes(),
        )
    }

    fn decrypt_personalization_text(
        &self,
        entry_id: &str,
        field: &str,
        value: &[u8],
    ) -> Result<String, ApplicationError> {
        let bytes = self.secure_storage.decrypt_bytes(
            &Self::personalization_encrypted_label(entry_id, field),
            value,
        )?;
        String::from_utf8(bytes).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to decode decrypted personalization UTF-8 text: {e}"
            ))
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_personalization_entry(
        &self,
        id: String,
        kind: String,
        source_text_enc: Vec<u8>,
        replacement_text_enc: Option<Vec<u8>>,
        language_code_enc: Option<Vec<u8>>,
        enabled: i64,
        hit_count: i64,
        created_at: String,
        updated_at: String,
    ) -> Result<PersonalizationEntry, ApplicationError> {
        let kind = PersonalizationEntryKind::parse_storage_value(&kind).ok_or_else(|| {
            ApplicationError::Persistence(format!("unknown personalization entry kind `{kind}`"))
        })?;
        let hit_count = u64::try_from(hit_count).map_err(|_| {
            ApplicationError::Persistence(
                "personalization entry hit count cannot be negative".to_string(),
            )
        })?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|e| {
                ApplicationError::Persistence(format!(
                    "invalid personalization created_at timestamp: {e}"
                ))
            })?;
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|e| {
                ApplicationError::Persistence(format!(
                    "invalid personalization updated_at timestamp: {e}"
                ))
            })?;

        Ok(PersonalizationEntry {
            source_text: self.decrypt_personalization_text(&id, "source_text", &source_text_enc)?,
            replacement_text: replacement_text_enc
                .as_deref()
                .map(|value| self.decrypt_personalization_text(&id, "replacement_text", value))
                .transpose()?,
            language_code: language_code_enc
                .as_deref()
                .map(|value| self.decrypt_personalization_text(&id, "language_code", value))
                .transpose()?,
            id,
            kind,
            enabled: enabled != 0,
            hit_count,
            created_at,
            updated_at,
        })
    }

    fn import_audio_file(
        &self,
        artifact_id: &str,
        source_path: &Path,
        legacy_backfill: bool,
    ) -> Result<AudioImportResult, ApplicationError> {
        let bytes = std::fs::read(source_path).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to read source audio {}: {e}",
                source_path.display()
            ))
        })?;

        let extension = source_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let mime_type = extension
            .as_deref()
            .map(mime_from_extension)
            .unwrap_or("audio/*")
            .to_string();
        let artifact_dir = self.vault_root.join(artifact_id);
        std::fs::create_dir_all(&artifact_dir).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create artifact vault directory {}: {e}",
                artifact_dir.display()
            ))
        })?;
        let encrypted_path = artifact_dir.join("source.enc");
        encrypt_to_file(
            &self.secure_storage,
            &format!("audio-vault:{artifact_id}"),
            &encrypted_path,
            &bytes,
        )?;

        let mut sha = Sha256::new();
        sha.update(&bytes);

        Ok(AudioImportResult {
            available: true,
            backfill_status: {
                let _ = legacy_backfill;
                ArtifactAudioBackfillStatus::Imported
            },
            encrypted_rel_path: Some(format!("{artifact_id}/source.enc")),
            extension,
            mime_type: Some(mime_type),
            sha256: Some(format!("{:x}", sha.finalize())),
            byte_size: Some(bytes.len() as u64),
        })
    }

    fn import_audio_bytes(
        &self,
        artifact_id: &str,
        bytes: &[u8],
        filename_hint: Option<&str>,
        backfill_status: ArtifactAudioBackfillStatus,
    ) -> Result<AudioImportResult, ApplicationError> {
        let extension = filename_hint
            .and_then(|value| Path::new(value).extension())
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        let mime_type = extension
            .as_deref()
            .map(mime_from_extension)
            .unwrap_or("audio/*")
            .to_string();
        let artifact_dir = self.vault_root.join(artifact_id);
        std::fs::create_dir_all(&artifact_dir).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to create artifact vault directory {}: {e}",
                artifact_dir.display()
            ))
        })?;
        let encrypted_path = artifact_dir.join("source.enc");
        encrypt_to_file(
            &self.secure_storage,
            &format!("audio-vault:{artifact_id}"),
            &encrypted_path,
            bytes,
        )?;

        let mut sha = Sha256::new();
        sha.update(bytes);

        Ok(AudioImportResult {
            available: true,
            backfill_status,
            encrypted_rel_path: Some(format!("{artifact_id}/source.enc")),
            extension,
            mime_type: Some(mime_type),
            sha256: Some(format!("{:x}", sha.finalize())),
            byte_size: Some(bytes.len() as u64),
        })
    }

    fn insert_artifact_tx(
        &self,
        tx: &Transaction<'_>,
        artifact: &TranscriptArtifact,
        audio: &AudioImportResult,
        is_deleted: bool,
        deleted_at: Option<&str>,
    ) -> Result<(), ApplicationError> {
        let metadata_json = serde_json::to_string(&artifact.metadata).map_err(|e| {
            ApplicationError::Persistence(format!("failed to serialize metadata JSON: {e}"))
        })?;
        let metadata_json_enc = self.encrypt_text(&artifact.id, "metadata_json", &metadata_json)?;
        let title_enc = self.encrypt_text(&artifact.id, "title", &artifact.title)?;
        let source_label_enc =
            self.encrypt_text(&artifact.id, "source_label", &artifact.source_label)?;
        let raw_transcript_enc =
            self.encrypt_text(&artifact.id, "raw_transcript", &artifact.raw_transcript)?;
        let optimized_transcript_enc = self.encrypt_text(
            &artifact.id,
            "optimized_transcript",
            &artifact.optimized_transcript,
        )?;
        let summary_enc = self.encrypt_text(&artifact.id, "summary", &artifact.summary)?;
        let faqs_enc = self.encrypt_text(&artifact.id, "faqs", &artifact.faqs)?;

        tx.execute(
            r#"
            INSERT INTO transcript_artifacts (
                id, job_id, kind, title_enc, source_label_enc, source_origin, raw_transcript_enc,
                optimized_transcript_enc, summary_enc, faqs_enc, metadata_json_enc, created_at,
                updated_at, revision, is_deleted, deleted_at, audio_available, audio_backfill_status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
            params![
                artifact.id,
                artifact.job_id,
                artifact.kind.as_str(),
                title_enc,
                source_label_enc,
                artifact.source_origin.as_str(),
                raw_transcript_enc,
                optimized_transcript_enc,
                summary_enc,
                faqs_enc,
                metadata_json_enc,
                artifact.created_at.to_rfc3339(),
                artifact.updated_at.to_rfc3339(),
                artifact.revision,
                if is_deleted { 1 } else { 0 },
                deleted_at,
                if audio.available { 1 } else { 0 },
                audio.backfill_status.as_str(),
            ],
        )
        .map_err(|e| ApplicationError::Persistence(format!("failed to insert artifact row: {e}")))?;

        let whisper_options_json_enc = artifact
            .whisper_options_json
            .as_deref()
            .map(|value| self.encrypt_text(&artifact.id, "whisper_options_json", value))
            .transpose()?;
        let diarization_settings_json_enc = artifact
            .diarization_settings_json
            .as_deref()
            .map(|value| self.encrypt_text(&artifact.id, "diarization_settings_json", value))
            .transpose()?;
        let ai_provider_snapshot_json_enc = artifact
            .ai_provider_snapshot_json
            .as_deref()
            .map(|value| self.encrypt_text(&artifact.id, "ai_provider_snapshot_json", value))
            .transpose()?;
        let source_fingerprint_json_enc = artifact
            .source_fingerprint_json
            .as_deref()
            .map(|value| self.encrypt_text(&artifact.id, "source_fingerprint_json", value))
            .transpose()?;
        tx.execute(
            r#"
            INSERT OR REPLACE INTO artifact_processing (
                artifact_id, engine, model, language, whisper_options_json_enc,
                diarization_settings_json_enc, ai_provider_snapshot_json_enc, parent_artifact_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                artifact.id,
                artifact.processing_engine,
                artifact.processing_model,
                artifact.processing_language,
                whisper_options_json_enc,
                diarization_settings_json_enc,
                ai_provider_snapshot_json_enc,
                artifact.parent_artifact_id,
            ],
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to insert processing row: {e}"))
        })?;

        let timeline_v2_json_enc = artifact
            .metadata
            .get("timeline_v2")
            .map(|value| self.encrypt_text(&artifact.id, "timeline_v2_json", value))
            .transpose()?;
        let emotion_analysis_json_enc = artifact
            .metadata
            .get(EMOTION_ANALYSIS_METADATA_KEY)
            .map(|value| self.encrypt_text(&artifact.id, "emotion_analysis_json", value))
            .transpose()?;
        tx.execute(
            r#"
            INSERT OR REPLACE INTO artifact_analysis (
                artifact_id, timeline_v2_json_enc, emotion_analysis_json_enc, emotion_generated_at
            ) VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                artifact.id,
                timeline_v2_json_enc,
                emotion_analysis_json_enc,
                artifact
                    .metadata
                    .get(EMOTION_ANALYSIS_GENERATED_AT_METADATA_KEY),
            ],
        )
        .map_err(|e| {
            ApplicationError::Persistence(format!("failed to insert analysis row: {e}"))
        })?;

        tx.execute(
            r#"
            INSERT OR REPLACE INTO artifact_audio (
                artifact_id, encrypted_rel_path, mime_type, file_extension, sha256, byte_size,
                duration_seconds, imported_at, backfill_status, source_fingerprint_json_enc
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                artifact.id,
                audio.encrypted_rel_path,
                audio.mime_type,
                audio.extension,
                audio.sha256,
                audio.byte_size.map(|value| value as i64),
                artifact.audio_duration_seconds,
                Utc::now().to_rfc3339(),
                audio.backfill_status.as_str(),
                source_fingerprint_json_enc,
            ],
        )
        .map_err(|e| ApplicationError::Persistence(format!("failed to insert audio row: {e}")))?;

        Ok(())
    }

    fn row_to_artifact(&self, row: &ArtifactRow) -> Result<TranscriptArtifact, ApplicationError> {
        let title = self.decrypt_text(&row.id, "title", &row.title_enc)?;
        let source_label = self.decrypt_text(&row.id, "source_label", &row.source_label_enc)?;
        let raw_transcript =
            self.decrypt_text(&row.id, "raw_transcript", &row.raw_transcript_enc)?;
        let optimized_transcript = self.decrypt_text(
            &row.id,
            "optimized_transcript",
            &row.optimized_transcript_enc,
        )?;
        let summary = self.decrypt_text(&row.id, "summary", &row.summary_enc)?;
        let faqs = self.decrypt_text(&row.id, "faqs", &row.faqs_enc)?;
        let metadata_json = self.decrypt_text(&row.id, "metadata_json", &row.metadata_json_enc)?;
        let mut metadata: BTreeMap<String, String> =
            serde_json::from_str(&metadata_json).unwrap_or_default();

        if let Some(timeline_enc) = row.timeline_v2_json_enc.as_deref() {
            metadata.insert(
                "timeline_v2".to_string(),
                self.decrypt_text(&row.id, "timeline_v2_json", timeline_enc)?,
            );
        }
        if let Some(emotion_enc) = row.emotion_analysis_json_enc.as_deref() {
            metadata.insert(
                EMOTION_ANALYSIS_METADATA_KEY.to_string(),
                self.decrypt_text(&row.id, "emotion_analysis_json", emotion_enc)?,
            );
        }
        if let Some(generated_at) = row.emotion_generated_at.clone() {
            metadata.insert(
                EMOTION_ANALYSIS_GENERATED_AT_METADATA_KEY.to_string(),
                generated_at,
            );
        }
        if !optimized_transcript.trim().is_empty() {
            metadata.insert(
                HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY.to_string(),
                "true".to_string(),
            );
        }

        let created_at = chrono::DateTime::parse_from_rfc3339(&row.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let updated_at = chrono::DateTime::parse_from_rfc3339(&row.updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(created_at);

        Ok(TranscriptArtifact {
            id: row.id.clone(),
            job_id: row.job_id.clone(),
            title,
            kind: if row.kind.eq_ignore_ascii_case("realtime") {
                ArtifactKind::Realtime
            } else {
                ArtifactKind::File
            },
            source_label,
            source_origin: match row.source_origin.as_str() {
                "trimmed" => ArtifactSourceOrigin::Trimmed,
                "realtime" => ArtifactSourceOrigin::Realtime,
                "legacy_external" => ArtifactSourceOrigin::LegacyExternal,
                _ => ArtifactSourceOrigin::Imported,
            },
            audio_available: row.audio_available,
            audio_backfill_status: match row.audio_backfill_status.as_str() {
                "missing" => ArtifactAudioBackfillStatus::Missing,
                "pending_backfill" => ArtifactAudioBackfillStatus::PendingBackfill,
                _ => ArtifactAudioBackfillStatus::Imported,
            },
            revision: row.revision,
            raw_transcript,
            optimized_transcript,
            summary,
            faqs,
            metadata,
            parent_artifact_id: row.parent_artifact_id.clone(),
            processing_engine: row.engine.clone(),
            processing_model: row.model.clone(),
            processing_language: row.language.clone(),
            audio_duration_seconds: row.duration_seconds,
            audio_byte_size: row.byte_size,
            created_at,
            updated_at,
            source_external_path: None,
            whisper_options_json: row
                .whisper_options_json_enc
                .as_deref()
                .map(|value| self.decrypt_text(&row.id, "whisper_options_json", value))
                .transpose()?,
            diarization_settings_json: row
                .diarization_settings_json_enc
                .as_deref()
                .map(|value| self.decrypt_text(&row.id, "diarization_settings_json", value))
                .transpose()?,
            ai_provider_snapshot_json: row
                .ai_provider_snapshot_json_enc
                .as_deref()
                .map(|value| self.decrypt_text(&row.id, "ai_provider_snapshot_json", value))
                .transpose()?,
            source_fingerprint_json: row
                .source_fingerprint_json_enc
                .as_deref()
                .map(|value| self.decrypt_text(&row.id, "source_fingerprint_json", value))
                .transpose()?,
        })
    }

    fn fetch_artifacts(
        &self,
        conn: &Connection,
        is_deleted: bool,
        kind: Option<ArtifactKind>,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let mut sql = String::from(
            r#"
            SELECT a.id, a.job_id, a.kind, a.title_enc, a.source_label_enc, a.source_origin,
                   a.raw_transcript_enc, a.optimized_transcript_enc, a.summary_enc, a.faqs_enc,
                   a.metadata_json_enc, a.created_at, a.updated_at, a.revision, a.audio_available,
                   a.audio_backfill_status, a.deleted_at,
                   p.engine, p.model, p.language, p.parent_artifact_id,
                   audio.duration_seconds, audio.byte_size,
                   p.whisper_options_json_enc, p.diarization_settings_json_enc,
                   p.ai_provider_snapshot_json_enc, audio.source_fingerprint_json_enc,
                   analysis.timeline_v2_json_enc, analysis.emotion_analysis_json_enc,
                   analysis.emotion_generated_at
            FROM transcript_artifacts a
            LEFT JOIN artifact_processing p ON p.artifact_id = a.id
            LEFT JOIN artifact_audio audio ON audio.artifact_id = a.id
            LEFT JOIN artifact_analysis analysis ON analysis.artifact_id = a.id
            WHERE a.is_deleted = ?1
            "#,
        );
        let mut params_values: Vec<Value> = vec![Value::Integer(if is_deleted { 1 } else { 0 })];
        if let Some(kind) = kind {
            sql.push_str(" AND a.kind = ?2");
            params_values.push(Value::Text(kind.as_str().to_string()));
        }
        sql.push_str(" ORDER BY COALESCE(a.deleted_at, a.created_at) DESC, a.updated_at DESC");

        let mut stmt = conn.prepare(&sql).map_err(|e| {
            ApplicationError::Persistence(format!("failed to prepare artifact query: {e}"))
        })?;
        let rows = stmt
            .query_map(params_from_iter(params_values.iter()), Self::row_from_query)
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to query artifacts: {e}"))
            })?;

        rows.map(|row| {
            row.map_err(|e| {
                ApplicationError::Persistence(format!("failed to parse artifact row: {e}"))
            })
            .and_then(|parsed| self.row_to_artifact(&parsed))
        })
        .collect()
    }

    fn row_from_query(row: &Row<'_>) -> Result<ArtifactRow, rusqlite::Error> {
        Ok(ArtifactRow {
            id: row.get(0)?,
            job_id: row.get(1)?,
            kind: row.get(2)?,
            title_enc: row.get(3)?,
            source_label_enc: row.get(4)?,
            source_origin: row.get(5)?,
            raw_transcript_enc: row.get(6)?,
            optimized_transcript_enc: row.get(7)?,
            summary_enc: row.get(8)?,
            faqs_enc: row.get(9)?,
            metadata_json_enc: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            revision: row.get(13)?,
            audio_available: row.get::<_, i64>(14)? != 0,
            audio_backfill_status: row.get(15)?,
            engine: row.get(17)?,
            model: row.get(18)?,
            language: row.get(19)?,
            parent_artifact_id: row.get(20)?,
            duration_seconds: row.get(21)?,
            byte_size: row.get::<_, Option<i64>>(22)?.map(|value| value as u64),
            whisper_options_json_enc: row.get(23)?,
            diarization_settings_json_enc: row.get(24)?,
            ai_provider_snapshot_json_enc: row.get(25)?,
            source_fingerprint_json_enc: row.get(26)?,
            timeline_v2_json_enc: row.get(27)?,
            emotion_analysis_json_enc: row.get(28)?,
            emotion_generated_at: row.get(29)?,
        })
    }

    fn load_one(
        &self,
        conn: &Connection,
        id: &str,
        is_deleted: bool,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let artifacts = self.fetch_artifacts(conn, is_deleted, None)?;
        Ok(artifacts.into_iter().find(|artifact| artifact.id == id))
    }

    pub fn import_backup_artifact(
        &self,
        artifact: &TranscriptArtifact,
        audio_bytes: Option<&[u8]>,
        audio_filename_hint: Option<&str>,
        is_deleted: bool,
        deleted_at: Option<&str>,
    ) -> Result<(), ApplicationError> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction().map_err(|e| {
            ApplicationError::Persistence(format!("failed to start backup import transaction: {e}"))
        })?;

        let audio = match audio_bytes {
            Some(bytes) => self.import_audio_bytes(
                &artifact.id,
                bytes,
                audio_filename_hint,
                artifact.audio_backfill_status.clone(),
            )?,
            None => AudioImportResult {
                available: false,
                backfill_status: artifact.audio_backfill_status.clone(),
                encrypted_rel_path: None,
                extension: None,
                mime_type: None,
                sha256: None,
                byte_size: None,
            },
        };

        self.insert_artifact_tx(&tx, artifact, &audio, is_deleted, deleted_at)?;
        tx.commit().map_err(|e| {
            ApplicationError::Persistence(format!("failed to commit backup artifact import: {e}"))
        })?;
        Ok(())
    }
}

#[async_trait]
impl ArtifactRepository for SqliteArtifactRepository {
    async fn save(&self, artifact: &TranscriptArtifact) -> Result<(), ApplicationError> {
        let repo = self.clone();
        let artifact = artifact.clone();

        tokio::task::spawn_blocking(move || {
            let mut conn = repo.open_connection()?;
            let tx = conn.transaction().map_err(|e| {
                ApplicationError::Persistence(format!("failed to start save transaction: {e}"))
            })?;

            let audio = match artifact.source_external_path.as_deref() {
                Some(path) if Path::new(path).exists() => {
                    repo.import_audio_file(&artifact.id, Path::new(path), false)?
                }
                Some(_) => AudioImportResult {
                    available: false,
                    backfill_status: ArtifactAudioBackfillStatus::Missing,
                    encrypted_rel_path: None,
                    extension: None,
                    mime_type: None,
                    sha256: None,
                    byte_size: None,
                },
                None => AudioImportResult {
                    available: false,
                    backfill_status: ArtifactAudioBackfillStatus::Missing,
                    encrypted_rel_path: None,
                    extension: None,
                    mime_type: None,
                    sha256: None,
                    byte_size: None,
                },
            };

            repo.insert_artifact_tx(&tx, &artifact, &audio, false, None)?;
            tx.commit().map_err(|e| {
                ApplicationError::Persistence(format!("failed to commit artifact save: {e}"))
            })?;
            Ok(())
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn list_recent(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        self.list_filtered(None, None, limit, offset).await
    }

    async fn list_filtered(
        &self,
        kind: Option<ArtifactKind>,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let query = query
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let mut artifacts = repo.fetch_artifacts(&conn, false, kind)?;
            if let Some(query) = query {
                artifacts.retain(|artifact| {
                    artifact.title.to_ascii_lowercase().contains(&query)
                        || artifact.source_label.to_ascii_lowercase().contains(&query)
                        || artifact
                            .raw_transcript
                            .to_ascii_lowercase()
                            .contains(&query)
                        || artifact
                            .optimized_transcript
                            .to_ascii_lowercase()
                            .contains(&query)
                });
            }
            Ok(artifacts.into_iter().skip(offset).take(limit).collect())
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            repo.load_one(&conn, &id, false)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn update_content(
        &self,
        id: &str,
        optimized_transcript: &str,
        summary: &str,
        faqs: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        let optimized_transcript = optimized_transcript.to_string();
        let summary = summary.to_string();
        let faqs = faqs.to_string();

        tokio::task::spawn_blocking(move || {
            let mut conn = repo.open_connection()?;
            let Some(current) = repo.load_one(&conn, &id, false)? else {
                return Ok(None);
            };

            let mut metadata = current.metadata.clone();
            if optimized_transcript.trim().is_empty() {
                metadata.remove(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY);
            } else {
                metadata.insert(
                    HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY.to_string(),
                    "true".to_string(),
                );
            }
            let metadata_json = serde_json::to_string(&metadata).map_err(|e| {
                ApplicationError::Persistence(format!("failed to serialize metadata: {e}"))
            })?;

            let tx = conn.transaction().map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to start content update transaction: {e}"
                ))
            })?;
            let changed = tx
                .execute(
                    r#"
                UPDATE transcript_artifacts
                SET optimized_transcript_enc = ?2,
                    summary_enc = ?3,
                    faqs_enc = ?4,
                    metadata_json_enc = ?5,
                    updated_at = ?6,
                    revision = revision + 1
                WHERE id = ?1 AND is_deleted = 0 AND revision = ?7
                "#,
                    params![
                        id,
                        repo.encrypt_text(
                            &current.id,
                            "optimized_transcript",
                            &optimized_transcript
                        )?,
                        repo.encrypt_text(&current.id, "summary", &summary)?,
                        repo.encrypt_text(&current.id, "faqs", &faqs)?,
                        repo.encrypt_text(&current.id, "metadata_json", &metadata_json)?,
                        Utc::now().to_rfc3339(),
                        current.revision,
                    ],
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to update artifact content: {e}"))
                })?;

            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "artifact update rejected because a newer revision already exists".to_string(),
                ));
            }
            tx.commit().map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to commit content update transaction: {e}"
                ))
            })?;

            repo.load_one(&repo.open_connection()?, &id, false)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn update_metadata_entry(
        &self,
        id: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        let key = key.to_string();
        let value = value.map(str::to_string);

        tokio::task::spawn_blocking(move || {
            let mut conn = repo.open_connection()?;
            let Some(current) = repo.load_one(&conn, &id, false)? else {
                return Ok(None);
            };

            let mut metadata = current.metadata.clone();
            match value.as_deref() {
                Some(next_value) => {
                    metadata.insert(key.clone(), next_value.to_string());
                }
                None => {
                    metadata.remove(&key);
                }
            }
            let metadata_json = serde_json::to_string(&metadata).map_err(|e| {
                ApplicationError::Persistence(format!("failed to serialize metadata: {e}"))
            })?;

            let tx = conn.transaction().map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to start metadata update transaction: {e}"
                ))
            })?;
            let changed = tx
                .execute(
                    r#"
                UPDATE transcript_artifacts
                SET metadata_json_enc = ?2,
                    updated_at = ?3,
                    revision = revision + 1
                WHERE id = ?1 AND is_deleted = 0 AND revision = ?4
                "#,
                    params![
                        id,
                        repo.encrypt_text(&current.id, "metadata_json", &metadata_json)?,
                        Utc::now().to_rfc3339(),
                        current.revision,
                    ],
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to update artifact metadata: {e}"
                    ))
                })?;

            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "artifact metadata update rejected because a newer revision already exists"
                        .to_string(),
                ));
            }
            tx.commit().map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to commit metadata update transaction: {e}"
                ))
            })?;

            repo.load_one(&repo.open_connection()?, &id, false)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn apply_artifact_review_update(
        &self,
        id: &str,
        expected_revision: i64,
        optimized_transcript: Option<&str>,
        review_metadata_json: &str,
        remembered_correction: Option<&PersonalizationEntry>,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        let optimized_transcript = optimized_transcript.map(str::to_string);
        let review_metadata_json = review_metadata_json.to_string();
        let remembered_correction = remembered_correction.cloned();

        tokio::task::spawn_blocking(move || {
            let mut conn = repo.open_connection()?;
            let Some(current) = repo.load_one(&conn, &id, false)? else {
                return Ok(None);
            };
            if current.revision != expected_revision {
                return Err(ApplicationError::Persistence(
                    "artifact review rejected because a newer revision already exists".to_string(),
                ));
            }
            let mut metadata = current.metadata.clone();
            metadata.insert(
                "personalization_review_v1".to_string(),
                review_metadata_json,
            );
            let next_optimized = optimized_transcript
                .as_deref()
                .unwrap_or(&current.optimized_transcript);
            if next_optimized.trim().is_empty() {
                metadata.remove(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY);
            } else {
                metadata.insert(
                    HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY.to_string(),
                    "true".to_string(),
                );
            }
            let metadata_json = serde_json::to_string(&metadata).map_err(|error| {
                ApplicationError::Persistence(format!(
                    "failed to serialize artifact review metadata: {error}"
                ))
            })?;
            let tx = conn.transaction().map_err(|error| {
                ApplicationError::Persistence(format!(
                    "failed to start artifact review transaction: {error}"
                ))
            })?;
            let changed = tx
                .execute(
                    r#"
                    UPDATE transcript_artifacts
                    SET optimized_transcript_enc = ?2,
                        metadata_json_enc = ?3,
                        updated_at = ?4,
                        revision = revision + 1
                    WHERE id = ?1 AND is_deleted = 0 AND revision = ?5
                    "#,
                    params![
                        id,
                        repo.encrypt_text(&current.id, "optimized_transcript", next_optimized)?,
                        repo.encrypt_text(&current.id, "metadata_json", &metadata_json)?,
                        Utc::now().to_rfc3339(),
                        expected_revision,
                    ],
                )
                .map_err(|error| {
                    ApplicationError::Persistence(format!(
                        "failed to apply artifact review: {error}"
                    ))
                })?;
            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "artifact review rejected because a newer revision already exists".to_string(),
                ));
            }
            if let Some(correction) = remembered_correction {
                let source_text_enc = repo.encrypt_personalization_text(
                    &correction.id,
                    "source_text",
                    &correction.source_text,
                )?;
                let replacement_text_enc = correction
                    .replacement_text
                    .as_deref()
                    .map(|value| {
                        repo.encrypt_personalization_text(&correction.id, "replacement_text", value)
                    })
                    .transpose()?;
                let language_code_enc = correction
                    .language_code
                    .as_deref()
                    .map(|value| {
                        repo.encrypt_personalization_text(&correction.id, "language_code", value)
                    })
                    .transpose()?;
                let hit_count = i64::try_from(correction.hit_count).map_err(|_| {
                    ApplicationError::Validation(
                        "personalization entry hit count exceeds SQLite integer range".to_string(),
                    )
                })?;
                tx.execute(
                    r#"
                    INSERT INTO personalization_entries (
                        id, kind, source_text_enc, replacement_text_enc, language_code_enc,
                        enabled, hit_count, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    "#,
                    params![
                        correction.id,
                        correction.kind.as_str(),
                        source_text_enc,
                        replacement_text_enc,
                        language_code_enc,
                        if correction.enabled { 1_i64 } else { 0_i64 },
                        hit_count,
                        correction.created_at.to_rfc3339(),
                        correction.updated_at.to_rfc3339(),
                    ],
                )
                .map_err(|error| {
                    ApplicationError::Persistence(format!(
                        "failed to remember reviewed correction: {error}"
                    ))
                })?;
            }
            tx.commit().map_err(|error| {
                ApplicationError::Persistence(format!("failed to commit artifact review: {error}"))
            })?;
            repo.load_one(&repo.open_connection()?, &id, false)
        })
        .await
        .map_err(|error| {
            ApplicationError::Persistence(format!("storage task join error: {error}"))
        })?
    }

    async fn update_timeline_v2(
        &self,
        id: &str,
        timeline_v2_json: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        let timeline_v2_json = timeline_v2_json.trim().to_string();
        tokio::task::spawn_blocking(move || {
            let _: serde_json::Value = serde_json::from_str(&timeline_v2_json).map_err(|e| {
                ApplicationError::Validation(format!("invalid timeline_v2 JSON: {e}"))
            })?;
            let mut conn = repo.open_connection()?;
            let Some(current) = repo.load_one(&conn, &id, false)? else {
                return Ok(None);
            };

            let tx = conn.transaction().map_err(|e| {
                ApplicationError::Persistence(format!("failed to start timeline update transaction: {e}"))
            })?;
            let changed = tx.execute(
                "UPDATE transcript_artifacts SET updated_at = ?2, revision = revision + 1 WHERE id = ?1 AND is_deleted = 0 AND revision = ?3",
                params![id, Utc::now().to_rfc3339(), current.revision],
            ).map_err(|e| ApplicationError::Persistence(format!("failed to bump artifact revision for timeline update: {e}")))?;
            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "artifact timeline update rejected because a newer revision already exists".to_string(),
                ));
            }
            tx.execute(
                "INSERT INTO artifact_analysis (artifact_id, timeline_v2_json_enc) VALUES (?1, ?2) ON CONFLICT(artifact_id) DO UPDATE SET timeline_v2_json_enc = excluded.timeline_v2_json_enc",
                params![current.id, repo.encrypt_text(&current.id, "timeline_v2_json", &timeline_v2_json)?],
            ).map_err(|e| ApplicationError::Persistence(format!("failed to update timeline analysis row: {e}")))?;
            tx.commit().map_err(|e| ApplicationError::Persistence(format!("failed to commit timeline update transaction: {e}")))?;
            repo.load_one(&repo.open_connection()?, &id, false)
        }).await.map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn update_emotion_analysis(
        &self,
        id: &str,
        emotion_analysis_json: &str,
        generated_at: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        let emotion_analysis_json = emotion_analysis_json.trim().to_string();
        let generated_at = generated_at.trim().to_string();
        tokio::task::spawn_blocking(move || {
            let _: serde_json::Value =
                serde_json::from_str(&emotion_analysis_json).map_err(|e| {
                    ApplicationError::Validation(format!("invalid emotion_analysis_v1 JSON: {e}"))
                })?;
            let mut conn = repo.open_connection()?;
            let Some(current) = repo.load_one(&conn, &id, false)? else {
                return Ok(None);
            };
            let tx = conn.transaction().map_err(|e| {
                ApplicationError::Persistence(format!("failed to start emotion update transaction: {e}"))
            })?;
            let changed = tx.execute(
                "UPDATE transcript_artifacts SET updated_at = ?2, revision = revision + 1 WHERE id = ?1 AND is_deleted = 0 AND revision = ?3",
                params![id, Utc::now().to_rfc3339(), current.revision],
            ).map_err(|e| ApplicationError::Persistence(format!("failed to bump artifact revision for emotion update: {e}")))?;
            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "artifact emotion update rejected because a newer revision already exists".to_string(),
                ));
            }
            tx.execute(
                r#"
                INSERT INTO artifact_analysis (artifact_id, emotion_analysis_json_enc, emotion_generated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(artifact_id) DO UPDATE
                SET emotion_analysis_json_enc = excluded.emotion_analysis_json_enc,
                    emotion_generated_at = excluded.emotion_generated_at
                "#,
                params![
                    current.id,
                    repo.encrypt_text(&current.id, "emotion_analysis_json", &emotion_analysis_json)?,
                    generated_at,
                ],
            ).map_err(|e| ApplicationError::Persistence(format!("failed to update emotion analysis row: {e}")))?;
            tx.commit().map_err(|e| ApplicationError::Persistence(format!("failed to commit emotion update transaction: {e}")))?;
            repo.load_one(&repo.open_connection()?, &id, false)
        }).await.map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn rename(
        &self,
        id: &str,
        new_title: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        let new_title = new_title.trim().to_string();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let Some(current) = repo.load_one(&conn, &id, false)? else {
                return Ok(None);
            };
            let changed = conn
                .execute(
                    r#"
                UPDATE transcript_artifacts
                SET title_enc = ?2, updated_at = ?3, revision = revision + 1
                WHERE id = ?1 AND is_deleted = 0 AND revision = ?4
                "#,
                    params![
                        id,
                        repo.encrypt_text(&current.id, "title", &new_title)?,
                        Utc::now().to_rfc3339(),
                        current.revision,
                    ],
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to rename artifact: {e}"))
                })?;
            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "artifact rename rejected because a newer revision already exists".to_string(),
                ));
            }
            repo.load_one(&repo.open_connection()?, &id, false)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn list_deleted(
        &self,
        kind: Option<ArtifactKind>,
        query: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TranscriptArtifact>, ApplicationError> {
        let repo = self.clone();
        let query = query
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let mut artifacts = repo.fetch_artifacts(&conn, true, kind)?;
            if let Some(query) = query {
                artifacts.retain(|artifact| {
                    artifact.title.to_ascii_lowercase().contains(&query)
                        || artifact.source_label.to_ascii_lowercase().contains(&query)
                        || artifact
                            .raw_transcript
                            .to_ascii_lowercase()
                            .contains(&query)
                        || artifact
                            .optimized_transcript
                            .to_ascii_lowercase()
                            .contains(&query)
                });
            }
            Ok(artifacts.into_iter().skip(offset).take(limit).collect())
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn restore_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let repo = self.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "UPDATE transcript_artifacts SET is_deleted = 0, deleted_at = NULL, updated_at = ? WHERE id IN ({placeholders}) AND is_deleted = 1"
            );
            let mut values: Vec<Value> = vec![Value::Text(Utc::now().to_rfc3339())];
            values.extend(ids.into_iter().map(Value::Text));
            conn.execute(&query, params_from_iter(values.iter()))
                .map_err(|e| ApplicationError::Persistence(format!("failed to restore artifacts: {e}")))
        }).await.map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn hard_delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let repo = self.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            for id in &ids {
                let artifact_dir = repo.vault_root.join(id);
                if artifact_dir.exists() {
                    let _ = std::fs::remove_dir_all(&artifact_dir);
                }
            }
            let conn = repo.open_connection()?;
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "DELETE FROM transcript_artifacts WHERE id IN ({placeholders}) AND is_deleted = 1"
            );
            let values: Vec<Value> = ids.into_iter().map(Value::Text).collect();
            conn.execute(&query, params_from_iter(values.iter()))
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to hard-delete artifacts: {e}"))
                })
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn purge_deleted_older_than_days(&self, days: u32) -> Result<usize, ApplicationError> {
        let repo = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let cutoff = (Utc::now() - Duration::days(i64::from(days))).to_rfc3339();
            conn.execute(
                "DELETE FROM transcript_artifacts WHERE is_deleted = 1 AND deleted_at IS NOT NULL AND deleted_at < ?1",
                params![cutoff],
            )
            .map_err(|e| ApplicationError::Persistence(format!("failed to purge deleted artifacts: {e}")))
        }).await.map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let repo = self.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "UPDATE transcript_artifacts SET is_deleted = 1, deleted_at = ?, updated_at = ? WHERE id IN ({placeholders}) AND is_deleted = 0"
            );
            let now = Utc::now().to_rfc3339();
            let mut values: Vec<Value> = vec![Value::Text(now.clone()), Value::Text(now)];
            values.extend(ids.into_iter().map(Value::Text));
            conn.execute(&query, params_from_iter(values.iter()))
                .map_err(|e| ApplicationError::Persistence(format!("failed to delete artifacts: {e}")))
        }).await.map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn read_audio_bytes(&self, id: &str) -> Result<Option<Vec<u8>>, ApplicationError> {
        let repo = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let rel_path: Option<String> = conn
                .query_row(
                    "SELECT encrypted_rel_path FROM artifact_audio WHERE artifact_id = ?1 LIMIT 1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to query artifact audio path: {e}"
                    ))
                })?;
            let Some(rel_path) = rel_path else {
                return Ok(None);
            };
            let full_path = repo.vault_root.join(rel_path);
            let bytes = decrypt_from_file(
                &repo.secure_storage,
                &format!("audio-vault:{id}"),
                &full_path,
            )?;
            Ok(Some(bytes))
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn append_chat_message(
        &self,
        message: &ArtifactChatMessage,
    ) -> Result<(), ApplicationError> {
        let repo = self.clone();
        let message = message.clone();
        tokio::task::spawn_blocking(move || repo.import_backup_chat_message(&message))
            .await
            .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn list_chat_messages(
        &self,
        artifact_id: &str,
    ) -> Result<Vec<ArtifactChatMessage>, ApplicationError> {
        let repo = self.clone();
        let artifact_id = artifact_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT id, role, text_enc, origin, status, provider, model, created_at
                    FROM artifact_chat_messages
                    WHERE artifact_id = ?1
                    ORDER BY created_at ASC, id ASC
                    "#,
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to prepare chat query: {e}"))
                })?;
            let rows = stmt
                .query_map(params![artifact_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to query artifact chat: {e}"))
                })?;

            rows.map(|row| {
                let (id, role, text_enc, origin, status, provider, model, created_at) = row
                    .map_err(|e| {
                        ApplicationError::Persistence(format!("failed to read chat row: {e}"))
                    })?;
                let field = format!("chat_message:{id}");
                let text = repo.decrypt_text(&artifact_id, &field, &text_enc)?;
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map(|value| value.with_timezone(&Utc))
                    .map_err(|e| {
                        ApplicationError::Persistence(format!("invalid chat timestamp: {e}"))
                    })?;
                Ok(ArtifactChatMessage {
                    id,
                    artifact_id: artifact_id.clone(),
                    role,
                    text,
                    origin,
                    status,
                    provider,
                    model,
                    created_at,
                })
            })
            .collect()
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn save_chat_summary(
        &self,
        artifact_id: &str,
        summary: &str,
    ) -> Result<(), ApplicationError> {
        let repo = self.clone();
        let artifact_id = artifact_id.to_string();
        let summary = summary.to_string();
        tokio::task::spawn_blocking(move || repo.import_backup_chat_summary(&artifact_id, &summary))
            .await
            .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn load_chat_summary(
        &self,
        artifact_id: &str,
    ) -> Result<Option<String>, ApplicationError> {
        let repo = self.clone();
        let artifact_id = artifact_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let encrypted: Option<Vec<u8>> = conn
                .query_row(
                    "SELECT summary_enc FROM artifact_chat_context WHERE artifact_id = ?1",
                    params![artifact_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to load chat summary: {e}"))
                })?;
            encrypted
                .map(|value| repo.decrypt_text(&artifact_id, "chat_summary", &value))
                .transpose()
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn list_personalization_entries(
        &self,
    ) -> Result<Vec<PersonalizationEntry>, ApplicationError> {
        let repo = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT id, kind, source_text_enc, replacement_text_enc,
                           language_code_enc, enabled, hit_count, created_at, updated_at
                    FROM personalization_entries
                    ORDER BY updated_at DESC, id ASC
                    "#,
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to prepare personalization query: {e}"
                    ))
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                })
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to query personalization entries: {e}"
                    ))
                })?;

            rows.map(|row| {
                let (
                    id,
                    kind,
                    source_text_enc,
                    replacement_text_enc,
                    language_code_enc,
                    enabled,
                    hit_count,
                    created_at,
                    updated_at,
                ) = row.map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to read personalization entry: {e}"
                    ))
                })?;
                repo.decode_personalization_entry(
                    id,
                    kind,
                    source_text_enc,
                    replacement_text_enc,
                    language_code_enc,
                    enabled,
                    hit_count,
                    created_at,
                    updated_at,
                )
            })
            .collect()
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn upsert_personalization_entry(
        &self,
        entry: &PersonalizationEntry,
    ) -> Result<(), ApplicationError> {
        let repo = self.clone();
        let entry = entry.clone();
        tokio::task::spawn_blocking(move || repo.upsert_personalization_entry_sync(&entry))
            .await
            .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn delete_personalization_entry(&self, id: &str) -> Result<usize, ApplicationError> {
        let id = id.trim().to_string();
        if id.is_empty() {
            return Ok(0);
        }
        let repo = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            conn.execute(
                "DELETE FROM personalization_entries WHERE id = ?1",
                params![id],
            )
            .map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to delete personalization entry: {e}"
                ))
            })
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn clear_personalization_entries(&self) -> Result<usize, ApplicationError> {
        let repo = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            conn.execute("DELETE FROM personalization_entries", [])
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to clear personalization entries: {e}"
                    ))
                })
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn increment_personalization_hit_count(&self, id: &str) -> Result<(), ApplicationError> {
        let repo = self.clone();
        let id = id.trim().to_string();
        tokio::task::spawn_blocking(move || {
            let conn = repo.open_connection()?;
            let changed = conn
                .execute(
                    "UPDATE personalization_entries SET hit_count = hit_count + 1, updated_at = ?2 WHERE id = ?1",
                    params![id, Utc::now().to_rfc3339()],
                )
                .map_err(|error| {
                    ApplicationError::Persistence(format!(
                        "failed to increment personalization hit count: {error}"
                    ))
                })?;
            if changed == 0 {
                return Err(ApplicationError::Persistence(
                    "personalization entry disappeared before its hit count could be updated"
                        .to_string(),
                ));
            }
            Ok(())
        })
        .await
        .map_err(|error| {
            ApplicationError::Persistence(format!("storage task join error: {error}"))
        })?
    }
}

fn mime_from_extension(extension: &str) -> &'static str {
    match extension {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        _ => "audio/*",
    }
}
