# Epic: Background worker and fast startup

Suggested labels: `epic`, `product`, `performance`, `automation`, `startup`

## Problem

Automatic ingestion is only useful if it does not make the desktop app feel heavier or slower to open. Sbobino already has a strong requirement to keep startup responsive and avoid blocking the first interactive render with expensive runtime work.

## Goal

Run source discovery and queue preparation in a background-friendly way that preserves a fast startup experience.

## User value

- Users open the app and find work already prepared.
- Automation feels native and quiet instead of like a heavyweight sync engine.

## Proposal

Introduce a background ingestion worker that:

- resumes lightweight scanner state from persistence
- performs discovery outside the critical startup path
- schedules rescans safely when file watching is unavailable
- can notify the user about newly prepared content without intrusive interruptions

## V1 scope

- background discovery lifecycle on macOS desktop
- persisted scanner checkpoints and last-seen source state
- UI surface for worker health and latest activity
- no requirement for a standalone always-on daemon outside the app bundle

## Acceptance criteria

- Automatic import does not delay the first interactive UI render noticeably compared with current startup behavior.
- Ingestion state survives app restart.
- Worker errors do not block the rest of the app.
- Users can inspect recent discovery activity and manually trigger a rescan.

## Test scenarios

- The app starts with multiple watched sources and the main UI remains responsive.
- A queued discovery resumes correctly after restart.
- A watcher cannot be created and the system falls back to periodic scanning.

## Out of scope

- Separate privileged background service
- Deep power-management policy automation in V1
- Continuous sync when the full app is never launched
