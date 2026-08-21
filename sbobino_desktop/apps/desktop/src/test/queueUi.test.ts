import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const appSource = fs
  .readFileSync(path.resolve(currentDir, "../App.tsx"), "utf8")
  .replaceAll("\r\n", "\n");

function extractFunction(source: string, name: string): string {
  const start = source.indexOf(`function ${name}`);
  if (start < 0) return "";
  const bodyStart = source.indexOf("{", start);
  let depth = 0;
  for (let index = bodyStart; index < source.length; index += 1) {
    const char = source[index];
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(start, index + 1);
    }
  }
  return "";
}

describe("queue UI wiring", () => {
  it("allows manual multi-file selection", () => {
    const onPickFile = extractFunction(appSource, "onPickFile");

    expect(onPickFile).toContain("multiple: true");
    expect(onPickFile).toContain("enqueueTranscriptionStartBatch");
  });

  it("does not clear active or queued jobs from Clear Finished", () => {
    expect(appSource).toContain("clearFinishedQueueItems(previous)");
    expect(appSource).not.toContain("onClick={() => setQueueItems([])}");
  });

  it("keeps completed jobs visible in the queue", () => {
    expect(appSource).toContain("markQueueItemTerminal(");
    expect(appSource).toContain('stage: "completed"');
    expect(appSource).toContain("const queueActiveItems = useMemo(\n    () => queueItems");
  });

  it("registers automatic-import jobs with queue metadata", () => {
    expect(appSource).toContain("registerAutomaticImportQueuedJobs");
    expect(appSource).toContain("sourceLabel: job.source_label");
    expect(appSource).toContain("sourceFolder: job.folder_path");
    expect(appSource).toContain("model: job.model");
    expect(appSource).toContain("language: job.language");
    expect(appSource).toContain("queue-meta-row");
  });

  it("keeps automatic-import queue registration in the background", () => {
    const registerAutomaticImportQueuedJobs = extractFunction(
      appSource,
      "registerAutomaticImportQueuedJobs",
    );

    expect(registerAutomaticImportQueuedJobs).not.toContain('setSection("queue")');
  });

  it("hydrates queue metadata from cross-window progress events", () => {
    expect(appSource).toContain("sourceLabel: event.source_label");
    expect(appSource).toContain("sourceFolder: event.source_folder");
    expect(appSource).toContain("inputPath: event.input_path");
    expect(appSource).toContain("workspaceId: event.workspace_id");
  });

  it("renders informative queue metadata and separate terminal counts", () => {
    expect(appSource).toContain("summarizeQueueItems(queueItems)");
    expect(appSource).toContain("queue.sourceAutomatic");
    expect(appSource).toContain("queue.translateToEnglishActive");
    expect(appSource).toContain("workspaceLabelMap.get(queueWorkspaceId)");
    expect(appSource).toContain("Completed {completed}");
    expect(appSource).toContain("Failed {failed}");
  });

  it("registers live sessions in the queue as soon as realtime starts", () => {
    const onStartRealtime = extractFunction(appSource, "onStartRealtime");

    expect(onStartRealtime).toContain("const realtimeJobId = startResult.job_id");
    expect(onStartRealtime).toContain('const liveEngine =');
    expect(onStartRealtime).toContain('? "whisper_cpp"');
    expect(onStartRealtime).toContain('engine: liveEngine');
    expect(onStartRealtime).toContain("const realtimeProgress: JobProgress");
    expect(onStartRealtime).toContain("setActiveRealtimeJobId(realtimeJobId)");
    expect(onStartRealtime).toContain("upsertQueueItem(previous, realtimeProgress)");
    expect(onStartRealtime).toContain('sourceOrigin: "realtime"');
  });

  it("moves live stop/save into the queue without hydrating the command response", () => {
    const onStopRealtime = extractFunction(appSource, "onStopRealtime");

    expect(onStopRealtime).toContain("result.queued");
    expect(onStopRealtime).toContain('stage: saveResult ? "persisting" : "cancelled"');
    expect(onStopRealtime).toContain('setSection("queue")');
    expect(onStopRealtime).not.toContain("hydrateDetail(result.artifact)");
  });

  it("routes live queue cards to the running live view or completed artifact", () => {
    const onFocusQueueJob = extractFunction(appSource, "onFocusQueueJob");

    expect(onFocusQueueJob).toContain("snapshot?.artifactId");
    expect(onFocusQueueJob).toContain("onOpenArtifact(snapshot.artifactId)");
    expect(onFocusQueueJob).toContain(
      "activeRealtimeJobIdRef.current === item.job_id",
    );
    expect(onFocusQueueJob).toContain("realtimeSessionOpen");
    expect(appSource).toContain("(!isTerminalQueueItem || Boolean(queueArtifactId))");
  });

  it("routes speaker detection directly to pyannote without retranscribing", () => {
    const onRunSpeakerDiarization = extractFunction(
      appSource,
      "onRunSpeakerDiarization",
    );

    expect(appSource).toContain("showSpeakerDetection={Boolean(");
    expect(appSource).toContain("detail.runSpeakerDetection");
    expect(appSource).toContain("detail.rerunSpeakerDetection");
    expect(appSource).toMatch(
      /onStartSpeakerDetection=\{\(\) =>\s+void onRunSpeakerDiarization\(\)\s+\}/,
    );
    expect(onRunSpeakerDiarization).toContain(
      "await runArtifactSpeakerDiarization(artifactId)",
    );
    expect(onRunSpeakerDiarization).not.toContain("onStartTranscription(");
    expect(onRunSpeakerDiarization).not.toContain("writeTrimmedAudio(");
  });

  it("registers live speaker detection before stop and consumes only the matching saved artifact", () => {
    const onToggleRealtimeSpeakerDetection = extractFunction(
      appSource,
      "onToggleRealtimeSpeakerDetection",
    );
    const onStopRealtime = extractFunction(appSource, "onStopRealtime");
    const renderRealtime = extractFunction(appSource, "renderRealtime");

    expect(onToggleRealtimeSpeakerDetection).toContain(
      "speaker_diarization?.enabled",
    );
    expect(onToggleRealtimeSpeakerDetection).toContain(
      "realtimeSpeakerDetectionRequestedRef.current",
    );
    const registrationIndex = onStopRealtime.indexOf(
      "registerRealtimeSpeakerDetectionBeforeStop(",
    );
    const stopInvocationIndex = onStopRealtime.indexOf("await stopRealtime(");
    const catchIndex = onStopRealtime.indexOf("catch (stopError)");
    const rollbackIndex = onStopRealtime.indexOf(
      "rollbackRealtimeSpeakerDetectionStop(",
    );
    expect(registrationIndex).toBeGreaterThan(-1);
    expect(stopInvocationIndex).toBeGreaterThan(registrationIndex);
    expect(catchIndex).toBeGreaterThan(stopInvocationIndex);
    expect(rollbackIndex).toBeGreaterThan(catchIndex);
    expect(appSource).toContain("subscribeRealtimeSaved((artifact) => {");
    expect(appSource).toContain("consumeRealtimeSavedSpeakerDetection(");
    expect(appSource).toContain("speakerDetection.matchedPendingJob");
    expect(appSource).toContain("if (!artifact.audio_available)");
    expect(appSource).toContain("runArtifactSpeakerDiarization(artifact.id)");
    expect(renderRealtime).toContain(
      "realtime-toolbar-button--speaker",
    );
    expect(renderRealtime).toContain('aria-pressed={realtimeSpeakerDetectionIsQueued}');
  });
});
