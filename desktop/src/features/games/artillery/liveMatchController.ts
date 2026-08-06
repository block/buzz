import {
  runArtilleryMatch,
  type ArtilleryAgent,
  type ArtilleryMatch,
  type ArtilleryMatchProgress,
  type ArtillerySide,
} from "@/features/games/artillery/referee";
import { artilleryRefereeHostSession } from "@/features/games/artillery/refereeHostSession";

export type LiveArtilleryMatchStatus =
  | "idle"
  | "waiting"
  | "running"
  | "complete"
  | "error";

export type LiveArtilleryWaitingState = {
  agentName: string;
  deadlineAt: number;
  side: ArtillerySide;
  startedAt: number;
  turn: number;
};

export type LiveArtilleryMatchSnapshot = {
  channelId: string | null;
  error: string | null;
  match: ArtilleryMatch | null;
  matchComplete: boolean;
  published: boolean;
  status: LiveArtilleryMatchStatus;
  statusEventId: string | null;
  timeoutMs: number;
  waitingFor: LiveArtilleryWaitingState | null;
};

type StartLiveArtilleryMatchInput = {
  agents: Record<ArtillerySide, ArtilleryAgent>;
  channelId: string;
  id?: string;
  maxTurns?: number;
  statusEventId?: string | null;
  timeoutMs: number;
  onMatchComplete?: (match: ArtilleryMatch) => Promise<void> | void;
  onTurnResolved?: (progress: ArtilleryMatchProgress) => Promise<void> | void;
  resumeMatch?: ArtilleryMatch;
};

type HydrateLiveArtilleryMatchInput = {
  channelId: string;
  match: ArtilleryMatch;
  matchComplete: boolean;
  statusEventId: string;
  timeoutMs: number;
};

const EMPTY_SNAPSHOT: LiveArtilleryMatchSnapshot = {
  channelId: null,
  error: null,
  match: null,
  matchComplete: false,
  published: false,
  status: "idle",
  statusEventId: null,
  timeoutMs: 5_000,
  waitingFor: null,
};

let snapshot = EMPTY_SNAPSHOT;
let generation = 0;
let hostingMatchId: string | null = null;
const listeners = new Set<() => void>();

function emit(next: LiveArtilleryMatchSnapshot) {
  snapshot = next;
  for (const listener of listeners) listener();
}

function initialMatch(
  id: string,
  agents: Record<ArtillerySide, ArtilleryAgent>,
): ArtilleryMatch {
  return {
    id,
    agents: {
      red: { id: agents.red.id, name: agents.red.name },
      blue: { id: agents.blue.id, name: agents.blue.name },
    },
    initialHealth: { red: 100, blue: 100 },
    turns: [],
    winner: "draw",
  };
}

export const liveArtilleryMatchController = {
  getSnapshot() {
    return snapshot;
  },

  subscribe(listener: () => void) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  async start({
    agents,
    channelId,
    id = `live-${crypto.randomUUID()}`,
    maxTurns = 8,
    statusEventId = null,
    timeoutMs,
    onMatchComplete,
    onTurnResolved,
    resumeMatch,
  }: StartLiveArtilleryMatchInput) {
    if (
      hostingMatchId &&
      (snapshot.status === "waiting" || snapshot.status === "running")
    ) {
      throw new Error("A live artillery match is already running");
    }

    const currentGeneration = generation + 1;
    generation = currentGeneration;
    hostingMatchId = id;
    emit({
      channelId,
      error: null,
      match: resumeMatch ?? initialMatch(id, agents),
      matchComplete: false,
      published: false,
      status: "waiting",
      statusEventId,
      timeoutMs,
      waitingFor: null,
    });

    try {
      const match = await runArtilleryMatch({
        agents,
        id,
        maxTurns,
        moveTimeoutMs: timeoutMs,
        onTurnRequest: ({ agent, state }) => {
          if (generation !== currentGeneration) return;
          const startedAt = Date.now();
          emit({
            ...snapshot,
            status: "waiting",
            waitingFor: {
              agentName: agent.name,
              deadlineAt: startedAt + timeoutMs,
              side: state.activeSide,
              startedAt,
              turn: state.turn,
            },
          });
        },
        onTurnResolved: async (progress) => {
          if (generation !== currentGeneration) return;
          emit({
            ...snapshot,
            match: progress.match,
            status: "running",
            waitingFor: null,
          });
          await onTurnResolved?.(progress);
        },
        resumeMatch,
      });
      if (generation !== currentGeneration) return match;
      hostingMatchId = null;
      emit({
        ...snapshot,
        match,
        matchComplete: true,
        status: "complete",
        waitingFor: null,
      });
      await onMatchComplete?.(match);
      return match;
    } catch (cause) {
      if (generation !== currentGeneration) throw cause;
      hostingMatchId = null;
      emit({
        ...snapshot,
        error: cause instanceof Error ? cause.message : "Live match failed",
        matchComplete: true,
        status: "error",
        waitingFor: null,
      });
      throw cause;
    }
  },

  markPublished() {
    emit({ ...snapshot, published: true });
  },

  /** Stops local refereeing after a newer channel lease fences this host. */
  yieldReferee() {
    generation += 1;
    hostingMatchId = null;
    emit({
      ...snapshot,
      error: "Another Buzz client took over the referee lease.",
      status: snapshot.matchComplete ? snapshot.status : "running",
      waitingFor: null,
    });
  },

  /** Hydrates a spectator or recovered route from canonical channel events. */
  hydrate({
    channelId,
    match,
    matchComplete,
    statusEventId,
    timeoutMs,
  }: HydrateLiveArtilleryMatchInput) {
    if (hostingMatchId === match.id) {
      return;
    }
    emit({
      channelId,
      error: null,
      match: structuredClone(match),
      matchComplete,
      published: matchComplete,
      status: matchComplete ? "complete" : "running",
      statusEventId,
      timeoutMs,
      waitingFor: null,
    });
  },

  reset() {
    generation += 1;
    hostingMatchId = null;
    void artilleryRefereeHostSession.stop(false);
    emit(EMPTY_SNAPSHOT);
  },
};
