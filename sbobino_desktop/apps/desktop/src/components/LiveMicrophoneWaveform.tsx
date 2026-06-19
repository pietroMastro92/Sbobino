import { useEffect, useRef } from "react";

type PreviewState = "idle" | "connecting" | "running" | "paused" | "blocked" | "unavailable";

type LiveMicrophoneWaveformProps = {
  ariaLabel: string;
  mode: "idle" | "running" | "paused";
  previewState: PreviewState;
  levels: number[];
  elapsedSeconds: number;
  runningLabel: string;
  pausedLabel: string;
  idleStatusLabel: string;
  idleLabel: string;
  connectingLabel: string;
  blockedLabel: string;
  unavailableLabel: string;
};

const BAR_WIDTH = 3;
const BAR_GAP = 2;
const BAR_RADIUS = 999;
const BAR_HEIGHT = 4;
const FADE_WIDTH = 28;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function formatElapsedTimestamp(seconds: number): string {
  const safe = Math.max(0, seconds);
  const minutes = Math.floor(safe / 60);
  const wholeSeconds = Math.floor(safe % 60);
  return `${String(minutes).padStart(2, "0")}:${String(wholeSeconds).padStart(2, "0")}`;
}

function ensureCanvasSize(canvas: HTMLCanvasElement): { width: number; height: number; context: CanvasRenderingContext2D | null } {
  const context = canvas.getContext("2d");
  if (!context) {
    return { width: 0, height: 0, context: null };
  }

  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  if (width <= 0 || height <= 0) {
    return { width, height, context };
  }

  const dpr = window.devicePixelRatio || 1;
  const targetWidth = Math.round(width * dpr);
  const targetHeight = Math.round(height * dpr);
  if (canvas.width !== targetWidth || canvas.height !== targetHeight) {
    canvas.width = targetWidth;
    canvas.height = targetHeight;
  }

  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { width, height, context };
}

function drawIdleBaseline(context: CanvasRenderingContext2D, width: number, height: number): void {
  context.save();
  context.strokeStyle = "rgba(110, 140, 186, 0.18)";
  context.lineWidth = 2;
  context.setLineDash([2.5, 4.5]);
  context.beginPath();
  context.moveTo(0, height / 2);
  context.lineTo(width, height / 2);
  context.stroke();
  context.restore();
}

function drawPlaceholder(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  color: string,
): void {
  const placeholderCount = Math.max(18, Math.floor(width / 14));
  const center = placeholderCount / 2;
  for (let index = 0; index < placeholderCount; index += 1) {
    const distance = Math.abs(index - center) / Math.max(1, center);
    const amplitude = 0.18 + (1 - distance) * 0.26;
    const barHeight = Math.max(BAR_HEIGHT, amplitude * height * 0.7);
    const x = index * ((width - BAR_WIDTH) / placeholderCount);
    const y = (height - barHeight) / 2;
    context.fillStyle = color;
    context.globalAlpha = 0.16 + amplitude * 0.22;
    context.beginPath();
    context.roundRect(x, y, BAR_WIDTH, barHeight, BAR_RADIUS);
    context.fill();
  }
  context.globalAlpha = 1;
}

function drawWaveform(
  canvas: HTMLCanvasElement,
  levels: number[],
  previewState: PreviewState,
  mode: "idle" | "running" | "paused",
): void {
  const { width, height, context } = ensureCanvasSize(canvas);
  if (!context || width <= 0 || height <= 0) {
    return;
  }

  context.clearRect(0, 0, width, height);

  const computedBarColor = getComputedStyle(canvas).getPropertyValue("--live-waveform-bar").trim() || "#5c8fdb";
  const step = BAR_WIDTH + BAR_GAP;
  const barCount = Math.max(1, Math.floor(width / step));
  const visibleLevels = levels.slice(-barCount);

  if (visibleLevels.length === 0) {
    drawIdleBaseline(context, width, height);
    if (previewState === "connecting") {
      drawPlaceholder(context, width, height, computedBarColor);
    }
    return;
  }

  const centerY = height / 2;
  const paused = mode === "paused";
  const alphaBase = paused ? 0.28 : 0.38;
  const alphaSpread = paused ? 0.32 : 0.52;

  for (let index = 0; index < visibleLevels.length; index += 1) {
    const dataIndex = visibleLevels.length - 1 - index;
    const rawValue = visibleLevels[dataIndex] ?? 0.05;
    const value = clamp(rawValue, 0.02, 1);
    const x = width - (index + 1) * step;
    const barHeight = Math.max(BAR_HEIGHT, value * height * 0.9);
    const y = centerY - barHeight / 2;

    context.fillStyle = computedBarColor;
    context.globalAlpha = alphaBase + value * alphaSpread;
    context.beginPath();
    context.roundRect(x, y, BAR_WIDTH, barHeight, BAR_RADIUS);
    context.fill();
  }

  if (FADE_WIDTH > 0 && width > 0) {
    const fadePercent = Math.min(0.3, FADE_WIDTH / width);
    const gradient = context.createLinearGradient(0, 0, width, 0);
    gradient.addColorStop(0, "rgba(255,255,255,1)");
    gradient.addColorStop(fadePercent, "rgba(255,255,255,0)");
    gradient.addColorStop(1 - fadePercent, "rgba(255,255,255,0)");
    gradient.addColorStop(1, "rgba(255,255,255,1)");

    context.globalCompositeOperation = "destination-out";
    context.fillStyle = gradient;
    context.fillRect(0, 0, width, height);
    context.globalCompositeOperation = "source-over";
  }

  context.globalAlpha = 1;
}

export function LiveMicrophoneWaveform({
  ariaLabel,
  mode,
  previewState,
  levels,
  elapsedSeconds,
  runningLabel,
  pausedLabel,
  idleStatusLabel,
  idleLabel,
  connectingLabel,
  blockedLabel,
  unavailableLabel,
}: LiveMicrophoneWaveformProps): JSX.Element {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) {
      return;
    }
    drawWaveform(canvas, levels, previewState, mode);
  }, [levels, previewState, mode]);

  const overlayLabel = (() => {
    if (mode === "paused") return pausedLabel;
    switch (previewState) {
      case "connecting":
        return connectingLabel;
      case "blocked":
        return blockedLabel;
      case "unavailable":
        return unavailableLabel;
      case "idle":
        return idleLabel;
      default:
        return null;
    }
  })();

  const statusLabel = mode === "running"
    ? runningLabel
    : mode === "paused"
      ? pausedLabel
      : idleStatusLabel;

  return (
    <section className="live-waveform-panel" aria-label={ariaLabel}>
      <div className="audio-waveform-track live-waveform-track">
        <canvas ref={canvasRef} className="audio-waveform-canvas" />
        {overlayLabel ? <div className="audio-waveform-hint live-waveform-hint">{overlayLabel}</div> : null}
      </div>
      <div className="live-waveform-footer">
        <span className={`live-waveform-status-badge ${mode === "running" ? "running" : mode === "paused" ? "paused" : "idle"}`}>
          {statusLabel}
        </span>
        <strong className="live-waveform-elapsed">{formatElapsedTimestamp(elapsedSeconds)}</strong>
      </div>
    </section>
  );
}
