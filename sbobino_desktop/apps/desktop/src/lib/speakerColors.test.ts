import { describe, expect, it } from "vitest";
import {
  getDefaultSpeakerColorForKey,
  moveSpeakerColorMapEntry,
  normalizeSpeakerColorKey,
  removeSpeakerColorMapEntry,
  resolveSpeakerColor,
  sanitizeSpeakerColorMap,
  setSpeakerColorForKey,
} from "./speakerColors";

describe("speakerColors", () => {
  it("normalizes speaker keys consistently", () => {
    expect(normalizeSpeakerColorKey(" Speaker 1 ")).toBe("speaker_1");
    expect(normalizeSpeakerColorKey("")).toBe("speaker");
  });

  it("sanitizes custom speaker color maps", () => {
    expect(sanitizeSpeakerColorMap({
      " Speaker 1 ": "#ff00aa",
      invalid: "blue",
    })).toEqual({
      speaker_1: "#FF00AA",
    });
  });

  it("resolves configured colors before deterministic defaults", () => {
    expect(resolveSpeakerColor({
      speakerId: "speaker_1",
      colorMap: { speaker_1: "#123456" },
    })).toBe("#123456");

    expect(resolveSpeakerColor({
      speakerLabel: "Speaker 2",
      colorMap: {},
    })).toBe(getDefaultSpeakerColorForKey("speaker_2"));
  });

  it("does not use the rejected purple diarization default", () => {
    const defaultColors = Array.from({ length: 24 }, (_, index) =>
      getDefaultSpeakerColorForKey(`speaker_${index + 1}`),
    );

    expect(defaultColors).not.toContain("#B06BF2");
  });

  it("drops custom overrides when the chosen color matches the default", () => {
    const defaultColor = getDefaultSpeakerColorForKey("speaker_1");
    expect(setSpeakerColorForKey({ speaker_1: "#112233" }, "speaker_1", defaultColor)).toEqual({});
  });

  it("moves custom colors across speaker renames", () => {
    expect(moveSpeakerColorMapEntry(
      { speaker_1: "#ABCDEF" },
      "speaker_1",
      "Pietro",
    )).toEqual({
      pietro: "#ABCDEF",
    });
  });

  it("removes custom colors for merged-away speakers", () => {
    expect(removeSpeakerColorMapEntry(
      { speaker_1: "#ABCDEF", speaker_2: "#123456" },
      "speaker_1",
    )).toEqual({
      speaker_2: "#123456",
    });
  });
});
