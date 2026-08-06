export type ArtilleryTrajectoryPoint = {
  t: number;
  x: number;
  y: number;
};

export type ArtilleryAnimationManifest = {
  id: string;
  turn?: number;
  durationMs: number;
  shooter: "red" | "blue";
  shooterName?: string;
  angle: number;
  power: number;
  wind: number;
  taunt?: string;
  resolution?: "accepted" | "invalid-fallback" | "timeout-fallback";
  trajectory: ArtilleryTrajectoryPoint[];
  impact: {
    t: number;
    x: number;
    y: number;
    radius: number;
  };
  damage: {
    target: "red" | "blue";
    before: number;
    after: number;
  };
};

const SHOT_DURATION_MS = 2_450;
const SAMPLE_INTERVAL_MS = 70;

/**
 * A fixed, referee-shaped trajectory used by the Phase 1 visual spike.
 *
 * The renderer deliberately consumes samples instead of calculating an
 * outcome. Later phases can replace this fixture with a signed manifest from
 * the referee without changing the animation contract.
 */
function createDemoTrajectory(): ArtilleryTrajectoryPoint[] {
  const points: ArtilleryTrajectoryPoint[] = [];
  const start = { x: 166, y: 364 };
  const end = { x: 783, y: 344 };
  const sampleCount = Math.ceil(SHOT_DURATION_MS / SAMPLE_INTERVAL_MS);

  for (let index = 0; index <= sampleCount; index += 1) {
    const progress = index / sampleCount;
    const arc = Math.sin(progress * Math.PI) * 255;
    const gust = Math.sin(progress * Math.PI * 2.4) * 10 * progress;
    points.push({
      t: Math.round(progress * SHOT_DURATION_MS),
      x: start.x + (end.x - start.x) * progress + gust,
      y: start.y + (end.y - start.y) * progress - arc,
    });
  }

  return points;
}

export const PHASE_ONE_DEMO_MANIFEST: ArtilleryAnimationManifest = {
  id: "phase-one-demo-shot",
  durationMs: SHOT_DURATION_MS,
  shooter: "red",
  angle: 42,
  power: 71,
  wind: -3.2,
  trajectory: createDemoTrajectory(),
  impact: {
    t: SHOT_DURATION_MS,
    x: 783,
    y: 344,
    radius: 48,
  },
  damage: {
    target: "blue",
    before: 72,
    after: 49,
  },
};

export function pointAtTime(
  manifest: ArtilleryAnimationManifest,
  elapsedMs: number,
): ArtilleryTrajectoryPoint {
  const clampedTime = Math.max(0, Math.min(elapsedMs, manifest.durationMs));
  const points = manifest.trajectory;

  for (let index = 1; index < points.length; index += 1) {
    const next = points[index];
    if (next.t < clampedTime) continue;

    const previous = points[index - 1];
    const span = Math.max(1, next.t - previous.t);
    const progress = (clampedTime - previous.t) / span;
    return {
      t: clampedTime,
      x: previous.x + (next.x - previous.x) * progress,
      y: previous.y + (next.y - previous.y) * progress,
    };
  }

  return points.at(-1) ?? { t: 0, x: 0, y: 0 };
}
