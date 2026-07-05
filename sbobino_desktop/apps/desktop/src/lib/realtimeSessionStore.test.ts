import { describe, expect, it } from "vitest";
import { RealtimeSessionStore } from "./realtimeSessionStore";

describe("RealtimeSessionStore", () => {
  it("keeps live deltas outside React and notifies subscribers once per delta", () => {
    const store = new RealtimeSessionStore();
    let notifications = 0;
    store.subscribe(() => { notifications += 1; });
    store.applyDelta({ kind: "append_final", text: "prima" }, null);
    store.applyDelta({ kind: "update_preview", text: "seconda" }, null);
    expect(store.transcript()).toBe("prima\nseconda");
    expect(notifications).toBe(2);
  });

  it("replaces only the latest final line and exposes a live timeline", () => {
    const store = new RealtimeSessionStore();
    const segment = {
      text: "corretto",
      start_seconds: 0,
      end_seconds: 1,
      words: [],
    };
    store.applyDelta({ kind: "append_final", text: "provvisorio" }, null);
    store.applyDelta({ kind: "replace_final", text: "corretto" }, segment);
    expect(store.transcript(false)).toBe("corretto");
    expect(store.timelineJson()).toContain("corretto");
  });
});
