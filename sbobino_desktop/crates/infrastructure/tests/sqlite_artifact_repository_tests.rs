use std::collections::BTreeMap;
use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::Connection;
use tempfile::tempdir;

use sbobino_application::ArtifactRepository;
use sbobino_domain::{ArtifactKind, ArtifactSourceOrigin, TranscriptArtifact};
use sbobino_infrastructure::repositories::sqlite_artifact_repository::SqliteArtifactRepository;

fn enable_local_secure_storage_for_tests() {
    std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
}

fn artifact_with_job(job_id: &str, input_path: &str, transcript: &str) -> TranscriptArtifact {
    TranscriptArtifact::new(
        job_id.to_string(),
        Path::new(input_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(input_path)
            .to_string(),
        ArtifactKind::File,
        input_path.to_string(),
        ArtifactSourceOrigin::Imported,
        transcript.to_string(),
        transcript.to_string(),
        String::new(),
        String::new(),
        BTreeMap::new(),
    )
    .expect("valid artifact")
}

#[tokio::test]
async fn save_then_get_by_id_returns_persisted_artifact() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let db_path = temp.path().join("artifacts.db");
    let repo = SqliteArtifactRepository::new(db_path).expect("repo should initialize");

    let artifact = artifact_with_job("job-a", "/tmp/audio-a.wav", "hello transcript");
    let artifact_id = artifact.id.clone();

    repo.save(&artifact).await.expect("save should succeed");
    let loaded = repo
        .get_by_id(&artifact_id)
        .await
        .expect("query should succeed")
        .expect("artifact should exist");

    assert_eq!(loaded.id, artifact.id);
    assert_eq!(loaded.job_id, "job-a");
    assert_eq!(loaded.raw_transcript, "hello transcript");
    assert_eq!(loaded.optimized_transcript, "hello transcript");
}

#[tokio::test]
async fn diarization_result_updates_status_and_timeline_in_one_revision() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let repo = SqliteArtifactRepository::new(temp.path().join("artifacts.db"))
        .expect("repo should initialize");
    let mut artifact = artifact_with_job("live-job", "live.wav", "hello live transcript");
    artifact.kind = ArtifactKind::Realtime;
    artifact
        .metadata
        .insert("speaker_diarization_status".into(), "queued".into());
    repo.save(&artifact).await.expect("save should succeed");

    let timeline = r#"{"version":2,"segments":[]}"#;
    let updated = repo
        .update_diarization_result(&artifact.id, Some(timeline), "completed", None)
        .await
        .expect("diarization update")
        .expect("artifact exists");
    assert_eq!(updated.revision, artifact.revision + 1);
    assert_eq!(
        updated
            .metadata
            .get("speaker_diarization_status")
            .map(String::as_str),
        Some("completed")
    );
    assert_eq!(
        updated.metadata.get("timeline_v2").map(String::as_str),
        Some(timeline)
    );
    assert!(!updated.metadata.contains_key("speaker_diarization_error"));
}

#[tokio::test]
async fn startup_marks_unfinished_audio_and_diarization_jobs_as_interrupted() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let repo = SqliteArtifactRepository::new(temp.path().join("artifacts.db"))
        .expect("repo should initialize");
    let mut queued = artifact_with_job("queued", "queued.wav", "queued transcript");
    queued
        .metadata
        .insert("speaker_diarization_status".into(), "queued".into());
    let mut running = artifact_with_job("running", "running.wav", "running transcript");
    running
        .metadata
        .insert("speaker_diarization_status".into(), "running".into());
    let mut completed = artifact_with_job("completed", "completed.wav", "done transcript");
    completed
        .metadata
        .insert("speaker_diarization_status".into(), "completed".into());
    let mut audio_queued = artifact_with_job("audio-queued", "audio.wav", "audio transcript");
    audio_queued
        .metadata
        .insert("audio_import_status".into(), "queued".into());
    for artifact in [&queued, &running, &completed, &audio_queued] {
        repo.save(artifact).await.expect("save should succeed");
    }

    assert_eq!(
        repo.interrupt_pending_postprocessing_jobs()
            .await
            .expect("interrupt"),
        3
    );
    for artifact in [&queued, &running] {
        let updated = repo.get_by_id(&artifact.id).await.unwrap().unwrap();
        assert_eq!(
            updated
                .metadata
                .get("speaker_diarization_status")
                .map(String::as_str),
            Some("interrupted")
        );
        assert_eq!(updated.revision, artifact.revision + 1);
    }
    let audio_interrupted = repo.get_by_id(&audio_queued.id).await.unwrap().unwrap();
    assert_eq!(
        audio_interrupted
            .metadata
            .get("audio_import_status")
            .map(String::as_str),
        Some("interrupted")
    );
    let untouched = repo.get_by_id(&completed.id).await.unwrap().unwrap();
    assert_eq!(untouched.revision, completed.revision);
}

#[tokio::test]
async fn attach_audio_updates_audio_row_and_import_status_in_one_revision() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let repo = SqliteArtifactRepository::new(temp.path().join("artifacts.db"))
        .expect("repo should initialize");
    let audio_path = temp.path().join("live.wav");
    std::fs::write(&audio_path, b"test audio bytes").expect("write audio fixture");
    let mut artifact = artifact_with_job("live-audio", "live.wav", "live transcript");
    artifact.audio_backfill_status = sbobino_domain::ArtifactAudioBackfillStatus::PendingBackfill;
    artifact
        .metadata
        .insert("audio_import_status".into(), "queued".into());
    repo.save(&artifact).await.expect("save transcript first");

    let updated = repo
        .attach_audio_file(&artifact.id, &audio_path)
        .await
        .expect("attach audio")
        .expect("artifact exists");
    assert!(updated.audio_available);
    assert_eq!(updated.revision, artifact.revision + 1);
    assert_eq!(updated.audio_byte_size, Some(16));
    assert_eq!(
        updated
            .metadata
            .get("audio_import_status")
            .map(String::as_str),
        Some("completed")
    );
}

#[tokio::test]
async fn list_recent_returns_newest_first_with_limit() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let db_path = temp.path().join("artifacts.db");
    let repo = SqliteArtifactRepository::new(db_path).expect("repo should initialize");

    let mut oldest = artifact_with_job("job-oldest", "/tmp/old.wav", "old");
    oldest.created_at = Utc::now() - Duration::minutes(2);
    let mut middle = artifact_with_job("job-middle", "/tmp/mid.wav", "mid");
    middle.created_at = Utc::now() - Duration::minutes(1);
    let mut newest = artifact_with_job("job-newest", "/tmp/new.wav", "new");
    newest.created_at = Utc::now();

    repo.save(&oldest)
        .await
        .expect("save oldest should succeed");
    repo.save(&middle)
        .await
        .expect("save middle should succeed");
    repo.save(&newest)
        .await
        .expect("save newest should succeed");

    let recent_two = repo.list_recent(2, 0).await.expect("list should succeed");

    assert_eq!(recent_two.len(), 2);
    assert_eq!(recent_two[0].job_id, "job-newest");
    assert_eq!(recent_two[1].job_id, "job-middle");
}

#[tokio::test]
async fn rename_updates_title_without_mutating_source_label() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let db_path = temp.path().join("artifacts.db");
    let repo = SqliteArtifactRepository::new(db_path).expect("repo should initialize");

    let artifact = artifact_with_job("job-a", "/tmp/my-audio-file.wav", "hello transcript");
    let artifact_id = artifact.id.clone();
    let original_source_label = artifact.source_label.clone();

    repo.save(&artifact).await.expect("save should succeed");

    let renamed = repo
        .rename(&artifact_id, "renamed title")
        .await
        .expect("rename should succeed")
        .expect("artifact should exist");

    assert_eq!(renamed.title, "renamed title");
    assert_eq!(renamed.source_label, original_source_label);

    let loaded = repo
        .get_by_id(&artifact_id)
        .await
        .expect("query should succeed")
        .expect("artifact should exist");

    assert_eq!(loaded.title, "renamed title");
    assert_eq!(loaded.source_label, original_source_label);
}

#[tokio::test]
async fn soft_delete_restore_and_hard_delete_follow_trash_flow() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let db_path = temp.path().join("artifacts.db");
    let repo = SqliteArtifactRepository::new(db_path).expect("repo should initialize");

    let artifact = artifact_with_job("job-trash", "/tmp/trash.wav", "trash me");
    let artifact_id = artifact.id.clone();
    repo.save(&artifact).await.expect("save should succeed");

    let soft_deleted = repo
        .delete_many(std::slice::from_ref(&artifact_id))
        .await
        .expect("soft delete should succeed");
    assert_eq!(soft_deleted, 1);

    let active_after_delete = repo
        .list_recent(10, 0)
        .await
        .expect("active list should query");
    assert!(active_after_delete.is_empty());

    let deleted_list = repo
        .list_deleted(None, None, 10, 0)
        .await
        .expect("deleted list should query");
    assert_eq!(deleted_list.len(), 1);
    assert_eq!(deleted_list[0].id, artifact_id);

    let restored = repo
        .restore_many(std::slice::from_ref(&artifact_id))
        .await
        .expect("restore should succeed");
    assert_eq!(restored, 1);

    let active_after_restore = repo
        .list_recent(10, 0)
        .await
        .expect("active list should query");
    assert_eq!(active_after_restore.len(), 1);
    assert_eq!(active_after_restore[0].id, artifact_id);

    repo.delete_many(std::slice::from_ref(&artifact_id))
        .await
        .expect("soft delete should succeed");
    let hard_deleted = repo
        .hard_delete_many(std::slice::from_ref(&artifact_id))
        .await
        .expect("hard delete should succeed");
    assert_eq!(hard_deleted, 1);
    assert!(repo
        .get_by_id(&artifact_id)
        .await
        .expect("lookup should query")
        .is_none());
}

#[tokio::test]
async fn purge_deleted_older_than_days_removes_only_expired_items() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let db_path = temp.path().join("artifacts.db");
    let repo = SqliteArtifactRepository::new(db_path.clone()).expect("repo should initialize");

    let old_artifact = artifact_with_job("job-old", "/tmp/old.wav", "old");
    let old_id = old_artifact.id.clone();
    let fresh_artifact = artifact_with_job("job-fresh", "/tmp/fresh.wav", "fresh");
    let fresh_id = fresh_artifact.id.clone();
    repo.save(&old_artifact)
        .await
        .expect("save old should succeed");
    repo.save(&fresh_artifact)
        .await
        .expect("save fresh should succeed");

    repo.delete_many(std::slice::from_ref(&old_id))
        .await
        .expect("delete old should succeed");
    repo.delete_many(std::slice::from_ref(&fresh_id))
        .await
        .expect("delete fresh should succeed");

    let conn = Connection::open(&db_path).expect("db should open");
    let stale_cutoff = (Utc::now() - Duration::days(45)).to_rfc3339();
    conn.execute(
        "UPDATE transcript_artifacts SET deleted_at = ?1 WHERE id = ?2",
        [stale_cutoff.as_str(), old_id.as_str()],
    )
    .expect("stale deleted_at should update");

    let purged = repo
        .purge_deleted_older_than_days(30)
        .await
        .expect("purge should succeed");
    assert_eq!(purged, 1);

    let deleted_remaining = repo
        .list_deleted(None, None, 10, 0)
        .await
        .expect("deleted list should query");
    assert_eq!(deleted_remaining.len(), 1);
    assert_eq!(deleted_remaining[0].id, fresh_id);
}

#[test]
fn migrates_legacy_schema_before_creating_kind_index() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let db_path = temp.path().join("artifacts.db");

    {
        let conn = Connection::open(&db_path).expect("legacy db should open");
        conn.execute_batch(
            r#"
            CREATE TABLE transcript_artifacts (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                input_path TEXT NOT NULL,
                raw_transcript TEXT NOT NULL,
                optimized_transcript TEXT NOT NULL,
                summary TEXT NOT NULL,
                faqs TEXT NOT NULL,
                metadata_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            "#,
        )
        .expect("legacy schema should be created");
    }

    let repo = SqliteArtifactRepository::new(db_path.clone());
    assert!(
        repo.is_ok(),
        "repo initialization should migrate legacy schema"
    );

    let conn = Connection::open(db_path).expect("db should open");
    let mut stmt = conn
        .prepare("PRAGMA table_info(transcript_artifacts)")
        .expect("pragma should prepare");

    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("pragma should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("pragma rows should parse");

    assert!(names.contains(&"title_enc".to_string()));
    assert!(names.contains(&"kind".to_string()));
    assert!(names.contains(&"updated_at".to_string()));
    assert!(names.contains(&"is_deleted".to_string()));
    assert!(names.contains(&"deleted_at".to_string()));
    assert!(names.contains(&"source_label_enc".to_string()));
    assert!(names.contains(&"audio_backfill_status".to_string()));
}
