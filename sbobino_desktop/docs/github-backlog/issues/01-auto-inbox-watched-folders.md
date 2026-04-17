# Epic: Auto Inbox from watched folders

Suggested labels: `epic`, `product`, `automation`, `macos`, `ingestion`

## Problem

Today Sbobino starts from a manual import flow. That adds friction for users who already save audio into a predictable folder, especially when the recording is created on another device and synced to the Mac automatically.

## Goal

Let users choose one or more local or sync-mounted folders and have Sbobino discover, queue, and transcribe new supported audio files automatically.

## User value

- Students can record from a phone and find the lecture ready on desktop later.
- Professionals can drop meetings or interviews into a team folder and let Sbobino prepare them without repeated manual import.

## Proposal

Add an Auto Inbox capability that:

- stores `watchedFolders[]` in settings
- scans each source incrementally
- uses file watching when available and periodic rescans as fallback
- deduplicates by stable path + size + modified time, with optional hash fallback
- queues new items for transcription without duplicating already processed audio

## V1 scope

- local folders on macOS
- filesystem-synced folders from iCloud Drive, Dropbox, Google Drive desktop sync, and OneDrive sync
- supported audio file extension filtering
- per-source enable/disable
- per-source workspace or preset association may be added later if not already available

## Acceptance criteria

- Users can add, remove, enable, and disable watched folders in Settings.
- New eligible audio files are discovered automatically and appear in the queue once.
- Already processed files are not retranscribed after restart, rename, or move when deduplication can match them safely.
- The queue persists across app restarts.
- Discovery failures do not block startup and are visible in UI.

## Test scenarios

- A new `.m4a` file appears in a watched folder while the app is open and is queued once.
- A new `.mp3` file appears while the app is closed and is queued on next launch.
- A file is renamed after successful transcription and does not create a duplicate transcript.
- A watched folder becomes unavailable and the error is surfaced without crashing the app.

## Out of scope

- Native Dropbox, Google Drive, or OneDrive APIs
- Team-wide shared queue orchestration
- Automatic remote AI post-processing by default
