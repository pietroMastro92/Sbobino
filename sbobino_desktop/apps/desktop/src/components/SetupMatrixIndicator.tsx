import { useEffect, useMemo, useState } from "react";

type SetupMatrixIndicatorProps = {
  progress?: number;
  size?: number;
  ariaLabel?: string;
};

type MatrixCell = {
  key: string;
  x: number;
  y: number;
  row: number;
  col: number;
  progressIndex: number;
};

const GRID_SIZE = 7;
const CELL_SPACING = 10;
const CELL_RADIUS = 3.1;
const VIEWBOX_SIZE = 72;
const FRAME_INTERVAL_MS = 1000 / 18;

function clampProgress(progress?: number): number {
  if (!Number.isFinite(progress)) {
    return 0.08;
  }
  return Math.min(1, Math.max(0, Number(progress) / 100));
}

function clampOpacity(value: number): number {
  return Math.min(1, Math.max(0.08, value));
}

function buildCells(): MatrixCell[] {
  const cells: MatrixCell[] = [];
  for (let row = 0; row < GRID_SIZE; row += 1) {
    for (let col = 0; col < GRID_SIZE; col += 1) {
      cells.push({
        key: `${row}-${col}`,
        x: 6 + col * CELL_SPACING,
        y: 6 + row * CELL_SPACING,
        row,
        col,
        // Bottom-left to top-right feels closer to setup progress than row-major fill.
        progressIndex: (GRID_SIZE - 1 - row) * GRID_SIZE + col,
      });
    }
  }
  return cells;
}

function cellOpacity(cell: MatrixCell, normalizedProgress: number, frame: number): number {
  const totalCells = GRID_SIZE * GRID_SIZE;
  const fillCutoff = normalizedProgress * totalCells;
  const isFilled = cell.progressIndex + 0.35 < fillCutoff;
  const phase = frame / 3.5;
  const pulse = (Math.sin(phase) + 1) / 2;
  const diagonalSweep = (frame * 0.22) % (GRID_SIZE + GRID_SIZE - 1);
  const diagonalDistance = Math.abs(cell.row + cell.col - diagonalSweep);
  const sweepGlow = Math.max(0, 1 - diagonalDistance / 2.15) * (0.34 + pulse * 0.18);
  const centerDistance = Math.hypot(cell.row - 3, cell.col - 3);
  const coreGlow = Math.max(0, 1 - centerDistance / 4.6) * (0.12 + pulse * 0.08);
  const leadingEdge = Math.max(0, 1 - Math.abs(cell.progressIndex - fillCutoff) / 1.25) * (0.18 + pulse * 0.18);
  const base = isFilled ? 0.82 : 0.08;

  return clampOpacity(Math.max(base, sweepGlow, coreGlow, leadingEdge));
}

export function SetupMatrixIndicator({
  progress,
  size = 64,
  ariaLabel = "Local setup progress",
}: SetupMatrixIndicatorProps): JSX.Element {
  const [frame, setFrame] = useState(0);
  const cells = useMemo(() => buildCells(), []);
  const normalizedProgress = clampProgress(progress);

  useEffect(() => {
    let animationFrame = 0;
    let lastTimestamp = 0;
    let accumulator = 0;

    const tick = (timestamp: number) => {
      if (lastTimestamp === 0) {
        lastTimestamp = timestamp;
      }

      accumulator += timestamp - lastTimestamp;
      lastTimestamp = timestamp;

      if (accumulator >= FRAME_INTERVAL_MS) {
        setFrame((current) => current + 1);
        accumulator %= FRAME_INTERVAL_MS;
      }

      animationFrame = window.requestAnimationFrame(tick);
    };

    animationFrame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(animationFrame);
  }, []);

  return (
    <span
      className="startup-matrix-indicator"
      role="img"
      aria-label={ariaLabel}
      style={{ width: `${size}px`, height: `${size}px` }}
    >
      <svg
        viewBox={`0 0 ${VIEWBOX_SIZE} ${VIEWBOX_SIZE}`}
        aria-hidden="true"
        focusable="false"
      >
        {cells.map((cell) => (
          <circle
            key={`track-${cell.key}`}
            className="startup-matrix-indicator__track"
            cx={cell.x}
            cy={cell.y}
            r={CELL_RADIUS}
          />
        ))}
        {cells.map((cell) => (
          <circle
            key={`active-${cell.key}`}
            className="startup-matrix-indicator__active"
            cx={cell.x}
            cy={cell.y}
            r={CELL_RADIUS}
            style={{ opacity: cellOpacity(cell, normalizedProgress, frame) }}
          />
        ))}
      </svg>
    </span>
  );
}
