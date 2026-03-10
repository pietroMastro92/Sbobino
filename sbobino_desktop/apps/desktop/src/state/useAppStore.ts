import { create } from "zustand";
import type { AppSettings, JobProgress, TranscriptArtifact } from "../types";

function dedupeArtifactsById(artifacts: TranscriptArtifact[]): TranscriptArtifact[] {
  const seen = new Set<string>();
  const uniqueArtifacts: TranscriptArtifact[] = [];

  for (const artifact of artifacts) {
    if (seen.has(artifact.id)) {
      continue;
    }
    seen.add(artifact.id);
    uniqueArtifacts.push(artifact);
  }

  return uniqueArtifacts;
}

type AppState = {
  settings: AppSettings | null;
  selectedFile: string | null;
  activeJobId: string | null;
  progress: JobProgress | null;
  error: string | null;
  artifacts: TranscriptArtifact[];
  setSettings: (settings: AppSettings) => void;
  setSelectedFile: (path: string | null) => void;
  setJobStarted: (jobId: string) => void;
  clearActiveJob: () => void;
  setProgress: (progress: JobProgress) => void;
  setError: (message: string | null) => void;
  setArtifacts: (artifacts: TranscriptArtifact[]) => void;
  prependArtifact: (artifact: TranscriptArtifact) => void;
  upsertArtifact: (artifact: TranscriptArtifact) => void;
  removeArtifacts: (ids: string[]) => void;
};

export const useAppStore = create<AppState>((set) => ({
  settings: null,
  selectedFile: null,
  activeJobId: null,
  progress: null,
  error: null,
  artifacts: [],
  setSettings: (settings) => set({ settings }),
  setSelectedFile: (selectedFile) => set({ selectedFile }),
  setJobStarted: (activeJobId) =>
    set({ activeJobId, progress: null, error: null }),
  clearActiveJob: () => set({ activeJobId: null }),
  setProgress: (progress) => set({ progress }),
  setError: (error) => set({ error }),
  setArtifacts: (artifacts) => set({ artifacts: dedupeArtifactsById(artifacts) }),
  prependArtifact: (artifact) =>
    set((state) => ({
      artifacts: [artifact, ...state.artifacts.filter((item) => item.id !== artifact.id)],
    })),
  upsertArtifact: (artifact) =>
    set((state) => {
      const existingIndex = state.artifacts.findIndex((item) => item.id === artifact.id);
      const dedupedArtifacts = state.artifacts.filter((item) => item.id !== artifact.id);
      if (existingIndex === -1) {
        return { artifacts: [artifact, ...dedupedArtifacts] };
      }
      dedupedArtifacts.splice(existingIndex, 0, artifact);
      return {
        artifacts: dedupedArtifacts,
      };
    }),
  removeArtifacts: (ids) =>
    set((state) => ({
      artifacts: state.artifacts.filter((artifact) => !ids.includes(artifact.id)),
    })),
}));
