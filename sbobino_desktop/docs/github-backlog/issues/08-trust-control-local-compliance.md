# Epic: Trust, control, and local compliance

Suggested labels: `epic`, `product`, `privacy`, `compliance`, `reliability`

## Problem

Automation only becomes credible if users can understand what happened, why it happened, and whether any sensitive audio left the machine. This is especially important for professional and enterprise workflows.

## Goal

Make automatic ingestion auditable, controllable, and explicitly local-first.

## User value

- Users can trust the automation instead of treating it as a black box.
- Teams can adopt Sbobino in more sensitive contexts with clearer safeguards.
- Failures are recoverable without silent data loss or repeated duplicate work.

## Proposal

Add a trust layer around automatic ingestion:

- ingestion status dashboard
- readable error log
- source audit trail
- retry controls
- quarantine state for problematic files
- exclusion rules for sensitive folders
- retention controls for automation metadata

## V1 scope

- local audit metadata only
- per-source visibility into last scan, last success, and last failure
- explicit UI language describing local-first behavior and any remote-step constraints

## Acceptance criteria

- Users can inspect where an automatically imported artifact came from.
- Failed ingestions expose readable reasons and retry options.
- Sensitive folders can be excluded from automatic import.
- Privacy-sensitive settings prevent implicit remote processing.

## Test scenarios

- A cloud placeholder file that is not downloaded locally enters a safe error state with retry guidance.
- A corrupt audio file is quarantined instead of poisoning the queue.
- An excluded folder is ignored even if it contains supported files.

## Out of scope

- Centralized enterprise admin console
- External SIEM or compliance platform export
- Legal policy generation beyond product copy and settings behavior
