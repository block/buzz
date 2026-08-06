import {
  resolveArtilleryTurn,
  validateArtilleryAction,
  type ArtilleryAction,
  type ArtilleryMatch,
  type ArtilleryMatchState,
  type ArtillerySide,
  type ArtilleryTurn,
} from "@/features/games/artillery/referee";

const EVENT_PREFIX = "<!-- buzz-artillery-event:";
const EVENT_SUFFIX = " -->";
const EVENT_PATTERN = /<!-- buzz-artillery-event:(%7B[\s\S]*?%7D) -->/;

type ArtilleryDurableEventBase = {
  matchId: string;
  type: "buzz.game.artillery.event.v1";
  version: 1;
};

export type ArtilleryMatchStartedEvent = ArtilleryDurableEventBase & {
  event: "match_started";
  agents: Record<ArtillerySide, { id: string; name: string }>;
  initialHealth: Record<ArtillerySide, number>;
  maxTurns: number;
  timeoutMs: number;
};

export type ArtilleryTurnRequestedEvent = ArtilleryDurableEventBase & {
  event: "turn_requested";
  agent: { id: string; name: string };
  deadlineAt: number;
  requestId: string;
  state: ArtilleryMatchState;
};

export type ArtilleryTurnResolvedEvent = ArtilleryDurableEventBase & {
  event: "turn_resolved";
  action: ArtilleryAction;
  resolution: ArtilleryTurn["resolution"];
  state: ArtilleryMatchState;
};

export type ArtilleryMatchFinishedEvent = ArtilleryDurableEventBase & {
  event: "match_finished";
  turnCount: number;
  winner: ArtilleryMatch["winner"];
};

export type ArtilleryDurableEvent =
  | ArtilleryMatchStartedEvent
  | ArtilleryTurnRequestedEvent
  | ArtilleryTurnResolvedEvent
  | ArtilleryMatchFinishedEvent;

export type RecoveredArtilleryMatch = {
  complete: boolean;
  lastRequest: ArtilleryTurnRequestedEvent | null;
  match: ArtilleryMatch;
  maxTurns: number;
  timeoutMs: number;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object";
}

function isSide(value: unknown): value is ArtillerySide {
  return value === "red" || value === "blue";
}

function isResolution(value: unknown): value is ArtilleryTurn["resolution"] {
  return (
    value === "accepted" ||
    value === "invalid-fallback" ||
    value === "timeout-fallback"
  );
}

function isState(value: unknown): value is ArtilleryMatchState {
  if (!isRecord(value) || !isSide(value.activeSide)) return false;
  if (!isRecord(value.health)) return false;
  return (
    typeof value.id === "string" &&
    Number.isInteger(value.turn) &&
    typeof value.wind === "number" &&
    typeof value.health.red === "number" &&
    typeof value.health.blue === "number"
  );
}

function isAgent(value: unknown): value is { id: string; name: string } {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    typeof value.name === "string"
  );
}

/** Serializes a versioned event inside a Markdown-invisible HTML comment. */
export function serializeArtilleryDurableEvent(event: ArtilleryDurableEvent) {
  return `${EVENT_PREFIX}${encodeURIComponent(JSON.stringify(event))}${EVENT_SUFFIX}`;
}

/** Adds a durable event to a human-readable channel message. */
export function appendArtilleryDurableEvent(
  content: string,
  event: ArtilleryDurableEvent,
) {
  return `${content}\n\n${serializeArtilleryDurableEvent(event)}`;
}

/** Removes the machine event marker before presenting plain text elsewhere. */
export function stripArtilleryDurableEvent(content: string) {
  return content.replace(EVENT_PATTERN, "").trim();
}

/** Parses and validates the supported artillery lifecycle event envelope. */
export function parseArtilleryDurableEvent(
  content: string,
): ArtilleryDurableEvent | null {
  const encoded = content.match(EVENT_PATTERN)?.[1];
  if (!encoded) return null;

  try {
    const value: unknown = JSON.parse(decodeURIComponent(encoded));
    if (
      !isRecord(value) ||
      value.type !== "buzz.game.artillery.event.v1" ||
      value.version !== 1 ||
      typeof value.matchId !== "string"
    ) {
      return null;
    }

    if (value.event === "match_started") {
      if (
        !isRecord(value.agents) ||
        !isAgent(value.agents.red) ||
        !isAgent(value.agents.blue) ||
        !isRecord(value.initialHealth) ||
        typeof value.initialHealth.red !== "number" ||
        typeof value.initialHealth.blue !== "number" ||
        !Number.isInteger(value.maxTurns) ||
        typeof value.timeoutMs !== "number"
      ) {
        return null;
      }
      return value as ArtilleryMatchStartedEvent;
    }

    if (value.event === "turn_requested") {
      if (
        !isAgent(value.agent) ||
        typeof value.deadlineAt !== "number" ||
        typeof value.requestId !== "string" ||
        !isState(value.state)
      ) {
        return null;
      }
      return value as ArtilleryTurnRequestedEvent;
    }

    if (value.event === "turn_resolved") {
      const action = validateArtilleryAction(value.action);
      if (!action || !isResolution(value.resolution) || !isState(value.state)) {
        return null;
      }
      return { ...(value as ArtilleryTurnResolvedEvent), action };
    }

    if (value.event === "match_finished") {
      if (
        !Number.isInteger(value.turnCount) ||
        (value.winner !== "draw" && !isSide(value.winner))
      ) {
        return null;
      }
      return value as ArtilleryMatchFinishedEvent;
    }
  } catch {
    return null;
  }

  return null;
}

function winnerForHealth(health: Record<ArtillerySide, number>) {
  if (health.red === health.blue) return "draw" as const;
  return health.red > health.blue ? ("red" as const) : ("blue" as const);
}

/**
 * Reduces channel lifecycle events into a deterministic match snapshot.
 * Duplicate, out-of-order, or state-inconsistent turns are ignored.
 */
export function recoverArtilleryMatch(
  events: readonly ArtilleryDurableEvent[],
  expectedMatchId?: string,
): RecoveredArtilleryMatch | null {
  const started = events.find(
    (event): event is ArtilleryMatchStartedEvent =>
      event.event === "match_started" &&
      (!expectedMatchId || event.matchId === expectedMatchId),
  );
  if (!started) return null;

  const health = { ...started.initialHealth };
  const turns: ArtilleryTurn[] = [];
  let lastRequest: ArtilleryTurnRequestedEvent | null = null;
  let complete = false;

  const resolvedEvents = events
    .filter(
      (event): event is ArtilleryTurnResolvedEvent =>
        event.matchId === started.matchId && event.event === "turn_resolved",
    )
    .sort((left, right) => left.state.turn - right.state.turn);
  for (const event of resolvedEvents) {
    const expectedTurn = turns.length + 1;
    const expectedSide: ArtillerySide = expectedTurn % 2 === 1 ? "red" : "blue";
    if (
      event.state.id !== started.matchId ||
      event.state.turn !== expectedTurn ||
      event.state.activeSide !== expectedSide ||
      event.state.health.red !== health.red ||
      event.state.health.blue !== health.blue
    ) {
      continue;
    }
    const turn = resolveArtilleryTurn(
      event.state,
      started.agents[expectedSide].name,
      event.action,
      event.resolution,
    );
    health[turn.manifest.damage.target] = turn.manifest.damage.after;
    turns.push(turn);
  }

  lastRequest =
    events
      .filter(
        (event): event is ArtilleryTurnRequestedEvent =>
          event.matchId === started.matchId &&
          event.event === "turn_requested" &&
          event.state.turn > turns.length,
      )
      .sort((left, right) => right.state.turn - left.state.turn)[0] ?? null;

  complete = events.some(
    (event) =>
      event.matchId === started.matchId &&
      event.event === "match_finished" &&
      event.turnCount === turns.length &&
      event.winner === winnerForHealth(health),
  );
  if (complete) lastRequest = null;

  return {
    complete,
    lastRequest,
    match: {
      id: started.matchId,
      agents: structuredClone(started.agents),
      initialHealth: { ...started.initialHealth },
      turns,
      winner: winnerForHealth(health),
    },
    maxTurns: started.maxTurns,
    timeoutMs: started.timeoutMs,
  };
}

/** Creates the durable root event for a new match. */
export function createArtilleryStartedEvent({
  agents,
  matchId,
  maxTurns,
  timeoutMs,
}: {
  agents: ArtilleryMatch["agents"];
  matchId: string;
  maxTurns: number;
  timeoutMs: number;
}): ArtilleryMatchStartedEvent {
  return {
    agents: structuredClone(agents),
    event: "match_started",
    initialHealth: { red: 100, blue: 100 },
    matchId,
    maxTurns,
    timeoutMs,
    type: "buzz.game.artillery.event.v1",
    version: 1,
  };
}

/** Creates a compact canonical resolved-turn event. */
export function createArtilleryTurnResolvedEvent(
  state: ArtilleryMatchState,
  turn: ArtilleryTurn,
): ArtilleryTurnResolvedEvent {
  return {
    action: structuredClone(turn.action),
    event: "turn_resolved",
    matchId: state.id,
    resolution: turn.resolution,
    state: structuredClone(state),
    type: "buzz.game.artillery.event.v1",
    version: 1,
  };
}

/** Creates a correlated durable request event embedded in an agent prompt. */
export function createArtilleryTurnRequestedEvent({
  agent,
  deadlineAt,
  requestId,
  state,
}: {
  agent: { id: string; name: string };
  deadlineAt: number;
  requestId: string;
  state: ArtilleryMatchState;
}): ArtilleryTurnRequestedEvent {
  return {
    agent: structuredClone(agent),
    deadlineAt,
    event: "turn_requested",
    matchId: state.id,
    requestId,
    state: structuredClone(state),
    type: "buzz.game.artillery.event.v1",
    version: 1,
  };
}

/** Creates the terminal durable event for a completed match. */
export function createArtilleryFinishedEvent(
  match: ArtilleryMatch,
): ArtilleryMatchFinishedEvent {
  return {
    event: "match_finished",
    matchId: match.id,
    turnCount: match.turns.length,
    type: "buzz.game.artillery.event.v1",
    version: 1,
    winner: match.winner,
  };
}
