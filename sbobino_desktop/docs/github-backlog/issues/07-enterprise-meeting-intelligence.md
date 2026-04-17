# Epic: Enterprise meeting intelligence

Suggested labels: `epic`, `product`, `enterprise`, `meetings`, `ai`

## Problem

Enterprise users need outputs that are closer to meeting operations than to raw transcripts. They need decisions, owners, deadlines, and unresolved questions in a format that can be reviewed and shared quickly.

## Goal

Provide structured meeting outputs on top of transcript artifacts without compromising Sbobino's local-first positioning.

## User value

- Faster meeting follow-up
- Better accountability from recorded discussions
- Higher value for interviews, client calls, and internal coordination

## Proposal

Add enterprise-oriented generated outputs such as:

- meeting minutes
- decision log
- action items with owner and due date fields
- risks and open questions
- export templates for internal or client-facing notes

## V1 scope

- generated outputs stored as artifact-derived content
- export templates tuned for work contexts
- no direct task-system integration required in V1

## Acceptance criteria

- A meeting-oriented artifact can generate structured action items and decisions.
- Generated outputs are editable or exportable without altering the raw transcript.
- Enterprise outputs respect existing privacy and AI-provider settings.

## Test scenarios

- A meeting preset produces action items and a decision log.
- An interview preset emphasizes open questions and key takeaways.
- Exported meeting notes include the structured sections when enabled.

## Out of scope

- Slack, Jira, Asana, or calendar writeback
- Organization-wide analytics
- Compliance archiving integrations beyond local metadata
