import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
  beforeEach(() => {
    vi.mocked(readAudioFile).mockReset();
  });

  it("does not render without a source path", () => {
    const { container } = render(<AudioPlayer inputPath={null} />);
    expect(container.querySelector("footer.audio-player")).toBeNull();
  });

  it("loads fallback only after user presses play when primary source fails", () => {
    vi.mocked(readAudioFile).mockImplementation(() => new Promise<number[]>(() => {}));

    const { container } = render(<AudioPlayer inputPath="/tmp/sample.mp3" />);
    const audio = container.querySelector("audio");
    expect(audio).not.toBeNull();

    fireEvent.error(audio as HTMLAudioElement);
    expect(readAudioFile).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTitle("Play/Pause"));
    expect(readAudioFile).toHaveBeenCalledWith("/tmp/sample.mp3");
    expect(screen.getByText(/loading audio/i)).toBeInTheDocument();
  });
});
