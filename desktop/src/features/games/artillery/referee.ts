import type {
  ArtilleryAnimationManifest,
  ArtilleryTrajectoryPoint,
} from "@/features/games/artillery/manifest";

export type ArtillerySide = "red" | "blue";
export type ArtilleryWeapon = "pulse-shell";

export type ArtilleryAction = {
  angle: number;
  power: number;
  weapon: ArtilleryWeapon;
  taunt?: string;
};

export type ArtilleryAgent = {
  id: string;
  name: string;
  side: ArtillerySide;
  decide: (state: Readonly<ArtilleryMatchState>) => Promise<unknown> | unknown;
};

export type ArtilleryMatchState = {
  id: string;
  turn: number;
  activeSide: ArtillerySide;
  health: Record<ArtillerySide, number>;
  wind: number;
};

export type ArtilleryTurn = {
  action: ArtilleryAction;
  manifest: ArtilleryAnimationManifest;
  resolution: "accepted" | "invalid-fallback" | "timeout-fallback";
};

export type ArtilleryMatch = {
  id: string;
  agents: Record<ArtillerySide, { id: string; name: string }>;
  initialHealth: Record<ArtillerySide, number>;
  turns: ArtilleryTurn[];
  winner: ArtillerySide | "draw";
};

export type ArtilleryChannelEnvelope = {
  type: "buzz.game.artillery.match.v1";
  version: 1;
  match: ArtilleryMatch;
};

export type ArtilleryMatchProgress = {
  match: ArtilleryMatch;
  state: ArtilleryMatchState;
  turn: ArtilleryTurn;
};

export type ArtilleryTurnRequest = {
  agent: ArtilleryAgent;
  state: ArtilleryMatchState;
};

const SHOT_DURATION_MS = 1_350;
const SAMPLE_INTERVAL_MS = 45;
const STARTS: Record<ArtillerySide, { x: number; y: number }> = {
  red: { x: 166, y: 364 },
  blue: { x: 783, y: 344 },
};
const SAFE_ACTION: ArtilleryAction = {
  angle: 45,
  power: 68,
  weapon: "pulse-shell",
};
const WIND_SEQUENCE = [0, -2, 3, 1, -1, 2, -3, 0] as const;

export class ArtilleryAgentTimeoutError extends Error {
  constructor() {
    super("Agent move timed out");
    this.name = "ArtilleryAgentTimeoutError";
  }
}

function otherSide(side: ArtillerySide): ArtillerySide {
  return side === "red" ? "blue" : "red";
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function validateArtilleryAction(
  value: unknown,
): ArtilleryAction | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<ArtilleryAction>;
  if (
    !isFiniteNumber(candidate.angle) ||
    candidate.angle < 20 ||
    candidate.angle > 80 ||
    !isFiniteNumber(candidate.power) ||
    candidate.power < 30 ||
    candidate.power > 100 ||
    candidate.weapon !== "pulse-shell"
  ) {
    return null;
  }

  return {
    angle: Math.round(candidate.angle * 10) / 10,
    power: Math.round(candidate.power * 10) / 10,
    weapon: candidate.weapon,
    taunt:
      typeof candidate.taunt === "string"
        ? candidate.taunt.trim().slice(0, 120)
        : undefined,
  };
}

function trajectoryFor(
  shooter: ArtillerySide,
  action: ArtilleryAction,
  wind: number,
): ArtilleryTrajectoryPoint[] {
  const start = STARTS[shooter];
  const direction = shooter === "red" ? 1 : -1;
  const travel = action.power * 8.55 + wind * direction * 5;
  const endX = PhaserMathClamp(start.x + direction * travel, 32, 928);
  const endY = shooter === "red" ? 344 : 364;
  const arcHeight = 105 + action.power * 1.75 + (action.angle - 45) * 2.4;
  const sampleCount = Math.ceil(SHOT_DURATION_MS / SAMPLE_INTERVAL_MS);
  const points: ArtilleryTrajectoryPoint[] = [];

  for (let index = 0; index <= sampleCount; index += 1) {
    const progress = index / sampleCount;
    points.push({
      t: Math.round(progress * SHOT_DURATION_MS),
      x: start.x + (endX - start.x) * progress,
      y:
        start.y +
        (endY - start.y) * progress -
        Math.sin(progress * Math.PI) * arcHeight,
    });
  }
  return points;
}

function PhaserMathClamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function damageForDistance(distance: number) {
  return Math.max(0, Math.round(44 - distance * 0.55));
}

export function resolveArtilleryTurn(
  state: ArtilleryMatchState,
  agentName: string,
  action: ArtilleryAction,
  resolution: ArtilleryTurn["resolution"] = "accepted",
): ArtilleryTurn {
  const target = otherSide(state.activeSide);
  const trajectory = trajectoryFor(state.activeSide, action, state.wind);
  const endpoint = trajectory.at(-1) ?? STARTS[state.activeSide];
  const targetPosition = STARTS[target];
  const damage = damageForDistance(
    Math.hypot(endpoint.x - targetPosition.x, endpoint.y - targetPosition.y),
  );
  const before = state.health[target];
  const after = Math.max(0, before - damage);
  const radius = damage > 0 ? 46 : 28;

  return {
    action,
    resolution,
    manifest: {
      id: `${state.id}-turn-${state.turn}`,
      turn: state.turn,
      durationMs: SHOT_DURATION_MS,
      shooter: state.activeSide,
      shooterName: agentName,
      angle: action.angle,
      power: action.power,
      wind: state.wind,
      taunt: action.taunt,
      resolution,
      trajectory,
      impact: {
        t: SHOT_DURATION_MS,
        x: endpoint.x,
        y: endpoint.y,
        radius,
      },
      damage: { target, before, after },
    },
  };
}

async function decideWithTimeout(
  agent: ArtilleryAgent,
  state: ArtilleryMatchState,
  timeoutMs: number,
) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      Promise.resolve(agent.decide(structuredClone(state))),
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new ArtilleryAgentTimeoutError()),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

export async function runArtilleryMatch({
  agents,
  id = "artillery-mock-match-001",
  maxTurns = 8,
  moveTimeoutMs = 250,
  onTurnRequest,
  onTurnResolved,
  resumeMatch,
}: {
  agents: Record<ArtillerySide, ArtilleryAgent>;
  id?: string;
  maxTurns?: number;
  moveTimeoutMs?: number;
  onTurnRequest?: (request: ArtilleryTurnRequest) => Promise<void> | void;
  onTurnResolved?: (progress: ArtilleryMatchProgress) => Promise<void> | void;
  resumeMatch?: ArtilleryMatch;
}): Promise<ArtilleryMatch> {
  if (resumeMatch && resumeMatch.id !== id) {
    throw new Error("Cannot resume a different artillery match");
  }
  const turns: ArtilleryTurn[] = structuredClone(resumeMatch?.turns ?? []);
  const health = { red: 100, blue: 100 };
  for (const turn of turns) {
    health[turn.manifest.damage.target] = turn.manifest.damage.after;
  }

  for (let index = turns.length; index < maxTurns; index += 1) {
    const activeSide: ArtillerySide = index % 2 === 0 ? "red" : "blue";
    const state: ArtilleryMatchState = {
      id,
      turn: index + 1,
      activeSide,
      health: { ...health },
      wind: WIND_SEQUENCE[index % WIND_SEQUENCE.length],
    };
    let action = SAFE_ACTION;
    let resolution: ArtilleryTurn["resolution"] = "accepted";
    await onTurnRequest?.({
      agent: agents[activeSide],
      state: structuredClone(state),
    });

    try {
      const proposed = await decideWithTimeout(
        agents[activeSide],
        state,
        moveTimeoutMs,
      );
      const validated = validateArtilleryAction(proposed);
      if (validated) action = validated;
      else resolution = "invalid-fallback";
    } catch (error) {
      if (!(error instanceof ArtilleryAgentTimeoutError)) throw error;
      resolution = "timeout-fallback";
    }

    const turn = resolveArtilleryTurn(
      state,
      agents[activeSide].name,
      action,
      resolution,
    );
    health[turn.manifest.damage.target] = turn.manifest.damage.after;
    turns.push(turn);
    await onTurnResolved?.({
      match: createMatchSnapshot(id, agents, turns, health),
      state: structuredClone(state),
      turn: structuredClone(turn),
    });
    if (health.red === 0 || health.blue === 0) break;
  }

  return createMatchSnapshot(id, agents, turns, health);
}

function createMatchSnapshot(
  id: string,
  agents: Record<ArtillerySide, ArtilleryAgent>,
  turns: ArtilleryTurn[],
  health: Record<ArtillerySide, number>,
): ArtilleryMatch {
  const winner =
    health.red === health.blue
      ? "draw"
      : health.red > health.blue
        ? "red"
        : "blue";
  return {
    id,
    agents: {
      red: { id: agents.red.id, name: agents.red.name },
      blue: { id: agents.blue.id, name: agents.blue.name },
    },
    initialHealth: { red: 100, blue: 100 },
    turns: structuredClone(turns),
    winner,
  };
}

export function createArtilleryChannelEnvelope(
  match: ArtilleryMatch,
): ArtilleryChannelEnvelope {
  return { type: "buzz.game.artillery.match.v1", version: 1, match };
}
