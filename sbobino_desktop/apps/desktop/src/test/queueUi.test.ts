import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const appSource = fs.readFileSync(path.resolve(currentDir, "../App.tsx"), "utf8");

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
});
