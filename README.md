# Sbobino

Sbobino is a desktop workspace for turning lessons, meetings, interviews, and voice notes into structured, usable knowledge.

Built with **Tauri v2, Rust, React, and TypeScript**, it combines **local-first transcription**, **speaker-aware processing**, and **AI-assisted post-processing** in a native-style desktop app designed for real study and work workflows.

Its name comes from the Italian verb *sbobinare*: traditionally, to transcribe the contents of a recorded tape. Sbobino keeps that original spirit, but gives it a modern twist: less drudgery, more clarity, and a much better desk companion when the audio is long and your time is short.

## Who It Is For

Sbobino is especially useful for:

- **Students** who need to turn lectures, oral explanations, and recorded study sessions into clean notes, summaries, and exportable material
- **Professionals** who want to capture meetings, interviews, calls, or brainstorming sessions and turn them into structured outputs, action-oriented summaries, and searchable knowledge
- **Knowledge workers** who need a practical bridge between raw audio and documents they can actually reuse

## Why It Matters

Most transcription tools stop at raw text.

Sbobino is built to go further:

- capture audio and generate a transcript
- improve and refine transcript quality
- summarize long conversations into useful takeaways
- ask questions directly on transcript content
- manage speaker-aware segments
- export professional outputs for study, reporting, or documentation

The goal is simple: reduce the time between “I recorded something important” and “I now have material I can study, share, or work from.”

## Core Capabilities

### Local-first transcription

- Transcribe local audio files in a desktop workflow
- Track progress live while jobs are running
- Manage active, queued, and completed transcriptions
- Reopen and continue working on previous transcripts from history

### Transcript improvement

- Refine transcript wording with AI-assisted post-processing
- Switch between original and optimized transcript versions
- Trim audio and generate focused child transcripts from specific regions

### Study and work outputs

- Generate structured summaries from long recordings
- Produce FAQs and supporting transcript-derived artifacts
- Chat with the transcript to extract facts, clarify points, or recover context
- Keep output aligned with the prompt language and selected formatting behavior

### Speaker-aware workflow

- Browse timestamped segments
- Assign and manage speaker labels
- Use local pyannote diarization when runtime assets are installed
- Keep transcript structure easier to review for lessons, meetings, and interviews

### Professional export

- Export in formats such as `txt`, `docx`, `html`, `pdf`, `md`, `csv`, `json`, `srt`, and `vtt`
- Export full transcripts, subtitles, or segment-oriented documents
- Include summaries and FAQs in exported outputs when available

## Practical Use Cases

### For students

- Turn lectures into readable notes
- Create study summaries after class
- Recover key explanations from long recordings
- Export clean material for revision and sharing

### For work

- Turn meetings into structured notes
- Capture interviews and conversations with better traceability
- Generate summaries, FAQs, and working documents faster
- Keep a searchable local archive of spoken knowledge

## Product Principles

- **Local-first where it matters**
  Runtime assets and transcription workflows are designed to run locally on the machine.

- **Useful beyond transcription**
  The app is meant to transform recordings into outputs that support studying, reporting, and decision-making.

- **Desktop-native experience**
  Tauri keeps the app lightweight while Rust handles orchestration, runtime integration, and reliability-sensitive flows.

- **Structured architecture**
  The codebase is organized for maintainability, long-term iteration, and production-grade desktop evolution.

## Architecture

The project is organized as a Rust workspace with a clean layered structure:

- `sbobino_desktop/crates/domain`
  Core business entities and rules
- `sbobino_desktop/crates/application`
  Use cases, orchestration, and ports
- `sbobino_desktop/crates/infrastructure`
  Adapters for transcription engines, AI providers, persistence, and runtime integration
- `sbobino_desktop/apps/desktop`
  React + TypeScript frontend
- `sbobino_desktop/apps/desktop/src-tauri`
  Tauri command layer and desktop runtime composition

## Repository Layout

```text
.
├── README.md
├── sbobino_desktop/
│   ├── apps/
│   │   └── desktop/
│   ├── crates/
│   │   ├── application/
│   │   ├── domain/
│   │   └── infrastructure/
│   ├── docs/
│   └── scripts/
```

## Getting Started

### Prerequisites

- Rust stable
- Node.js 20+
- npm
- macOS recommended for the current desktop workflow

### Run the app

```bash
cd sbobino_desktop/apps/desktop
npm ci
npm run tauri:dev
```

### Runtime setup

From the `sbobino_desktop` workspace root:

```bash
./scripts/setup_runtime.sh
```

Optional:

- install speaker diarization assets from `Settings > Local Models > Speaker Diarization`
- configure AI services in `Settings > AI Services`

## Development Commands

### Frontend

```bash
cd sbobino_desktop/apps/desktop
npm test -- --run
npm run build
```

### Rust

```bash
cd sbobino_desktop
cargo test -p sbobino-application --test transcription_service_tests
cargo check -p sbobino-desktop
```

## Documentation

Useful technical references live inside `sbobino_desktop/docs`:

- [Architecture](./sbobino_desktop/docs/architecture.md)
- [Release and migration notes](./sbobino_desktop/docs/release-and-migration.md)
- [Feature migration matrix](./sbobino_desktop/docs/feature-migration-matrix.md)
- [Workspace README](./sbobino_desktop/README.md)

## Status

Sbobino is evolving as a serious desktop product for people who need to turn spoken content into usable knowledge quickly, clearly, and with more control than a generic transcription tool.
