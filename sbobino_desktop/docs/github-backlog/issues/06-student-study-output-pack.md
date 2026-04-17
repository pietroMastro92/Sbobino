# Epic: Student study output pack

Suggested labels: `epic`, `product`, `students`, `study-tools`, `ai`

## Problem

Students often do not need just a transcript. They need study material that can be reviewed quickly, reused for revision, and exported into personal study systems.

## Goal

Turn lecture-style transcripts into study-ready outputs with minimal manual effort.

## User value

- Faster revision after class
- Better extraction of key concepts from long recordings
- Reusable artifacts for notes, flashcards, and exam preparation

## Proposal

Introduce study-oriented outputs such as:

- structured lecture notes
- glossary of key terms
- probable exam questions
- topic timeline
- flashcard-friendly exports

These should sit on top of the existing artifact model as generated outputs rather than replacing the transcript itself.

## V1 scope

- preset-driven generation for lecture-style recordings
- dedicated export templates for study artifacts
- local-safe behavior when remote providers are disabled

## Acceptance criteria

- A lecture artifact can generate at least one dedicated study output beyond the generic summary.
- Study outputs are saved as reusable artifact-derived content.
- Export surfaces can include study outputs where available.

## Test scenarios

- A lecture recording generates structured notes and a glossary.
- A study export includes the new generated sections when enabled.
- Missing AI capability degrades gracefully instead of breaking the transcript workflow.

## Out of scope

- LMS integrations
- Spaced repetition syncing to third-party services
- Multi-document study synthesis in V1
