use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension, Row};

use sbobino_application::{ApplicationError, ArtifactRepository};
use sbobino_domain::{ArtifactKind, TranscriptArtifact};

const HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY: &str = "has_optimized_transcript";

#[derive(Debug, Clone)]
pub struct SqliteArtifactRepository {
    db_path: PathBuf,
}

impl SqliteArtifactRepository {
    pub fn new(db_path: PathBuf) -> Result<Self, ApplicationError> {
        let repo = Self { db_path };
        repo.init_schema()?;
        Ok(repo)
    }

    fn init_schema(&self) -> Result<(), ApplicationError> {
        let conn = Connection::open(&self.db_path).map_err(|e| {
            ApplicationError::Persistence(format!(
                "failed to open sqlite database {}: {e}",
                self.db_path.display()
            ))
        })?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS transcript_artifacts (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                title TEXT NOT NULL,
                kind TEXT NOT NULL,
                input_path TEXT NOT NULL,
                raw_transcript TEXT NOT NULL,
                optimized_transcript TEXT NOT NULL,
                summary TEXT NOT NULL,
                faqs TEXT NOT NULL,
                metadata_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                deleted_at TEXT
            );
            "#,
        )
        .map_err(|e| ApplicationError::Persistence(format!("failed to initialize schema: {e}")))?;

        // Forward-compatible migration from older schema revisions.
        self.ensure_column(&conn, "title", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column(&conn, "kind", "TEXT NOT NULL DEFAULT 'file'")?;
        self.ensure_column(&conn, "updated_at", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column(&conn, "is_deleted", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column(&conn, "deleted_at", "TEXT")?;

        // Create indexes only after migrations complete, otherwise legacy databases
        // without `kind` would fail during bootstrap before `ensure_column` runs.
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_created_at
            ON transcript_artifacts(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_kind
            ON transcript_artifacts(kind);
            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_is_deleted
            ON transcript_artifacts(is_deleted);
            CREATE INDEX IF NOT EXISTS idx_transcript_artifacts_deleted_at
            ON transcript_artifacts(deleted_at DESC);
            "#,
        )
        .map_err(|e| ApplicationError::Persistence(format!("failed to initialize indexes: {e}")))?;

        Ok(())
    }

    fn ensure_column(
        &self,
        conn: &Connection,
        name: &str,
        definition: &str,
    ) -> Result<(), ApplicationError> {
        let mut stmt = conn
            .prepare("PRAGMA table_info(transcript_artifacts)")
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to query table_info: {e}"))
            })?;

        let mut rows = stmt
            .query([])
            .map_err(|e| ApplicationError::Persistence(format!("failed to query schema: {e}")))?;

        while let Some(row) = rows
            .next()
            .map_err(|e| ApplicationError::Persistence(format!("failed to read schema row: {e}")))?
        {
            let column_name: String = row.get(1).map_err(|e| {
                ApplicationError::Persistence(format!("failed to parse schema row: {e}"))
            })?;
            if column_name == name {
                return Ok(());
            }
        }

        let alter = format!("ALTER TABLE transcript_artifacts ADD COLUMN {name} {definition}");
        conn.execute(&alter, []).map_err(|e| {
            ApplicationError::Persistence(format!("failed to migrate schema with `{alter}`: {e}"))
        })?;

        Ok(())
    }

    fn row_to_artifact(row: &Row<'_>) -> Result<TranscriptArtifact, rusqlite::Error> {
        let metadata_json: String = row.get("metadata_json")?;
        let metadata = serde_json::from_str(&metadata_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let created_at_str: String = row.get("created_at")?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

        let updated_at_str: String = row.get("updated_at")?;
        let updated_at = if updated_at_str.trim().is_empty() {
            created_at
        } else {
            chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or(created_at)
        };

        let kind_raw: String = row.get("kind")?;
        let kind = if kind_raw.eq_ignore_ascii_case("realtime") {
            ArtifactKind::Realtime
        } else {
            ArtifactKind::File
        };

        let input_path: String = row.get("input_path")?;
        let title: String = row.get("title")?;

        Ok(TranscriptArtifact {
            id: row.get("id")?,
            job_id: row.get("job_id")?,
            title: if title.trim().is_empty() {
                Path::new(&input_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&input_path)
                    .to_string()
            } else {
                title
            },
            kind,
            input_path,
            raw_transcript: row.get("raw_transcript")?,
            optimized_transcript: row.get("optimized_transcript")?,
            summary: row.get("summary")?,
            faqs: row.get("faqs")?,
            metadata,
            created_at,
            updated_at,
        })
    }
}

#[async_trait]
impl ArtifactRepository for SqliteArtifactRepository {
    async fn save(&self, artifact: &TranscriptArtifact) -> Result<(), ApplicationError> {
        let db_path = self.db_path.clone();
        let artifact = artifact.clone();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let metadata_json = serde_json::to_string(&artifact.metadata).map_err(|e| {
                ApplicationError::Persistence(format!("failed to serialize metadata: {e}"))
            })?;

            conn.execute(
                r#"
                INSERT INTO transcript_artifacts (
                    id, job_id, title, kind, input_path, raw_transcript, optimized_transcript,
                    summary, faqs, metadata_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                "#,
                params![
                    artifact.id,
                    artifact.job_id,
                    artifact.title,
                    artifact.kind.as_str(),
                    artifact.input_path,
                    artifact.raw_transcript,
                    artifact.optimized_transcript,
                    artifact.summary,
                    artifact.faqs,
                    metadata_json,
                    artifact.created_at.to_rfc3339(),
                    artifact.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| {
                ApplicationError::Persistence(format!("failed to insert artifact: {e}"))
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
        let db_path = self.db_path.clone();
        let kind_value = kind.map(|k| k.as_str().to_string());
        let query_value = query
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty());

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let mut sql = String::from(
                "SELECT id, job_id, title, kind, input_path, raw_transcript, optimized_transcript, summary, faqs, metadata_json, created_at, updated_at FROM transcript_artifacts",
            );

            let mut clauses: Vec<String> = Vec::new();
            clauses.push("is_deleted = 0".to_string());
            let mut params_values: Vec<Value> = Vec::new();

            if let Some(kind) = kind_value {
                clauses.push("kind = ?".to_string());
                params_values.push(Value::Text(kind));
            }

            if let Some(query) = query_value {
                clauses.push(
                    "(LOWER(title) LIKE LOWER(?) OR LOWER(input_path) LIKE LOWER(?) OR LOWER(raw_transcript) LIKE LOWER(?) OR LOWER(optimized_transcript) LIKE LOWER(?))".to_string(),
                );
                let like = format!("%{query}%");
                params_values.push(Value::Text(like.clone()));
                params_values.push(Value::Text(like.clone()));
                params_values.push(Value::Text(like.clone()));
                params_values.push(Value::Text(like));
            }

            if !clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&clauses.join(" AND "));
            }

            sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
            params_values.push(Value::Integer(limit as i64));
            params_values.push(Value::Integer(offset as i64));

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| ApplicationError::Persistence(format!("failed to prepare query: {e}")))?;

            let rows = stmt
                .query_map(params_from_iter(params_values.iter()), Self::row_to_artifact)
                .map_err(|e| ApplicationError::Persistence(format!("failed to query artifacts: {e}")))?;

            let mut artifacts = Vec::new();
            for row in rows {
                artifacts.push(row.map_err(|e| {
                    ApplicationError::Persistence(format!("failed to parse artifact row: {e}"))
                })?);
            }

            Ok(artifacts)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let db_path = self.db_path.clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT id, job_id, title, kind, input_path, raw_transcript, optimized_transcript,
                           summary, faqs, metadata_json, created_at, updated_at
                    FROM transcript_artifacts
                    WHERE id = ?1 AND is_deleted = 0
                    LIMIT 1
                    "#,
                )
                .map_err(|e| ApplicationError::Persistence(format!("failed to prepare query: {e}")))?;

            let mut rows = stmt
                .query(params![id])
                .map_err(|e| ApplicationError::Persistence(format!("failed to run query: {e}")))?;

            let Some(row) = rows
                .next()
                .map_err(|e| ApplicationError::Persistence(format!("failed to read row: {e}")))?
            else {
                return Ok(None);
            };

            let artifact = Self::row_to_artifact(row)
                .map_err(|e| ApplicationError::Persistence(format!("failed to parse artifact row: {e}")))?;

            Ok(Some(artifact))
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
        let db_path = self.db_path.clone();
        let id = id.to_string();
        let optimized_transcript = optimized_transcript.to_string();
        let summary = summary.to_string();
        let faqs = faqs.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let Some(existing_metadata_json) = conn
                .query_row(
                    "SELECT metadata_json FROM transcript_artifacts WHERE id = ?1 AND is_deleted = 0 LIMIT 1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to load artifact metadata before update: {e}"
                    ))
                })?
            else {
                return Ok(None);
            };

            let mut metadata: BTreeMap<String, String> = serde_json::from_str(&existing_metadata_json)
                .map_err(|e| {
                    ApplicationError::Persistence(format!(
                        "failed to parse artifact metadata before update: {e}"
                    ))
                })?;
            if optimized_transcript.trim().is_empty() {
                metadata.remove(HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY);
            } else {
                metadata.insert(
                    HAS_OPTIMIZED_TRANSCRIPT_METADATA_KEY.to_string(),
                    "true".to_string(),
                );
            }
            let metadata_json = serde_json::to_string(&metadata).map_err(|e| {
                ApplicationError::Persistence(format!(
                    "failed to serialize artifact metadata during update: {e}"
                ))
            })?;

            let updated_rows = conn
                .execute(
                    r#"
                    UPDATE transcript_artifacts
                    SET optimized_transcript = ?2,
                        summary = ?3,
                        faqs = ?4,
                        metadata_json = ?5,
                        updated_at = ?6
                    WHERE id = ?1 AND is_deleted = 0
                    "#,
                    params![
                        id,
                        optimized_transcript,
                        summary,
                        faqs,
                        metadata_json,
                        Utc::now().to_rfc3339()
                    ],
                )
                .map_err(|e| ApplicationError::Persistence(format!("failed to update artifact: {e}")))?;

            if updated_rows == 0 {
                return Ok(None);
            }

            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT id, job_id, title, kind, input_path, raw_transcript, optimized_transcript,
                           summary, faqs, metadata_json, created_at, updated_at
                    FROM transcript_artifacts
                    WHERE id = ?1 AND is_deleted = 0
                    LIMIT 1
                    "#,
                )
                .map_err(|e| ApplicationError::Persistence(format!("failed to prepare query: {e}")))?;

            let mut rows = stmt
                .query(params![id])
                .map_err(|e| ApplicationError::Persistence(format!("failed to run query: {e}")))?;

            let Some(row) = rows
                .next()
                .map_err(|e| ApplicationError::Persistence(format!("failed to read row: {e}")))?
            else {
                return Ok(None);
            };

            let artifact = Self::row_to_artifact(row)
                .map_err(|e| ApplicationError::Persistence(format!("failed to parse artifact row: {e}")))?;

            Ok(Some(artifact))
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn update_timeline_v2(
        &self,
        id: &str,
        timeline_v2_json: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let db_path = self.db_path.clone();
        let id = id.to_string();
        let timeline_v2_json = timeline_v2_json.trim().to_string();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            // Validate timeline payload before persisting it.
            let _: serde_json::Value = serde_json::from_str(&timeline_v2_json).map_err(|e| {
                ApplicationError::Validation(format!("invalid timeline_v2 JSON: {e}"))
            })?;

            let mut metadata_stmt = conn
                .prepare(
                    r#"
                    SELECT metadata_json
                    FROM transcript_artifacts
                    WHERE id = ?1 AND is_deleted = 0
                    LIMIT 1
                    "#,
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to prepare metadata query: {e}"))
                })?;

            let metadata_json: String = match metadata_stmt.query_row(params![id], |row| row.get(0))
            {
                Ok(value) => value,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => {
                    return Err(ApplicationError::Persistence(format!(
                        "failed to read metadata: {e}"
                    )))
                }
            };

            let mut metadata: BTreeMap<String, String> =
                serde_json::from_str(&metadata_json).unwrap_or_default();
            metadata.insert("timeline_v2".to_string(), timeline_v2_json);
            let updated_metadata_json = serde_json::to_string(&metadata).map_err(|e| {
                ApplicationError::Persistence(format!("failed to serialize metadata: {e}"))
            })?;

            let updated_rows = conn
                .execute(
                    r#"
                    UPDATE transcript_artifacts
                    SET metadata_json = ?2,
                        updated_at = ?3
                    WHERE id = ?1 AND is_deleted = 0
                    "#,
                    params![id, updated_metadata_json, Utc::now().to_rfc3339()],
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to update timeline metadata: {e}"))
                })?;

            if updated_rows == 0 {
                return Ok(None);
            }

            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT id, job_id, title, kind, input_path, raw_transcript, optimized_transcript,
                           summary, faqs, metadata_json, created_at, updated_at
                    FROM transcript_artifacts
                    WHERE id = ?1 AND is_deleted = 0
                    LIMIT 1
                    "#,
                )
                .map_err(|e| ApplicationError::Persistence(format!("failed to prepare query: {e}")))?;

            let mut rows = stmt
                .query(params![id])
                .map_err(|e| ApplicationError::Persistence(format!("failed to run query: {e}")))?;

            let Some(row) = rows
                .next()
                .map_err(|e| ApplicationError::Persistence(format!("failed to read row: {e}")))?
            else {
                return Ok(None);
            };

            let artifact = Self::row_to_artifact(row)
                .map_err(|e| ApplicationError::Persistence(format!("failed to parse artifact row: {e}")))?;

            Ok(Some(artifact))
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn rename(
        &self,
        id: &str,
        new_title: &str,
    ) -> Result<Option<TranscriptArtifact>, ApplicationError> {
        let db_path = self.db_path.clone();
        let id = id.to_string();
        let new_title = new_title.trim().to_string();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let updated_rows = conn
                .execute(
                r#"
                UPDATE transcript_artifacts
                SET title = ?2,
                    updated_at = ?3
                WHERE id = ?1 AND is_deleted = 0
                "#,
                params![id, new_title, Utc::now().to_rfc3339()],
            )
            .map_err(|e| ApplicationError::Persistence(format!("failed to rename artifact: {e}")))?;

            if updated_rows == 0 {
                return Ok(None);
            }

            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT id, job_id, title, kind, input_path, raw_transcript, optimized_transcript,
                           summary, faqs, metadata_json, created_at, updated_at
                    FROM transcript_artifacts
                    WHERE id = ?1 AND is_deleted = 0
                    LIMIT 1
                    "#,
                )
                .map_err(|e| ApplicationError::Persistence(format!("failed to prepare query: {e}")))?;

            let mut rows = stmt
                .query(params![id])
                .map_err(|e| ApplicationError::Persistence(format!("failed to run query: {e}")))?;

            let Some(row) = rows
                .next()
                .map_err(|e| ApplicationError::Persistence(format!("failed to read row: {e}")))?
            else {
                return Ok(None);
            };

            let artifact = Self::row_to_artifact(row)
                .map_err(|e| ApplicationError::Persistence(format!("failed to parse artifact row: {e}")))?;

            Ok(Some(artifact))
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let db_path = self.db_path.clone();
        let ids = ids.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "UPDATE transcript_artifacts SET is_deleted = 1, deleted_at = ?, updated_at = ? WHERE id IN ({placeholders}) AND is_deleted = 0"
            );
            let now = Utc::now().to_rfc3339();
            let mut values: Vec<Value> = Vec::with_capacity(ids.len() + 2);
            values.push(Value::Text(now.clone()));
            values.push(Value::Text(now));
            values.extend(ids.into_iter().map(Value::Text));

            let deleted = conn
                .execute(&query, params_from_iter(values.iter()))
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to delete artifacts: {e}"))
                })?;

            Ok(deleted)
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
        let db_path = self.db_path.clone();
        let kind_value = kind.map(|k| k.as_str().to_string());
        let query_value = query
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty());

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let mut sql = String::from(
                "SELECT id, job_id, title, kind, input_path, raw_transcript, optimized_transcript, summary, faqs, metadata_json, created_at, updated_at FROM transcript_artifacts",
            );

            let mut clauses: Vec<String> = Vec::new();
            clauses.push("is_deleted = 1".to_string());
            let mut params_values: Vec<Value> = Vec::new();

            if let Some(kind) = kind_value {
                clauses.push("kind = ?".to_string());
                params_values.push(Value::Text(kind));
            }

            if let Some(query) = query_value {
                clauses.push(
                    "(LOWER(title) LIKE LOWER(?) OR LOWER(input_path) LIKE LOWER(?) OR LOWER(raw_transcript) LIKE LOWER(?) OR LOWER(optimized_transcript) LIKE LOWER(?))".to_string(),
                );
                let like = format!("%{query}%");
                params_values.push(Value::Text(like.clone()));
                params_values.push(Value::Text(like.clone()));
                params_values.push(Value::Text(like.clone()));
                params_values.push(Value::Text(like));
            }

            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
            sql.push_str(" ORDER BY deleted_at DESC, updated_at DESC LIMIT ? OFFSET ?");
            params_values.push(Value::Integer(limit as i64));
            params_values.push(Value::Integer(offset as i64));

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| ApplicationError::Persistence(format!("failed to prepare query: {e}")))?;

            let rows = stmt
                .query_map(params_from_iter(params_values.iter()), Self::row_to_artifact)
                .map_err(|e| ApplicationError::Persistence(format!("failed to query artifacts: {e}")))?;

            let mut artifacts = Vec::new();
            for row in rows {
                artifacts.push(row.map_err(|e| {
                    ApplicationError::Persistence(format!("failed to parse artifact row: {e}"))
                })?);
            }

            Ok(artifacts)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn restore_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let db_path = self.db_path.clone();
        let ids = ids.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "UPDATE transcript_artifacts SET is_deleted = 0, deleted_at = NULL, updated_at = ? WHERE id IN ({placeholders}) AND is_deleted = 1"
            );
            let now = Utc::now().to_rfc3339();
            let mut values: Vec<Value> = Vec::with_capacity(ids.len() + 1);
            values.push(Value::Text(now));
            values.extend(ids.into_iter().map(Value::Text));

            let restored = conn
                .execute(&query, params_from_iter(values.iter()))
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to restore artifacts: {e}"))
                })?;

            Ok(restored)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn hard_delete_many(&self, ids: &[String]) -> Result<usize, ApplicationError> {
        let db_path = self.db_path.clone();
        let ids = ids.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;

            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let query = format!(
                "DELETE FROM transcript_artifacts WHERE id IN ({placeholders}) AND is_deleted = 1"
            );
            let values: Vec<Value> = ids.into_iter().map(Value::Text).collect();

            let deleted = conn
                .execute(&query, params_from_iter(values.iter()))
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to hard-delete artifacts: {e}"))
                })?;

            Ok(deleted)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }

    async fn purge_deleted_older_than_days(&self, days: u32) -> Result<usize, ApplicationError> {
        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || {
            let conn = Connection::open(db_path).map_err(|e| {
                ApplicationError::Persistence(format!("failed to open sqlite database: {e}"))
            })?;
            let cutoff = (Utc::now() - Duration::days(i64::from(days))).to_rfc3339();

            let deleted = conn
                .execute(
                    "DELETE FROM transcript_artifacts WHERE is_deleted = 1 AND deleted_at IS NOT NULL AND deleted_at < ?1",
                    params![cutoff],
                )
                .map_err(|e| {
                    ApplicationError::Persistence(format!("failed to purge deleted artifacts: {e}"))
                })?;

            Ok(deleted)
        })
        .await
        .map_err(|e| ApplicationError::Persistence(format!("storage task join error: {e}")))?
    }
}
