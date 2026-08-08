import { describe, expect, it, vi } from "vitest";

import { runArtifactExportFlow } from "./artifactExportFlow";

describe("runArtifactExportFlow", () => {
  it("does not sync or write when the save dialog is cancelled", async () => {
    const syncArtifact = vi.fn();
    const writeArtifact = vi.fn();

    const result = await runArtifactExportFlow({
      artifact: { id: "a1", revision: 1 },
      chooseDestination: vi.fn().mockResolvedValue(null),
      syncArtifact,
      writeArtifact,
    });

    expect(result).toEqual({ status: "cancelled" });
    expect(syncArtifact).not.toHaveBeenCalled();
    expect(writeArtifact).not.toHaveBeenCalled();
  });

  it("chooses a destination before syncing and writes the synced artifact", async () => {
    const calls: string[] = [];
    const synced = { id: "a1", revision: 2 };

    const result = await runArtifactExportFlow({
      artifact: { id: "a1", revision: 1 },
      chooseDestination: async () => {
        calls.push("destination");
        return "/tmp/export.txt";
      },
      syncArtifact: async () => {
        calls.push("sync");
        return synced;
      },
      onArtifactSynced: (artifact) => {
        calls.push(`synced:${artifact.revision}`);
      },
      writeArtifact: async (artifact) => {
        calls.push(`write:${artifact.revision}`);
      },
    });

    expect(calls).toEqual(["destination", "sync", "synced:2", "write:2"]);
    expect(result).toEqual({
      status: "exported",
      artifact: synced,
      destination: "/tmp/export.txt",
    });
  });

  it("stops when synchronization no longer finds the artifact", async () => {
    const writeArtifact = vi.fn();
    const result = await runArtifactExportFlow({
      artifact: { id: "a1" },
      chooseDestination: vi.fn().mockResolvedValue("/tmp/export.txt"),
      syncArtifact: vi.fn().mockResolvedValue(null),
      writeArtifact,
    });

    expect(result).toEqual({ status: "sync_not_found" });
    expect(writeArtifact).not.toHaveBeenCalled();
  });

  it("identifies the failing phase without swallowing the original error", async () => {
    const diskError = new Error("disk unavailable");
    await expect(
      runArtifactExportFlow({
        artifact: { id: "a1" },
        chooseDestination: vi.fn().mockResolvedValue("/tmp/export.txt"),
        writeArtifact: vi.fn().mockRejectedValue(diskError),
      }),
    ).rejects.toMatchObject({
      phase: "write",
      originalError: diskError,
    });
  });
});
