import { Profiler } from "react";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { RealtimeInputLevelEvent } from "../types";
import { subscribeRealtimeInputLevel } from "../lib/tauri";
import { LiveMicrophoneWaveform } from "./LiveMicrophoneWaveform";

vi.mock("../lib/tauri", () => ({ subscribeRealtimeInputLevel: vi.fn() }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("coalesces microphone bursts without React renders and cleans up late subscriptions", async () => {
  let receive!: (event: RealtimeInputLevelEvent) => void;
  let finishSubscription!: (stop: () => void) => void;
  vi.mocked(subscribeRealtimeInputLevel).mockImplementation((callback) => {
    receive = callback;
    return new Promise((resolve) => { finishSubscription = resolve; });
  });
  const frames = new Map<number, FrameRequestCallback>();
  let nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback) => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id) => frames.delete(id)));
  const context = { clearRect: vi.fn(), setTransform: vi.fn(), beginPath: vi.fn(),
    roundRect: vi.fn(), fill: vi.fn(), fillRect: vi.fn(),
    createLinearGradient: () => ({ addColorStop: vi.fn() }),
    save: vi.fn(), restore: vi.fn(), setLineDash: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn() };
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(context as unknown as CanvasRenderingContext2D);
  const commits = vi.fn();
  const view = render(<Profiler id="waveform" onRender={commits}>
    <LiveMicrophoneWaveform ariaLabel="Microphone" mode="running" previewState="running"
      elapsedSeconds={1} runningLabel="Live" pausedLabel="Paused" idleStatusLabel="Idle"
      idleLabel="Idle" connectingLabel="Connecting" blockedLabel="Blocked" unavailableLabel="Unavailable" />
  </Profiler>);
  const canvas = view.container.querySelector("canvas")!;
  Object.defineProperties(canvas, { clientWidth: { value: 800 }, clientHeight: { value: 144 } });
  const initialCommits = commits.mock.calls.length;
  act(() => {
    for (let i = 0; i < 200; i++) receive({ state: "running", level: i === 199 ? NaN : 0.5, message: "" });
  });
  expect(commits).toHaveBeenCalledTimes(initialCommits);
  expect(frames.size).toBe(1);
  act(() => { const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach((callback) => callback(0)); });
  expect(context.clearRect).toHaveBeenCalledTimes(1);
  expect(context.roundRect).toHaveBeenCalledTimes(160);
  expect(context.roundRect.mock.calls.flat().every(Number.isFinite)).toBe(true);
  view.rerender(<Profiler id="waveform" onRender={commits}>
    <LiveMicrophoneWaveform ariaLabel="Microphone" mode="paused" previewState="paused"
      elapsedSeconds={2} runningLabel="Live" pausedLabel="Paused" idleStatusLabel="Idle"
      idleLabel="Idle" connectingLabel="Connecting" blockedLabel="Blocked" unavailableLabel="Unavailable" />
  </Profiler>);
  act(() => receive({ state: "paused", level: 0.8, message: "" }));
  expect(frames.size).toBe(1);
  act(() => { const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach((callback) => callback(0)); });
  expect(context.roundRect).toHaveBeenCalledTimes(320);
  const stop = vi.fn();
  view.unmount();
  await act(async () => finishSubscription(stop));
  expect(stop).toHaveBeenCalledOnce();
  receive({ state: "running", level: 0.5, message: "" });
  expect(frames.size).toBe(0);
});
