# Epic: Apple Voice Memos / iCloud-first import

Suggested labels: `epic`, `product`, `macos`, `icloud`, `ingestion`

## Problem

One of the most natural capture flows on Apple devices is recording with Voice Memos on iPhone and letting the recording sync to the Mac. Sbobino should fit that workflow directly instead of forcing a manual export or drag-and-drop step.

## Goal

Provide a guided macOS-first source preset for Apple Voice Memos and other iCloud-synced recordings already present on the local filesystem.

## User value

- Students can record explanations or classes on iPhone and find them prepared on Mac automatically.
- Professionals can capture quick spoken notes or meeting recaps from mobile and have them enter Sbobino with no extra handling.

## Proposal

Add a source preset called `Voice Memos` that:

- helps users point Sbobino at the correct synced location on macOS
- applies safe discovery rules for Apple-generated recordings
- maps imported items to source metadata that clearly identifies their origin
- cooperates with Auto Inbox deduplication and queueing

## V1 scope

- macOS only
- relies on files already synchronized to the local filesystem
- guided setup copy explaining that Sbobino reads local synced files and does not use Apple cloud APIs
- origin metadata such as `source_origin`, source label, and source path snapshot

## Acceptance criteria

- Users can enable a `Voice Memos` source without needing cloud API credentials.
- Newly synced recordings are discovered and queued automatically.
- Imported Voice Memos artifacts preserve readable origin metadata in history and detail views.
- Missing or moved synced locations produce a readable remediation path.

## Test scenarios

- A Voice Memos recording created on iPhone syncs to the Mac and is picked up once.
- A synced memo already processed before restart is not ingested again.
- The Voice Memos source path is missing and the app surfaces a non-blocking warning.

## Out of scope

- Direct iPhone device pairing
- Apple private APIs or direct iCloud service integration
- Automatic upload or sync from non-filesystem Apple services
