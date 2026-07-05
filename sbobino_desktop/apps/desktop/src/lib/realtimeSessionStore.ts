import type { RealtimeDelta, TimelineV2Segment } from "../types";

export type RealtimeSessionSnapshot = {
  version: number;
  finalLines: readonly string[];
  preview: string;
  segments: readonly TimelineV2Segment[];
  previewSegment: TimelineV2Segment | null;
};

const EMPTY_SNAPSHOT: RealtimeSessionSnapshot = {
  version: 0,
  finalLines: [],
  preview: "",
  segments: [],
  previewSegment: null,
};

export class RealtimeSessionStore {
  private snapshot: RealtimeSessionSnapshot = EMPTY_SNAPSHOT;
  private readonly listeners = new Set<() => void>();

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): RealtimeSessionSnapshot => this.snapshot;

  reset(): void {
    this.replace({ finalLines: [], preview: "", segments: [], previewSegment: null });
  }

  applyDelta(delta: RealtimeDelta, segment: TimelineV2Segment | null): void {
    if (delta.kind === "append_final") {
      this.replace({
        finalLines: [...this.snapshot.finalLines, delta.text],
        preview: "",
        segments: segment ? [...this.snapshot.segments, segment] : this.snapshot.segments,
        previewSegment: segment ? null : this.snapshot.previewSegment,
      });
      return;
    }
    if (delta.kind === "replace_final") {
      const finalLines = this.snapshot.finalLines.length === 0
        ? [delta.text]
        : [...this.snapshot.finalLines.slice(0, -1), delta.text];
      const segments = segment
        ? (this.snapshot.segments.length === 0
          ? [segment]
          : [...this.snapshot.segments.slice(0, -1), segment])
        : this.snapshot.segments;
      this.replace({ finalLines, preview: "", segments, previewSegment: segment ? null : this.snapshot.previewSegment });
      return;
    }
    this.replace({ preview: delta.text, previewSegment: segment });
  }

  transcript(includePreview = true): string {
    const lines = [...this.snapshot.finalLines];
    if (includePreview && this.snapshot.preview.trim()) lines.push(this.snapshot.preview.trim());
    return lines.filter((line) => line.trim()).join("\n");
  }

  timelineJson(): string | null {
    const segments = this.snapshot.previewSegment
      ? [...this.snapshot.segments, this.snapshot.previewSegment]
      : this.snapshot.segments;
    return segments.length > 0 ? JSON.stringify({ version: 2, segments }) : null;
  }

  private replace(patch: Partial<Omit<RealtimeSessionSnapshot, "version">>): void {
    this.snapshot = { ...this.snapshot, ...patch, version: this.snapshot.version + 1 };
    this.listeners.forEach((listener) => listener());
  }
}
