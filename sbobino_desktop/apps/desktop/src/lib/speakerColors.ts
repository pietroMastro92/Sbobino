const SPEAKER_COLOR_PALETTE = [
  "#4F7CFF",
  "#EC6A5E",
  "#27A376",
  "#D88B15",
  "#1293A5",
  "#C85F39",
  "#5F8D3D",
  "#B04A64",
  "#6C7A2D",
];

const HEX_COLOR_PATTERN = /^#[0-9a-fA-F]{6}$/;

export function normalizeSpeakerColorKey(value: string | null | undefined): string {
  if (typeof value !== "string") {
    return "";
  }

  const candidate = value
    .toLowerCase()
    .trim()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");

  return candidate.length > 0 ? candidate : "speaker";
}

function normalizeSpeakerColorValue(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }

  const trimmed = value.trim();
  if (!HEX_COLOR_PATTERN.test(trimmed)) {
    return null;
  }

  return trimmed.toUpperCase();
}

function hashSpeakerColorKey(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = ((hash << 5) - hash) + value.charCodeAt(index);
    hash |= 0;
  }
  return Math.abs(hash);
}

export function getDefaultSpeakerColorForKey(key: string | null | undefined): string {
  const normalizedKey = normalizeSpeakerColorKey(key);
  const paletteIndex = hashSpeakerColorKey(normalizedKey) % SPEAKER_COLOR_PALETTE.length;
  return SPEAKER_COLOR_PALETTE[paletteIndex];
}

export function sanitizeSpeakerColorMap(
  value: Record<string, string> | null | undefined,
): Record<string, string> {
  if (!value || typeof value !== "object") {
    return {};
  }

  return Object.entries(value).reduce<Record<string, string>>((accumulator, [rawKey, rawColor]) => {
    const normalizedKey = normalizeSpeakerColorKey(rawKey);
    const normalizedColor = normalizeSpeakerColorValue(rawColor);
    if (!normalizedKey || !normalizedColor) {
      return accumulator;
    }
    accumulator[normalizedKey] = normalizedColor;
    return accumulator;
  }, {});
}

export function resolveSpeakerColor(params: {
  speakerId?: string | null;
  speakerLabel?: string | null;
  colorMap?: Record<string, string> | null;
}): string | null {
  const keys = [
    normalizeSpeakerColorKey(params.speakerId),
    normalizeSpeakerColorKey(params.speakerLabel),
  ].filter((value, index, values): value is string => Boolean(value) && values.indexOf(value) === index);

  if (keys.length === 0) {
    return null;
  }

  const colorMap = sanitizeSpeakerColorMap(params.colorMap ?? {});
  for (const key of keys) {
    const configured = colorMap[key];
    if (configured) {
      return configured;
    }
  }

  return getDefaultSpeakerColorForKey(keys[0]);
}

export function setSpeakerColorForKey(
  colorMap: Record<string, string> | null | undefined,
  key: string,
  nextColor: string,
): Record<string, string> {
  const normalizedKey = normalizeSpeakerColorKey(key);
  const normalizedColor = normalizeSpeakerColorValue(nextColor);
  const nextMap = sanitizeSpeakerColorMap(colorMap ?? {});

  if (!normalizedKey || !normalizedColor) {
    return nextMap;
  }

  if (normalizedColor === getDefaultSpeakerColorForKey(normalizedKey)) {
    delete nextMap[normalizedKey];
    return nextMap;
  }

  nextMap[normalizedKey] = normalizedColor;
  return nextMap;
}

export function moveSpeakerColorMapEntry(
  colorMap: Record<string, string> | null | undefined,
  previousKey: string | null | undefined,
  nextKey: string | null | undefined,
): Record<string, string> {
  const normalizedPreviousKey = normalizeSpeakerColorKey(previousKey);
  const normalizedNextKey = normalizeSpeakerColorKey(nextKey);
  const nextMap = sanitizeSpeakerColorMap(colorMap ?? {});

  if (!normalizedPreviousKey || !normalizedNextKey || normalizedPreviousKey === normalizedNextKey) {
    return nextMap;
  }

  const currentColor = nextMap[normalizedPreviousKey];
  if (!currentColor) {
    return nextMap;
  }

  delete nextMap[normalizedPreviousKey];
  nextMap[normalizedNextKey] = currentColor;
  return nextMap;
}

export function removeSpeakerColorMapEntry(
  colorMap: Record<string, string> | null | undefined,
  key: string | null | undefined,
): Record<string, string> {
  const normalizedKey = normalizeSpeakerColorKey(key);
  const nextMap = sanitizeSpeakerColorMap(colorMap ?? {});

  if (!normalizedKey) {
    return nextMap;
  }

  delete nextMap[normalizedKey];
  return nextMap;
}
