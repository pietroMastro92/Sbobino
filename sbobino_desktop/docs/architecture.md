# Sbobino Rewrite Architecture

## Dependency Rules

- `domain` has zero dependency on Tauri, IO, network, and process execution.
- `application` depends on `domain` and declares ports.
- `infrastructure` depends on `application` and `domain` and implements ports.
- `apps/desktop/src-tauri` composes services and exposes command handlers.
- `apps/desktop` consumes only typed Tauri API wrappers and event streams.

## Layer Responsibilities

### Domain (`crates/domain`)
- Business entities: `TranscriptionJob`, `TranscriptArtifact`, `AppSettings`
- Domain enums: `JobStage`, `JobStatus`, `SpeechModel`, `LanguageCode`
- Validation constraints

### Application (`crates/application`)
- Use cases: `TranscriptionService`, `SettingsService`
- Ports: `AudioTranscoder`, `SpeechToTextEngine`, `TranscriptEnhancer`, repositories
- Orchestration and lifecycle progression

### Infrastructure (`crates/infrastructure`)
- Process adapters:
  - `FfmpegAdapter`
  - `WhisperCppEngine` (file transcription through the native CLI)
  - `ParakeetCppEngine` (resident batch file transcription)
  - `WhisperStreamEngine` (Live transcription through the native streaming runtime)
- API adapters:
  - `GeminiEnhancer`
  - `NoopEnhancer`
- Persistence adapters:
  - `SqliteArtifactRepository`
  - `FsSettingsRepository`
- Runtime composition:
  - `RuntimeTranscriptionFactory` builds a fresh `TranscriptionService` from current settings for each new job.
  - This ensures adapter reconfiguration (e.g. Gemini key/model, binary paths) applies without app restart.

### Tauri Command Layer (`apps/desktop/src-tauri`)
- Commands:
  - `start_transcription`
  - `cancel_transcription`
  - `list_recent_artifacts`
  - `get_artifact`
  - `update_artifact`
  - `get_settings`
  - `update_settings`
- Event bus topics:
  - `transcription://progress`
  - `transcription://completed`
  - `transcription://failed`

### Frontend Presentation (`apps/desktop`)
- React + TypeScript
- Zustand state for local app state
- Typed Tauri service wrappers in `src/lib/tauri.ts`
- No domain logic in React components

## Why This Is Better Than Python MVC

- Business workflow moves from GUI callbacks to testable Rust use-cases.
- Process execution is adapter-owned, not spread across UI/controller code.
- Persistence is strongly typed and centralized in repository adapters.
- Frontend communicates via stable commands/events, mirroring native desktop app boundaries.
- Clear module ownership enables team parallelism and lower regression risk.

## Native transcription selection

The domain terms are defined in [GLOSSARY.md](../../GLOSSARY.md). The configured engine is a persistent file-transcription preference; the effective engine belongs to a particular session. A Live fallback must not replace the configured engine.

The runtime command module owns the Live selection interface used by readiness and start. It resolves both configured engines to Whisper and the existing certified Live model manifest. The frontend forwards the configured engine instead of repeating this decision. File transcription continues to select the Whisper or Parakeet adapter through `RuntimeTranscriptionFactory`.

This seam concentrates the current engine/model policy: one decision gives locality to changes and leverage to both command callers. Runtime executability, model presence, microphone readiness, language routing and device checks remain in their existing implementation. Passing readiness is not a guarantee of realtime throughput on every device.

Whisper runtime path normalization repairs legacy executable paths without changing the configured engine. Readiness therefore cannot silently replace a Parakeet file preference with Whisper while preparing a Live session.

The meaningful regression cases are a Parakeet-configured session resolving to the same certified Whisper model in readiness and start, preservation of the file preference during runtime path normalization, and unchanged explicit language and compute-device routing. Gemma and Redux remain isolated benchmark candidates.
