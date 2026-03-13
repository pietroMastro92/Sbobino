# Sbobino Desktop Rewrite (Tauri v2 + Rust Clean Architecture)

This repository is the production-grade rewrite of the original Python Sbobino app.

## Workspace Layout

- `crates/domain`: pure business model and rules
- `crates/application`: use cases and ports (interfaces)
- `crates/infrastructure`: adapters (whisper/ffmpeg/gemini/filesystem/sqlite)
- `apps/desktop`: React presentation layer
- `apps/desktop/src-tauri`: Tauri command host and runtime composition
- `docs/architecture.md`: architecture and dependency rules
- `docs/release-and-migration.md`: release pipeline and migration plan
- `docs/feature-migration-matrix.md`: feature-by-feature parity checklist

## Quick Start

1. Install Rust stable and Node 20+.
2. `cd apps/desktop`
3. `npm ci`
4. `npm run tauri:dev`

## Runtime Setup (first run)

From workspace root:

1. `./scripts/setup_runtime.sh` (downloads `ggml-base.bin` into app data models dir)
2. In app, keep `Model = Base` for first run.
3. Optional for local speaker diarization: install pyannote assets from `Settings > Local Models > Speaker Diarization`.

## Current Milestone

- Initial clean architecture scaffold
- First background transcription command
- Progress/completion events to frontend
- Minimal working desktop UI
