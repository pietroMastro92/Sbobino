import { describe, expect, it } from "vitest";
import { RealtimeLevelBuffer } from "./realtimeLevelBuffer";

describe("RealtimeLevelBuffer", () => {
  it("keeps the newest clamped values in chronological order", () => {
    const buffer = new RealtimeLevelBuffer(3);
    [-1, 0.25, 0.75, 2].forEach((value) => buffer.push(value));
    expect(buffer.snapshot()).toEqual([0.25, 0.75, 1]);
  });

  it("clears samples without replacing the stable buffer object", () => {
    const buffer = new RealtimeLevelBuffer(2);
    buffer.push(0.5);
    buffer.clear();
    expect(buffer.snapshot()).toEqual([]);
    buffer.push(0.8);
    expect(buffer.snapshot()).toHaveLength(1);
    expect(buffer.snapshot()[0]).toBeCloseTo(0.8);
  });

  it("consumes each microphone envelope bin exactly once", () => {
    const buffer = new RealtimeLevelBuffer(4);
    buffer.pushEnvelope([
      { min: -0.2, max: 0.4 },
      { min: -0.7, max: 0.6 },
    ]);

    const first = buffer.readSince(0);
    expect(first.bins).toHaveLength(2);
    expect(first.bins[0]?.min).toBeCloseTo(-0.2);
    expect(first.bins[0]?.max).toBeCloseTo(0.4);
    expect(first.bins[1]?.min).toBeCloseTo(-0.7);
    expect(first.bins[1]?.max).toBeCloseTo(0.6);
    expect(buffer.readSince(first.sequence).bins).toEqual([]);

    buffer.pushEnvelope([{ min: -0.1, max: 0.3 }]);
    const next = buffer.readSince(first.sequence).bins;
    expect(next).toHaveLength(1);
    expect(next[0]?.min).toBeCloseTo(-0.1);
    expect(next[0]?.max).toBeCloseTo(0.3);
  });

  it("notifies the canvas once per envelope and stops after unsubscribe", () => {
    const buffer = new RealtimeLevelBuffer(8);
    let notifications = 0;
    const unsubscribe = buffer.subscribe(() => { notifications += 1; });
    buffer.pushEnvelope([
      { min: -0.1, max: 0.2 },
      { min: -0.4, max: 0.5 },
      { min: -0.2, max: 0.3 },
    ]);
    expect(notifications).toBe(1);
    unsubscribe();
    buffer.push(0.8);
    expect(notifications).toBe(1);
  });
});
