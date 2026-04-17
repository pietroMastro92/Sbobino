# Epic: Post-processing automation rules

Suggested labels: `epic`, `product`, `automation`, `ai`, `workflow`

## Problem

Transcription alone is not enough for many workflows. Users often want summaries, titles, FAQs, diarization, or emotion analysis, but they do not always want every expensive AI step to run automatically.

## Goal

Let users define safe automation presets that run selected post-processing steps after transcription based on source type or workflow intent.

## User value

- Students get study-ready notes with minimal repetition.
- Enterprise users get structured outputs without manual clicking after every transcript.
- Privacy-sensitive users stay in control of which steps can run automatically.

## Proposal

Add source-aware automation presets such as:

- `Lecture`
- `Meeting`
- `Interview`
- `Voice Memo`

Each preset can toggle:

- summary generation
- FAQ generation
- title suggestions
- speaker diarization
- emotion analysis
- local-first or remote-capable behavior according to current AI/privacy settings

## V1 scope

- preset-based automation only
- default behavior favors low-cost or local-safe steps unless the user opts in
- uses the shared transcript post-processing pipeline instead of separate one-off paths

## Acceptance criteria

- Users can assign a preset to an automatic source.
- Selected post-processing starts automatically after a successful transcript.
- Remote-capable steps do not run when privacy or provider settings disallow them.
- Automatic post-processing status is visible per artifact.

## Test scenarios

- A `Lecture` preset creates a transcript summary automatically.
- A `Meeting` preset generates summary plus action-oriented output when enabled.
- Remote AI steps remain skipped under restrictive privacy settings.
- Failed post-processing can be retried without duplicating the transcript.

## Out of scope

- Fully custom workflow builder with arbitrary step graphs
- Cross-artifact batch rules
- Background execution of unsupported provider combinations
