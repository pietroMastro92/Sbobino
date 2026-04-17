# GitHub Backlog: Automated Ingestion and Productive Outputs

This folder contains issue-ready product epics for the next Sbobino backlog wave.

The backlog is intentionally shaped around the current desktop architecture:

- persistent settings and repositories
- job queue and artifact history
- automatic post-processing hooks
- local-first privacy boundaries
- non-blocking startup

## Product direction

Shared core:

- ingest new audio automatically from user-controlled sources
- prepare usable outputs before the user opens the app
- keep automation visible, reversible, and privacy-safe

Initial priority order:

1. Auto Inbox from watched folders
2. Apple Voice Memos / iCloud-first import
3. Background worker with fast startup
4. Post-processing automation rules
5. Workspaces and smart folders
6. Student study outputs
7. Enterprise meeting intelligence
8. Trust, control, and local compliance

## V1 defaults

- Sources: local folders and filesystem-synced cloud folders only
- Cloud scope: no native provider APIs in V1
- Deduplication: stable path + size + modified time, with optional hash fallback
- Runtime model: persistent scanner plus watcher where available, periodic rescan otherwise
- Privacy: no implicit upload of new audio
- UX: one "Automatic Import" dashboard for source status, recent discoveries, and errors

## Sequencing guidance

Wave 1:

- `01-auto-inbox-watched-folders.md`
- `02-apple-voice-memos-icloud-import.md`
- `03-background-worker-fast-startup.md`
- `08-trust-control-local-compliance.md`

Wave 2:

- `04-post-processing-automation-rules.md`
- `05-workspaces-smart-folders.md`

Wave 3:

- `06-student-study-output-pack.md`
- `07-enterprise-meeting-intelligence.md`

## Shared test scenarios

- A new Voice Memos recording synced to the Mac is queued once and is already available when the user opens the app.
- A new file appears in an iCloud Drive or Dropbox synced folder while the app is closed and is ingested correctly on the next startup.
- A file rename or move does not create a duplicate transcript when the audio was already processed.
- A cloud placeholder that is not yet downloaded locally produces a readable error and a safe retry path.
- Automatic import remains responsive with many watched sources and does not block bootstrap.
- Restrictive privacy settings prevent remote AI automation from running implicitly.

## Issue drafts

- [01 Auto Inbox from watched folders](./issues/01-auto-inbox-watched-folders.md)
- [02 Apple Voice Memos / iCloud-first import](./issues/02-apple-voice-memos-icloud-import.md)
- [03 Background worker and fast startup](./issues/03-background-worker-fast-startup.md)
- [04 Post-processing automation rules](./issues/04-post-processing-automation-rules.md)
- [05 Workspaces and smart folders](./issues/05-workspaces-smart-folders.md)
- [06 Student study output pack](./issues/06-student-study-output-pack.md)
- [07 Enterprise meeting intelligence](./issues/07-enterprise-meeting-intelligence.md)
- [08 Trust, control, and local compliance](./issues/08-trust-control-local-compliance.md)
