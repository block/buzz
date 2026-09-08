// Ported/adapted from Berd's agent-activity-panel (branch
// zmarley/agent-activity-panel). Berd segmented turns from chat messages;
// in Buzz turns are first-class in the observer stream, bounded by
// turn_started / turn_completed frames and correlated back to the channel
// messages that triggered them.

import type { ObserverEvent } from "../ui/agentSessionTypes";
import type { ActivityEvent } from "./activityModel";

export type ActivityTurnSource = "channel" | "heartbeat";

export interface ActivityTurn {
  /** The observer turnId. */
  id: string;
  /** Epoch ms of the turn_started frame (or first frame seen for the turn). */
  startT: number;
  /** Epoch ms of turn_completed; undefined while the turn looks live. */
  endT?: number;
  sessionId?: string;
  source?: ActivityTurnSource;
  /** Hex Nostr event ids of the channel events drained into this turn. */
  triggeringEventIds: string[];
}

export interface TurnActivitySummary {
  /** Distinct touched paths. */
  fileCount: number;
  readCount: number;
  editCount: number;
}

function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function frameTime(frame: ObserverEvent): number {
  const parsed = Date.parse(frame.timestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}

function turnSource(value: unknown): ActivityTurnSource | undefined {
  return value === "channel" || value === "heartbeat" ? value : undefined;
}

function triggeringEventIds(payload: Record<string, unknown>): string[] {
  const ids = payload.triggeringEventIds;
  if (!Array.isArray(ids)) return [];
  return ids.filter((id): id is string => typeof id === "string");
}

/**
 * Derive turn bounds from an agent's observer frames.
 *
 * A turn opens at its turn_started frame (or the first frame carrying its
 * turnId, for streams that joined mid-turn) and closes at turn_completed.
 * A turn with no turn_completed is closed at its last seen frame — unless it
 * is the newest turn in the stream, which stays open (endT undefined) so the
 * caller can treat it as live.
 */
export function deriveActivityTurns(
  frames: readonly ObserverEvent[],
): ActivityTurn[] {
  const turnsById = new Map<string, ActivityTurn>();
  const order: ActivityTurn[] = [];
  const lastSeenT = new Map<string, number>();
  const completed = new Set<string>();

  for (const frame of frames) {
    const turnId = frame.turnId;
    if (!turnId) continue;

    const t = frameTime(frame);
    let turn = turnsById.get(turnId);
    if (!turn) {
      turn = { id: turnId, startT: t, triggeringEventIds: [] };
      turnsById.set(turnId, turn);
      order.push(turn);
    }

    lastSeenT.set(turnId, t);
    if (frame.sessionId && turn.sessionId === undefined) {
      turn.sessionId = frame.sessionId;
    }

    if (frame.kind === "turn_started") {
      const payload = asRecord(frame.payload);
      turn.startT = t;
      turn.source = turnSource(payload.source) ?? turn.source;
      const ids = triggeringEventIds(payload);
      if (ids.length > 0) turn.triggeringEventIds = ids;
    } else if (frame.kind === "session_resolved") {
      const payload = asRecord(frame.payload);
      const sessionId = payload.sessionId;
      if (typeof sessionId === "string") turn.sessionId = sessionId;
    } else if (frame.kind === "turn_completed") {
      turn.endT = t;
      completed.add(turnId);
    }
  }

  // Close abandoned turns at their last frame; keep only the newest turn
  // open so a crashed/incomplete historical turn doesn't render as live.
  for (let index = 0; index < order.length; index += 1) {
    const turn = order[index];
    const isNewest = index === order.length - 1;
    if (turn.endT === undefined && !completed.has(turn.id) && !isNewest) {
      turn.endT = lastSeenT.get(turn.id) ?? turn.startT;
    }
  }

  return order;
}

/** Events belonging to one turn (matched by sourceTurnId). */
export function eventsForTurn(
  events: readonly ActivityEvent[],
  turnId: string,
): ActivityEvent[] {
  return events.filter((event) => event.sourceTurnId === turnId);
}

/**
 * Chip-style summary counts. Matches Berd's semantics: distinct paths for
 * the file count, writes+creates count as edits, reads as reads; pathless
 * status events contribute nothing.
 */
export function summarizeTurnActivity(
  events: readonly ActivityEvent[],
): TurnActivitySummary {
  const paths = new Set<string>();
  let readCount = 0;
  let editCount = 0;

  for (const event of events) {
    if (!event.path) continue;
    paths.add(event.path);
    if (event.kind === "r") readCount += 1;
    if (event.kind === "w" || event.kind === "c") editCount += 1;
  }

  return { fileCount: paths.size, readCount, editCount };
}
