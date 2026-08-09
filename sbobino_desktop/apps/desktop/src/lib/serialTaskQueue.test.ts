import { describe, expect, it } from "vitest";

import { SerialTaskQueue } from "./serialTaskQueue";

describe("SerialTaskQueue", () => {
  it("runs settings mutations in submission order", async () => {
    const queue = new SerialTaskQueue();
    const events: string[] = [];
    let releaseFirst: () => void = () => undefined;
    const firstGate = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });

    const first = queue.enqueue(async () => {
      events.push("first:start");
      await firstGate;
      events.push("first:end");
    });
    const second = queue.enqueue(async () => {
      events.push("second");
    });

    await Promise.resolve();
    expect(events).toEqual(["first:start"]);
    releaseFirst();
    await Promise.all([first, second]);
    expect(events).toEqual(["first:start", "first:end", "second"]);
  });

  it("continues after a failed mutation", async () => {
    const queue = new SerialTaskQueue();
    const failed = queue.enqueue(async () => {
      throw new Error("save failed");
    });
    const recovered = queue.enqueue(async () => "saved");

    await expect(failed).rejects.toThrow("save failed");
    await expect(recovered).resolves.toBe("saved");
  });
});
