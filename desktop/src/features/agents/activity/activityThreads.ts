// Thread scoping for the activity map.
//
// Observer frames carry no thread identity, but every channel-triggered
// turn_started frame carries the Nostr event ids of the messages that were
// drained into the turn (`triggeringEventIds`). Resolving the LAST of those
// ids (matching buzz-acp's "thread context follows the last reply" rule) to
// its thread root lets each thread's pill scope the map to that thread's
// turns instead of rendering the channel's entire history as one stream.
//
// Pure module: the event fetcher is injected so node tests never touch the
// Tauri bridge. The React wiring lives in useThreadScope.ts.

import { getThreadReference } from "@/features/messages/lib/threading";
import type { RelayEvent } from "@/shared/api/types";
import type { ObserverEvent } from "../ui/agentSessionTypes";
import type { ActivityTurn } from "./activityTurns";

/** Resolves an event id to the event, or null when unavailable/deleted. */
export type FetchEvent = (eventId: string) => Promise<RelayEvent | null>;

export interface TurnThreadResolution {
  /** Thread root event id; null for heartbeat/unresolvable turns. */
  rootId: string | null;
}

const UNTHREADED: TurnThreadResolution = { rootId: null };

/**
 * Resolve one turn to its thread root.
 *
 * Heartbeat turns and turns with no triggering events are unthreaded. A
 * triggering event that is itself a thread root (no thread e-tags) roots the
 * thread at that event; a reply resolves through its root marker. Fetch
 * failures degrade to unthreaded rather than erroring the map.
 *
 * One fetch per turn: the root id comes from the trigger's own tags, and
 * thread identity in the UI is positional (the pill sits on its thread's
 * summary row), so the root message body is never needed here.
 */
export async function resolveTurnThread(
  turn: ActivityTurn,
  fetchEvent: FetchEvent,
): Promise<TurnThreadResolution> {
  if (turn.source === "heartbeat" || turn.triggeringEventIds.length === 0) {
    return UNTHREADED;
  }
  const triggerId = turn.triggeringEventIds.at(-1);
  if (!triggerId) return UNTHREADED;

  const trigger = await fetchEvent(triggerId);
  if (!trigger) return UNTHREADED;

  return { rootId: getThreadReference(trigger.tags).rootId ?? trigger.id };
}

/**
 * Resolve a batch of turns in parallel. The fetcher is expected to cache and
 * dedupe by event id (see useThreadScope), so re-resolution on live appends
 * costs one map rebuild, not repeated I/O.
 */
export async function resolveTurnThreads(
  turns: readonly ActivityTurn[],
  fetchEvent: FetchEvent,
): Promise<Map<string, TurnThreadResolution>> {
  const entries = await Promise.all(
    turns.map(
      async (turn) =>
        [turn.id, await resolveTurnThread(turn, fetchEvent)] as const,
    ),
  );
  return new Map(entries);
}

export interface ThreadGroup {
  /** Thread root event id; null for the unthreaded (heartbeat) bucket. */
  rootId: string | null;
  /** Epoch ms of the group's most recent turn activity, for ordering. */
  lastT: number;
  turnCount: number;
  /** Member turn ids per agent pubkey — the scene-scoping allowlist. */
  turnIdsByAgent: Map<string, Set<string>>;
  /**
   * Agents whose newest turn in this group is still open (no turn_completed).
   * Intersect with the channel working signal for a live indicator: an open
   * turn alone can be a crashed agent, and a working agent may be working in
   * a different thread.
   */
  agentsWithOpenTurn: Set<string>;
}

export interface ThreadGroups {
  /** Resolved threads, most recently active first. */
  threads: ThreadGroup[];
  /** Heartbeat/unresolvable turns, or null when every turn resolved. */
  unthreaded: ThreadGroup | null;
}

function turnLastT(turn: ActivityTurn): number {
  return turn.endT ?? turn.startT;
}

/**
 * Group each agent's turns by resolved thread root.
 *
 * Turns missing from `resolutions` (still resolving) are excluded from every
 * group — they participate only in the unscoped "All" view until resolution
 * lands, which avoids flickering a live turn through the wrong bucket.
 */
export function buildThreadGroups(
  turnsByAgent: ReadonlyMap<string, readonly ActivityTurn[]>,
  resolutions: ReadonlyMap<string, TurnThreadResolution>,
): ThreadGroups {
  const groups = new Map<string | null, ThreadGroup>();

  for (const [agentId, turns] of turnsByAgent) {
    for (const turn of turns) {
      const resolution = resolutions.get(turn.id);
      if (!resolution) continue;

      let group = groups.get(resolution.rootId);
      if (!group) {
        group = {
          rootId: resolution.rootId,
          lastT: 0,
          turnCount: 0,
          turnIdsByAgent: new Map(),
          agentsWithOpenTurn: new Set(),
        };
        groups.set(resolution.rootId, group);
      }

      group.lastT = Math.max(group.lastT, turnLastT(turn));
      group.turnCount += 1;
      if (turn.endT === undefined) group.agentsWithOpenTurn.add(agentId);

      let agentTurns = group.turnIdsByAgent.get(agentId);
      if (!agentTurns) {
        agentTurns = new Set();
        group.turnIdsByAgent.set(agentId, agentTurns);
      }
      agentTurns.add(turn.id);
    }
  }

  const unthreaded = groups.get(null) ?? null;
  groups.delete(null);

  const threads = [...groups.values()].sort(
    (a, b) =>
      b.lastT - a.lastT || (a.rootId ?? "").localeCompare(b.rootId ?? ""),
  );

  return { threads, unthreaded };
}

/**
 * Frames belonging to the allowed turns. Frames with no turnId (connection
 * and pre-turn session frames) carry no activity and drop out of a scoped
 * view; the unscoped view never calls this.
 */
export function filterFramesByTurnIds(
  frames: readonly ObserverEvent[],
  allowedTurnIds: ReadonlySet<string>,
): ObserverEvent[] {
  return frames.filter(
    (frame) => frame.turnId !== null && allowedTurnIds.has(frame.turnId),
  );
}
