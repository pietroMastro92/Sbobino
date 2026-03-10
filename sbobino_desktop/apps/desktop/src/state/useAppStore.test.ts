import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./useAppStore";
import type { TranscriptArtifact } from "../types";

const artifact = (id: string, title: string): TranscriptArtifact => ({
  id,
  job_id: `job-${id}`,
  title,
  kind: "file",
  input_path: `/tmp/${title}.wav`,
  raw_transcript: "raw",
  optimized_transcript: "optimized",
  summary: "",
  faqs: "",
  metadata: {},
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
});

describe("useAppStore", () => {
  beforeEach(() => {
    useAppStore.setState({
      artifacts: [],
      error: null,
      activeJobId: null,
      progress: null,
      selectedFile: null,
    });
  });

  it("tracks active job lifecycle", () => {
    const store = useAppStore.getState();
    store.setJobStarted("job-123");
    expect(useAppStore.getState().activeJobId).toBe("job-123");

    store.clearActiveJob();
    expect(useAppStore.getState().activeJobId).toBeNull();
  });

  it("prepends, upserts and removes artifacts", () => {
    const store = useAppStore.getState();
    store.setArtifacts([]);

    const first = artifact("a1", "first");
    const second = artifact("a2", "second");
    store.prependArtifact(first);
    store.prependArtifact(second);
    expect(useAppStore.getState().artifacts.map((item) => item.id)).toEqual(["a2", "a1"]);

    store.upsertArtifact({ ...first, title: "first-updated" });
    expect(useAppStore.getState().artifacts.find((item) => item.id === "a1")?.title).toBe(
      "first-updated",
    );

    store.removeArtifacts(["a2"]);
    expect(useAppStore.getState().artifacts.map((item) => item.id)).toEqual(["a1"]);
  });

  it("deduplicates a trim artifact when the same id is prepended twice", () => {
    const store = useAppStore.getState();
    const parent = artifact("parent-1", "parent");
    const trim = {
      ...artifact("trim-1", "parent - Trim 03:14-06:38"),
      metadata: { parent_id: parent.id },
    };

    store.setArtifacts([parent]);
    store.prependArtifact(trim);
    store.prependArtifact({ ...trim, updated_at: "2026-01-01T00:01:00Z" });

    expect(useAppStore.getState().artifacts.map((item) => item.id)).toEqual(["trim-1", "parent-1"]);
    expect(useAppStore.getState().artifacts.filter((item) => item.id === "trim-1")).toHaveLength(1);
  });

  it("drops duplicate artifact ids received through setArtifacts", () => {
    const store = useAppStore.getState();
    const parent = artifact("parent-1", "parent");
    const trim = {
      ...artifact("trim-1", "parent - Trim 03:14-06:38"),
      metadata: { parent_id: parent.id },
    };

    store.setArtifacts([parent, trim, trim]);

    expect(useAppStore.getState().artifacts.map((item) => item.id)).toEqual(["parent-1", "trim-1"]);
  });
});
