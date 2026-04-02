import { useEffect, useRef, useState } from "react";

type LiveMicrophoneWaveformProps = {
  mode: "idle" | "running" | "paused";
  elapsedSeconds: number;
  runningLabel: string;
  pausedLabel: string;
  idleStatusLabel: string;
  idleLabel: string;
  connectingLabel: string;
  blockedLabel: string;
  unavailableLabel: string;
};

type PreviewState = "idle" | "connecting" | "running" | "blocked" | "unavailable";

const BAR_WIDTH = 3;
const BAR_GAP = 2;
const BAR_RADIUS = 999;
const BAR_HEIGHT = 4;
const FADE_WIDTH = 28;
const UPDATE_RATE_MS = 30;
const FFT_SIZE = 256;
const SMOOTHING = 0.8;
const SENSITIVITY = 1;

function createAudioContextInstance(): AudioContext | null {
  const AudioContextCtor = window.AudioContext
    ?? (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  return AudioContextCtor ? new AudioContextCtor() : null;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function formatElapsedTimestamp(seconds: number): string {
  const safe = Math.max(0, seconds);
  const minutes = Math.floor(safe / 60);
  const wholeSeconds = Math.floor(safe % 60);
  return `${String(minutes).padStart(2, "0")}:${String(wholeSeconds).padStart(2, "0")}`;
}

function averageRelevantBands(samples: Uint8Array): number {
  if (samples.length === 0) {
    return 0;
  }

  const start = Math.floor(samples.length * 0.05);
  const end = Math.max(start + 1, Math.floor(samples.length * 0.4));
  let sum = 0;
  for (let index = start; index < end; index += 1) {
    sum += samples[index] ?? 0;
  }

  const average = sum / Math.max(1, end - start);
  return clamp((average / 255) * SENSITIVITY, 0.05, 1);
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

function drawWaveform(
  canvas: HTMLCanvasElement,
  history: number[],
  state: PreviewState,
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
  const visibleHistory = history.slice(-barCount);

  if (visibleHistory.length === 0) {
    drawIdleBaseline(context, width, height);

    if (state === "connecting") {
      const placeholderCount = Math.max(18, Math.floor(width / 14));
      const center = placeholderCount / 2;
      for (let index = 0; index < placeholderCount; index += 1) {
        const distance = Math.abs(index - center) / Math.max(1, center);
        const amplitude = 0.18 + (1 - distance) * 0.26;
        const barHeight = Math.max(BAR_HEIGHT, amplitude * height * 0.7);
        const x = index * ((width - BAR_WIDTH) / placeholderCount);
        const y = (height - barHeight) / 2;
        context.fillStyle = computedBarColor;
        context.globalAlpha = 0.16 + amplitude * 0.22;
        context.beginPath();
        context.roundRect(x, y, BAR_WIDTH, barHeight, BAR_RADIUS);
        context.fill();
      }
      context.globalAlpha = 1;
    }

    return;
  }

  const centerY = height / 2;
  const paused = mode === "paused";
  const alphaBase = paused ? 0.28 : 0.38;
  const alphaSpread = paused ? 0.32 : 0.52;

  for (let index = 0; index < visibleHistory.length; index += 1) {
    const dataIndex = visibleHistory.length - 1 - index;
    const value = clamp(visibleHistory[dataIndex] ?? 0.05, 0.05, 1);
    const x = width - (index + 1) * step;
    const barHeight = Math.max(BAR_HEIGHT, value * height * 0.8);
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
  mode,
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
  const frameRef = useRef<number | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);
  const sourceRef = useRef<MediaStreamAudioSourceNode | null>(null);
  const analyserRef = useRef<AnalyserNode | null>(null);
  const samplesRef = useRef<Uint8Array | null>(null);
  const historyRef = useRef<number[]>([]);
  const previewStateRef = useRef<PreviewState>("idle");
  const lastSampleAtRef = useRef(0);
  const [previewState, setPreviewState] = useState<PreviewState>("idle");

  const updatePreviewState = (next: PreviewState): void => {
    previewStateRef.current = next;
    setPreviewState(next);
  };

  const stopAnimation = (): void => {
    if (frameRef.current !== null) {
      window.cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
  };

  const stopPreview = (): void => {
    stopAnimation();
    sourceRef.current?.disconnect();
    sourceRef.current = null;
    analyserRef.current?.disconnect();
    analyserRef.current = null;
    samplesRef.current = null;
    streamRef.current?.getTracks().forEach((track) => track.stop());
    streamRef.current = null;
    if (audioContextRef.current) {
      void audioContextRef.current.close();
      audioContextRef.current = null;
    }
  };

  useEffect(() => {
    return () => {
      stopPreview();
      historyRef.current = [];
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    drawWaveform(canvas, historyRef.current, previewState, mode);
  }, [mode, previewState]);

  useEffect(() => {
    let cancelled = false;

    if (mode === "idle") {
      stopPreview();
      historyRef.current = [];
      updatePreviewState("idle");
      const canvas = canvasRef.current;
      if (canvas) {
        drawWaveform(canvas, [], "idle", "idle");
      }
      return;
    }

    if (!navigator.mediaDevices?.getUserMedia) {
      updatePreviewState("unavailable");
      return;
    }

    const renderFrame = (timestamp: number): void => {
      const canvas = canvasRef.current;
      const analyser = analyserRef.current;
      const samples = samplesRef.current;
      if (!canvas || !analyser || !samples) {
        return;
      }

      if (mode === "running" && timestamp - lastSampleAtRef.current >= UPDATE_RATE_MS) {
        lastSampleAtRef.current = timestamp;
        analyser.getByteFrequencyData(samples);
        historyRef.current.push(averageRelevantBands(samples));

        const visibleBarCount = Math.max(30, Math.floor(canvas.clientWidth / (BAR_WIDTH + BAR_GAP)));
        const maxHistory = Math.max(visibleBarCount, 120);
        if (historyRef.current.length > maxHistory) {
          historyRef.current.splice(0, historyRef.current.length - maxHistory);
        }
      }

      drawWaveform(canvas, historyRef.current, previewStateRef.current, mode);
      frameRef.current = window.requestAnimationFrame(renderFrame);
    };

    const ensurePreview = async (): Promise<void> => {
      if (streamRef.current && analyserRef.current && samplesRef.current) {
        if (previewStateRef.current === "connecting") {
          updatePreviewState("running");
        }
        if (frameRef.current === null) {
          frameRef.current = window.requestAnimationFrame(renderFrame);
        }
        return;
      }

      updatePreviewState("connecting");

      try {
        const stream = await navigator.mediaDevices.getUserMedia({
          audio: {
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
          },
          video: false,
        });
        if (cancelled) {
          stream.getTracks().forEach((track) => track.stop());
          return;
        }

        const audioContext = createAudioContextInstance();
        if (!audioContext) {
          stream.getTracks().forEach((track) => track.stop());
          updatePreviewState("unavailable");
          return;
        }

        audioContextRef.current = audioContext;
        streamRef.current = stream;

        const analyser = audioContext.createAnalyser();
        analyser.fftSize = FFT_SIZE;
        analyser.smoothingTimeConstant = SMOOTHING;

        const source = audioContext.createMediaStreamSource(stream);
        source.connect(analyser);

        sourceRef.current = source;
        analyserRef.current = analyser;
        samplesRef.current = new Uint8Array(analyser.frequencyBinCount);

        if (audioContext.state === "suspended") {
          await audioContext.resume().catch(() => undefined);
        }
        if (cancelled) {
          stopPreview();
          return;
        }

        updatePreviewState("running");
        stopAnimation();
        frameRef.current = window.requestAnimationFrame(renderFrame);
      } catch (error) {
        if (cancelled) {
          return;
        }

        const code = error instanceof DOMException ? error.name : "";
        updatePreviewState(
          code === "NotAllowedError" || code === "PermissionDeniedError"
            ? "blocked"
            : "unavailable",
        );
      }
    };

    void ensurePreview();

    if (mode === "paused") {
      stopAnimation();
      const canvas = canvasRef.current;
      if (canvas) {
        drawWaveform(canvas, historyRef.current, previewStateRef.current, "paused");
      }
    } else if (mode === "running" && analyserRef.current && frameRef.current === null) {
      frameRef.current = window.requestAnimationFrame(renderFrame);
    }

    return () => {
      cancelled = true;
      stopAnimation();
    };
  }, [mode]);

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
    <section className="live-waveform-panel" aria-label="Live microphone waveform">
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
