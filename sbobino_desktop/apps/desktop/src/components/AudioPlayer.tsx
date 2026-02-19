import { Pause, Play, Rabbit } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { readAudioFile } from "../lib/tauri";

type AudioPlayerProps = {
  inputPath: string | null;
  onMetadataLoaded?: (metadata: { durationSeconds: number }) => void;
};

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) {
    return "00:00";
  }

  const mm = String(Math.floor(seconds / 60)).padStart(2, "0");
  const ss = String(Math.floor(seconds % 60)).padStart(2, "0");
  return `${mm}:${ss}`;
}

function mimeFromPath(path: string): string {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  switch (extension) {
    case "mp3":
      return "audio/mpeg";
    case "wav":
      return "audio/wav";
    case "m4a":
      return "audio/mp4";
    case "aac":
      return "audio/aac";
    case "ogg":
      return "audio/ogg";
    case "opus":
      return "audio/opus";
    case "flac":
      return "audio/flac";
    default:
      return "audio/*";
  }
}

export function AudioPlayer({ inputPath, onMetadataLoaded }: AudioPlayerProps): JSX.Element | null {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const fallbackBlobUrlRef = useRef<string | null>(null);
  const fallbackAttemptedRef = useRef(false);
  const pendingAutoPlayRef = useRef(false);
  const sourceVersionRef = useRef(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [playbackRate, setPlaybackRate] = useState(1);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [needsFallback, setNeedsFallback] = useState(false);

  const sourcePath = useMemo(() => {
    if (!inputPath || inputPath.trim().length === 0) {
      return null;
    }
    return inputPath;
  }, [inputPath]);

  useEffect(() => {
    if (fallbackBlobUrlRef.current) {
      URL.revokeObjectURL(fallbackBlobUrlRef.current);
      fallbackBlobUrlRef.current = null;
    }

    if (!sourcePath) {
      setSrc(null);
      setLoadError(null);
      setIsLoading(false);
      fallbackAttemptedRef.current = false;
      pendingAutoPlayRef.current = false;
      setNeedsFallback(false);
      onMetadataLoaded?.({ durationSeconds: 0 });
      return;
    }

    if (sourcePath.startsWith("http://") || sourcePath.startsWith("https://")) {
      setSrc(sourcePath);
      setLoadError(null);
      setIsLoading(false);
      fallbackAttemptedRef.current = false;
      pendingAutoPlayRef.current = false;
      setNeedsFallback(false);
      return;
    }

    const primarySrc = isTauri() ? convertFileSrc(sourcePath) : sourcePath;
    const sourceVersion = sourceVersionRef.current + 1;
    sourceVersionRef.current = sourceVersion;

    setSrc(primarySrc);
    setLoadError(null);
    setIsLoading(false);
    fallbackAttemptedRef.current = false;
    pendingAutoPlayRef.current = false;
    setNeedsFallback(false);

    return () => {
      if (sourceVersionRef.current === sourceVersion) {
        sourceVersionRef.current += 1;
      }
    };
  }, [sourcePath]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    audio.pause();
    audio.currentTime = 0;
    setIsPlaying(false);
    setCurrentTime(0);
    setDuration(0);
    setLoadError(null);
    pendingAutoPlayRef.current = false;
  }, [src]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.playbackRate = playbackRate;
  }, [playbackRate]);

  if (!sourcePath) {
    return null;
  }

  async function loadFallbackAudio(autoPlay: boolean): Promise<boolean> {
    const currentSourcePath = sourcePath;
    if (!currentSourcePath) {
      return false;
    }

    if (fallbackBlobUrlRef.current) {
      setSrc(fallbackBlobUrlRef.current);
      setNeedsFallback(false);
      pendingAutoPlayRef.current = autoPlay;
      return true;
    }

    if (fallbackAttemptedRef.current) {
      return false;
    }

    fallbackAttemptedRef.current = true;
    const sourceVersion = sourceVersionRef.current;
    setIsLoading(true);
    setLoadError(null);

    try {
      const bytes = await readAudioFile(currentSourcePath);
      if (sourceVersionRef.current !== sourceVersion) return false;

      const blob = new Blob([new Uint8Array(bytes)], { type: mimeFromPath(currentSourcePath) });
      const objectUrl = URL.createObjectURL(blob);
      if (fallbackBlobUrlRef.current) {
        URL.revokeObjectURL(fallbackBlobUrlRef.current);
      }
      fallbackBlobUrlRef.current = objectUrl;
      pendingAutoPlayRef.current = autoPlay;
      setSrc(objectUrl);
      setNeedsFallback(false);
      setLoadError(null);
      return true;
    } catch (error) {
      if (sourceVersionRef.current !== sourceVersion) return false;
      setLoadError(`Cannot load fallback audio: ${String(error)}`);
      return false;
    } finally {
      if (sourceVersionRef.current === sourceVersion) {
        setIsLoading(false);
      }
    }
  }

  async function togglePlayback(): Promise<void> {
    const audio = audioRef.current;
    if (!audio) return;

    if (audio.paused) {
      if (needsFallback && !fallbackBlobUrlRef.current) {
        const prepared = await loadFallbackAudio(true);
        if (prepared) return;
      }

      try {
        await audio.play();
        setIsPlaying(true);
      } catch (error) {
        setIsPlaying(false);
        setLoadError(`Cannot play audio: ${String(error)}`);
      }
      return;
    }

    audio.pause();
    setIsPlaying(false);
  }

  function onSeek(value: number): void {
    const audio = audioRef.current;
    if (!audio || !Number.isFinite(value)) return;
    audio.currentTime = value;
    setCurrentTime(value);
  }

  function onChangeSpeed(nextRate: number): void {
    const audio = audioRef.current;
    if (audio) {
      audio.playbackRate = nextRate;
    }
    setPlaybackRate(nextRate);
  }

  return (
    <footer className="audio-player">
      <audio
        ref={audioRef}
        src={src ?? undefined}
        preload="metadata"
        onLoadedMetadata={(event) => {
          const durationSeconds = event.currentTarget.duration || 0;
          setDuration(durationSeconds);
          onMetadataLoaded?.({ durationSeconds });
          setLoadError(null);
          if (pendingAutoPlayRef.current) {
            pendingAutoPlayRef.current = false;
            void event.currentTarget.play().catch(() => {
              setIsPlaying(false);
            });
          }
        }}
        onTimeUpdate={(event) => {
          setCurrentTime(event.currentTarget.currentTime || 0);
        }}
        onEnded={() => {
          setIsPlaying(false);
        }}
        onPause={() => setIsPlaying(false)}
        onPlay={() => setIsPlaying(true)}
        onError={() => {
          const audioPath = sourcePath;
          if (!audioPath) {
            setIsPlaying(false);
            return;
          }

          const fallbackSrc = fallbackBlobUrlRef.current;
          if (fallbackSrc && src !== fallbackSrc) {
            setSrc(fallbackSrc);
            setNeedsFallback(false);
            setLoadError(null);
            return;
          }

          if (fallbackAttemptedRef.current) {
            setLoadError("Unable to load audio source.");
            setIsPlaying(false);
            setIsLoading(false);
            return;
          }

          setNeedsFallback(true);
          setLoadError(null);
          setIsPlaying(false);
          setIsLoading(false);
        }}
      />

      <button
        className="playback-button"
        onClick={() => void togglePlayback()}
        title="Play/Pause"
        disabled={!src || isLoading}
      >
        {isPlaying ? <Pause size={16} /> : <Play size={16} />}
      </button>

      <input
        className="audio-slider"
        type="range"
        min={0}
        max={Math.max(duration, 0.01)}
        step={0.05}
        value={Math.min(currentTime, duration || 0)}
        onChange={(event) => onSeek(Number(event.target.value))}
        onInput={(event) => onSeek(Number((event.target as HTMLInputElement).value))}
        disabled={duration <= 0 || !src || isLoading}
        aria-label="Seek audio"
      />

      <span className="audio-time">
        {formatTime(currentTime)} / {formatTime(duration)}
      </span>

      <label className="audio-speed">
        <Rabbit size={14} />
        <select
          value={String(playbackRate)}
          onChange={(event) => onChangeSpeed(Number(event.target.value))}
          aria-label="Playback speed"
        >
          <option value="0.75">0.75x</option>
          <option value="1">1x</option>
          <option value="1.25">1.25x</option>
          <option value="1.5">1.5x</option>
          <option value="1.75">1.75x</option>
          <option value="2">2x</option>
        </select>
      </label>

      {loadError ? <span className="audio-error">{loadError}</span> : null}
      {isLoading ? <span className="audio-error">Loading audio...</span> : null}
    </footer>
  );
}
