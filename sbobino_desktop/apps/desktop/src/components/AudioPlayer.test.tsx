import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AudioPlayer } from "./AudioPlayer";
import { readAudioFile } from "../lib/tauri";

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  convertFileSrc: (path: string) => `asset://${path}`,
}));

vi.mock("../lib/tauri", () => ({
  readAudioFile: vi.fn(),
}));

describe("AudioPlayer", () => {
  const originalPlay = HTMLMediaElement.prototype.play;
  const originalPause = HTMLMediaElement.prototype.pause;
  const originalLoad = HTMLMediaElement.prototype.load;

  beforeEach(() => {
    vi.mocked(readAudioFile).mockReset();
    HTMLMediaElement.prototype.play = vi.fn().mockResolvedValue(undefined);
    HTMLMediaElement.prototype.pause = vi.fn();
    HTMLMediaElement.prototype.load = vi.fn();
  });

  afterEach(() => {
    HTMLMediaElement.prototype.play = originalPlay;
    HTMLMediaElement.prototype.pause = originalPause;
    HTMLMediaElement.prototype.load = originalLoad;
  });

  it("does not render without a source path", () => {
    const { container } = render(<AudioPlayer inputPath={null} />);
    expect(container.querySelector("footer.audio-player")).toBeNull();
  });

  it("preloads local audio in background before the first play", async () => {
    vi.mocked(readAudioFile).mockImplementation(() => new Promise<number[]>(() => {}));

    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" />);
    const audio = container.querySelector("audio");
    expect(audio).not.toBeNull();

    await vi.waitFor(() => {
      expect(readAudioFile).toHaveBeenCalledWith("/tmp/sample.mp3");
    });
  });

  it("shows the trim editor once metadata is available and trim mode is enabled", () => {
    const { container } = render(<AudioPlayer inputPath="https://example.com/sample.mp3" />);
    const audio = container.querySelector("audio");
    expect(audio).not.toBeNull();

    Object.defineProperty(audio as HTMLAudioElement, "duration", {
      configurable: true,
      value: 32,
    });

    fireEvent.loadedMetadata(audio as HTMLAudioElement);
    fireEvent.click(container.querySelector(".trim-toggle") as HTMLButtonElement);

    expect(screen.getByText("Trim editor")).toBeInTheDocument();
    expect(screen.getByText("No ranges yet")).toBeInTheDocument();
  });

  it("uses the first play click after a local preload finishes", async () => {
    let resolveAudio: ((value: number[]) => void) | undefined;
    vi.mocked(readAudioFile).mockImplementation(
      () => new Promise<number[]>((resolve) => { resolveAudio = resolve; }),
    );

    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" />);
    const audio = container.querySelector("audio") as HTMLAudioElement;
    expect(audio).not.toBeNull();

    fireEvent.click(container.querySelector(".playback-button") as HTMLButtonElement);
    expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();

    if (resolveAudio) {
      resolveAudio([1, 2, 3, 4]);
    }

    await vi.waitFor(() => {
      expect(audio.getAttribute("src")).toBeTruthy();
    });

    fireEvent.loadedMetadata(audio);
    fireEvent.loadedData(audio);

    await vi.waitFor(() => {
      expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(1);
    });
  });

  it("does not restart local loading when the metadata callback identity changes", async () => {
    vi.mocked(readAudioFile).mockResolvedValue([1, 2, 3, 4]);

    const { container, rerender } = render(
      <AudioPlayer inputPath="/tmp/sample.mp3" onMetadataLoaded={() => undefined} />,
    );

    const audio = container.querySelector("audio") as HTMLAudioElement;

    await vi.waitFor(() => {
      expect(audio.getAttribute("src")).toBeTruthy();
    });

    const initialSrc = audio.getAttribute("src");

    rerender(<AudioPlayer inputPath="/tmp/sample.mp3" onMetadataLoaded={() => undefined} />);

    expect(audio.getAttribute("src")).toBe(initialSrc);
    expect(readAudioFile).toHaveBeenCalledTimes(1);
  });

  it("can rerender from a valid source to no source without crashing hooks", async () => {
    vi.mocked(readAudioFile).mockResolvedValue([1, 2, 3, 4]);

    const { container, rerender } = render(<AudioPlayer inputPath="/tmp/sample.mp3" />);

    await vi.waitFor(() => {
      expect(container.querySelector("audio")?.getAttribute("src")).toBeTruthy();
    });

    expect(() => {
      rerender(<AudioPlayer inputPath={null} />);
    }).not.toThrow();

    expect(container.querySelector("footer.audio-player")).toBeNull();
  });

  it("can render as a playback-only preview without trim controls", () => {
    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" trimEnabled={false} />);
    expect(container.querySelector(".trim-toggle")).toBeNull();
    expect(container.querySelector(".audio-slider")).not.toBeNull();
  });

  it("plays selected trim ranges in sequence from the first range by default", async () => {
    const { container } = render(
      <AudioPlayer
        inputPath="https://example.com/sample.mp3"
        initialTrimRegions={[
          { id: "region-1", startTime: 2, endTime: 6 },
          { id: "region-2", startTime: 12, endTime: 16 },
        ]}
      />,
    );
    const audio = container.querySelector("audio") as HTMLAudioElement;

    Object.defineProperty(audio, "duration", {
      configurable: true,
      value: 32,
    });
    Object.defineProperty(audio, "currentTime", {
      configurable: true,
      writable: true,
      value: 0,
    });

    fireEvent.loadedMetadata(audio);
    fireEvent.click(container.querySelector(".trim-toggle") as HTMLButtonElement);

    expect(screen.getByText("2 ranges")).toBeInTheDocument();

    fireEvent.click(container.querySelector(".playback-button") as HTMLButtonElement);
    expect(audio.currentTime).toBeCloseTo(2, 3);

    audio.currentTime = 6;
    fireEvent.timeUpdate(audio);
    expect(audio.currentTime).toBeCloseTo(12, 3);

    audio.currentTime = 16;
    fireEvent.timeUpdate(audio);
    expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();
    expect(audio.currentTime).toBeCloseTo(16, 3);
  });
});
