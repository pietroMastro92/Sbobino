export type RealtimeWaveformBin = { min: number; max: number };

export type RealtimeWaveformRead = {
  sequence: number;
  bins: RealtimeWaveformBin[];
};

function clampSigned(value: number): number {
  return Math.min(1, Math.max(-1, Number.isFinite(value) ? value : 0));
}

export class RealtimeLevelBuffer {
  private readonly minimums: Float32Array;
  private readonly maximums: Float32Array;
  private writeIndex = 0;
  private length = 0;
  private sequence = 0;
  private readonly listeners = new Set<() => void>();

  constructor(capacity: number) {
    const safeCapacity = Math.max(1, Math.floor(capacity));
    this.minimums = new Float32Array(safeCapacity);
    this.maximums = new Float32Array(safeCapacity);
  }

  push(value: number): void {
    const amplitude = Math.min(1, Math.max(0, Number.isFinite(value) ? value : 0));
    this.pushEnvelope([{ min: -amplitude, max: amplitude }]);
  }

  pushEnvelope(bins: RealtimeWaveformBin[]): void {
    if (bins.length === 0) return;
    for (const bin of bins) {
      const minimum = Math.min(clampSigned(bin.min), clampSigned(bin.max));
      const maximum = Math.max(clampSigned(bin.min), clampSigned(bin.max));
      this.minimums[this.writeIndex] = minimum;
      this.maximums[this.writeIndex] = maximum;
      this.writeIndex = (this.writeIndex + 1) % this.minimums.length;
      this.length = Math.min(this.length + 1, this.minimums.length);
      this.sequence += 1;
    }
    this.notify();
  }

  clear(): void {
    this.writeIndex = 0;
    this.length = 0;
    this.sequence += 1;
    this.notify();
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private notify(): void {
    this.listeners.forEach((listener) => listener());
  }

  readSince(sequence: number): RealtimeWaveformRead {
    const earliestSequence = this.sequence - this.length;
    const startSequence = Math.max(earliestSequence, Math.min(this.sequence, sequence));
    const count = this.sequence - startSequence;
    const firstIndex = (this.writeIndex - this.length + this.minimums.length) % this.minimums.length;
    const offset = startSequence - earliestSequence;
    const bins = Array.from({ length: count }, (_, index) => {
      const bufferIndex = (firstIndex + offset + index) % this.minimums.length;
      return {
        min: this.minimums[bufferIndex] ?? 0,
        max: this.maximums[bufferIndex] ?? 0,
      };
    });
    return { sequence: this.sequence, bins };
  }

  snapshot(): number[] {
    const read = this.readSince(this.sequence - this.length);
    return read.bins.map((bin) => Math.max(Math.abs(bin.min), Math.abs(bin.max)));
  }
}
