import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AudioPlayer } from "./AudioPlayer";
import { readAudioFile, writeTrimmedAudio } from "../lib/tauri";
import { changeLanguage } from "../i18n";

vi.mock("@tauri-apps/api/core", () => ({
  isTauri: () => true,
  convertFileSrc: (path: string) => `asset://${path}`,
}));

vi.mock("../lib/tauri", () => ({
  readAudioFile: vi.fn(),
  writeTrimmedAudio: vi.fn(),
}));

describe("AudioPlayer", () => {
  const originalPlay = HTMLMediaElement.prototype.play;
  const originalPause = HTMLMediaElement.prototype.pause;
  const originalLoad = HTMLMediaElement.prototype.load;

  beforeEach(() => {
    changeLanguage("en");
    vi.mocked(readAudioFile).mockReset();
    vi.mocked(writeTrimmedAudio).mockReset();
    HTMLMediaElement.prototype.play = vi.fn().mockResolvedValue(undefined);
    HTMLMediaElement.prototype.pause = vi.fn();
    HTMLMediaElement.prototype.load = vi.fn();
  });

  afterEach(() => {
    changeLanguage("en");
    HTMLMediaElement.prototype.play = originalPlay;
    HTMLMediaElement.prototype.pause = originalPause;
    HTMLMediaElement.prototype.load = originalLoad;
  });

  it("does not render without a source path", () => {
    const { container } = render(<AudioPlayer inputPath={null} />);
    expect(container.querySelector("footer.audio-player")).toBeNull();
  });

  it("prefers the native Tauri file source for local audio", () => {
    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" />);
    const audio = container.querySelector("audio");
    expect(audio).not.toBeNull();

    expect(audio?.getAttribute("src")).toBe("asset:///tmp/sample.mp3");
    expect(readAudioFile).not.toHaveBeenCalled();
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

    expect(container.querySelector(".audio-trim-shell.is-open .audio-trim-header strong")?.textContent).toBe("Trim editor");
    expect(container.querySelector(".audio-trim-shell.is-open .trim-selection-status")?.textContent).toBe("No ranges yet");
  });

  it("localizes the trim editor copy based on the app language", () => {
    changeLanguage("it");

    const { container } = render(<AudioPlayer inputPath="https://example.com/sample.mp3" />);
    const audio = container.querySelector("audio");
    expect(audio).not.toBeNull();

    Object.defineProperty(audio as HTMLAudioElement, "duration", {
      configurable: true,
      value: 32,
    });

    fireEvent.loadedMetadata(audio as HTMLAudioElement);
    fireEvent.click(container.querySelector(".trim-toggle") as HTMLButtonElement);

    expect(container.querySelector(".audio-trim-shell.is-open .audio-trim-header strong")?.textContent).toBe("Editor ritaglio");
    expect(container.querySelector(".audio-trim-shell.is-open .trim-selection-status")?.textContent).toBe("Nessun intervallo ancora");
    expect(container.querySelector(".audio-trim-shell.is-open .audio-trim-header span")?.textContent).toContain("Trascina sulla forma d'onda");
  });

  it("formats long durations with hours instead of raw minutes", () => {
    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" trimEnabled={false} />);
    const audio = container.querySelector("audio") as HTMLAudioElement;

    Object.defineProperty(audio, "duration", {
      configurable: true,
      value: 7669,
    });
    Object.defineProperty(audio, "currentTime", {
      configurable: true,
      writable: true,
      value: 2053,
    });

    fireEvent.loadedMetadata(audio);
    fireEvent.timeUpdate(audio);

    expect(container.querySelector(".audio-time")?.textContent).toBe("34:13 / 02:07:49");
    expect(container.querySelector(".audio-time")?.getAttribute("title")).toBe("34 min 13 s / 2 h 7 min 49 s");
  });

  it("falls back to a blob source after a local media load error", async () => {
    vi.mocked(readAudioFile).mockResolvedValue([1, 2, 3, 4]);

    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" />);
    const audio = container.querySelector("audio") as HTMLAudioElement;
    expect(audio).not.toBeNull();

    expect(audio.getAttribute("src")).toBe("asset:///tmp/sample.mp3");
    fireEvent.error(audio);

    await vi.waitFor(() => {
      expect(readAudioFile).toHaveBeenCalledWith("/tmp/sample.mp3");
    });

    await vi.waitFor(() => {
      expect(audio.getAttribute("src")).not.toBe("asset:///tmp/sample.mp3");
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
    expect(readAudioFile).toHaveBeenCalledTimes(0);
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
        inputPath="/tmp/sample.mp3"
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

  it("applies the current trim ranges through the callback", async () => {
    vi.mocked(writeTrimmedAudio).mockResolvedValue({ path: "/tmp/trimmed-sample.mp3" });
    const onTrimApplied = vi.fn();

    const { container } = render(
      <AudioPlayer
        inputPath="/tmp/sample.mp3"
        initialTrimRegions={[
          { id: "region-1", startTime: 2, endTime: 6 },
          { id: "region-2", startTime: 12, endTime: 16 },
        ]}
        onTrimApplied={onTrimApplied}
      />,
    );
    const audio = container.querySelector("audio") as HTMLAudioElement;

    Object.defineProperty(audio, "duration", {
      configurable: true,
      value: 32,
    });

    fireEvent.loadedMetadata(audio);
    fireEvent.click(container.querySelector(".trim-toggle") as HTMLButtonElement);
    fireEvent.click(container.querySelector(".trim-apply-button") as HTMLButtonElement);

    await vi.waitFor(() => {
      expect(writeTrimmedAudio).toHaveBeenCalledWith("/tmp/sample.mp3", [
        { start: 2, end: 6 },
        { start: 12, end: 16 },
      ]);
    });

    expect(onTrimApplied).toHaveBeenCalledWith("/tmp/trimmed-sample.mp3", [
      { id: "region-1", startTime: 2, endTime: 6 },
      { id: "region-2", startTime: 12, endTime: 16 },
    ]);
  });
});
