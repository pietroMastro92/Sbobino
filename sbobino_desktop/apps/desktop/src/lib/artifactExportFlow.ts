export type ArtifactExportFlowPhase = "destination" | "sync" | "write";

export class ArtifactExportFlowError extends Error {
  readonly phase: ArtifactExportFlowPhase;
  readonly originalError: unknown;

  constructor(phase: ArtifactExportFlowPhase, originalError: unknown) {
    super(`Artifact export failed during ${phase}`);
    this.name = "ArtifactExportFlowError";
    this.phase = phase;
    this.originalError = originalError;
  }
}

export type ArtifactExportFlowResult<TArtifact> =
  | { status: "cancelled" }
  | { status: "sync_not_found" }
  | { status: "exported"; artifact: TArtifact; destination: string };

export async function runArtifactExportFlow<TArtifact>(params: {
  artifact: TArtifact;
  chooseDestination: () => Promise<string | null>;
  syncArtifact?: () => Promise<TArtifact | null>;
  onArtifactSynced?: (artifact: TArtifact) => void;
  writeArtifact: (artifact: TArtifact, destination: string) => Promise<void>;
}): Promise<ArtifactExportFlowResult<TArtifact>> {
  let destination: string | null;
  try {
    destination = await params.chooseDestination();
  } catch (error) {
    throw new ArtifactExportFlowError("destination", error);
  }

  if (!destination) {
    return { status: "cancelled" };
  }

  let artifact = params.artifact;
  if (params.syncArtifact) {
    try {
      const synced = await params.syncArtifact();
      if (!synced) {
        return { status: "sync_not_found" };
      }
      artifact = synced;
      params.onArtifactSynced?.(synced);
    } catch (error) {
      throw new ArtifactExportFlowError("sync", error);
    }
  }

  try {
    await params.writeArtifact(artifact, destination);
  } catch (error) {
    throw new ArtifactExportFlowError("write", error);
  }

  return { status: "exported", artifact, destination };
}
