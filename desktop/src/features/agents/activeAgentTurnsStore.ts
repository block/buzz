import * as React from "react";

import {
  subscribeAgentObserverStore,
  getAgentObserverSnapshot,
  compareObserverEvents,
} from "@/features/agents/observerRelayStore";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";

/** Remove a turn entirely after 90s of no activity. */
const REMOVE_AFTER_MS = 90_000;
/** Maximum concurrent active turns tracked per agent (matches pool size). */
const MAX_TURNS_PER_AGENT = 4;
/** Interval for pruning stale/expired turns. */
const PRUNE_INTERVAL_MS = 5_000;

type ActiveTurn = {
  turnId: string;
  channelId: string;
  startedAt: number;
  lastActivityAt: number;
};

// Module-level state: agentPubkey → turnId → ActiveTurn
const activeTurnsByAgent = new Map<string, Map<string, ActiveTurn>>();
const listeners = new Set<() => void>();

// Cached snapshots for useSyncExternalStore reference stability.
// Only regenerated when the underlying turn map for an agent actually changes.
const cachedChannelSets = new Map<string, Set<string>>();

// Composite watermark per agent: the newest observer event processed, by
// (timestamp, seq) ordering. An event is processed only if it is strictly
// newer than this — making full-buffer replays idempotent and post-restart
// streams (seq resets to 1, timestamp keeps climbing) handled for free.
const lastProcessed = new Map<string, ObserverEvent>();

let pruneInterval: ReturnType<typeof setInterval> | null = null;

function invalidateCache(agentKey: string) {
  cachedChannelSets.delete(agentKey);
}

function notifyListeners() {
  for (const listener of listeners) {
    listener();
  }
}

function startTurn(
  agentPubkey: string,
  channelId: string,
  turnId: string,
  timestamp: string,
) {
  const key = normalizePubkey(agentPubkey);
  let agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns) {
    agentTurns = new Map();
    activeTurnsByAgent.set(key, agentTurns);
  }

  // Cap at MAX_TURNS_PER_AGENT — evict oldest if exceeded
  if (agentTurns.size >= MAX_TURNS_PER_AGENT && !agentTurns.has(turnId)) {
    let oldestKey: string | null = null;
    let oldestTime = Number.POSITIVE_INFINITY;
    for (const [tid, turn] of agentTurns) {
      if (turn.startedAt < oldestTime) {
        oldestTime = turn.startedAt;
        oldestKey = tid;
      }
    }
    if (oldestKey) {
      agentTurns.delete(oldestKey);
    }
  }

  const now = Date.parse(timestamp) || Date.now();
  agentTurns.set(turnId, {
    turnId,
    channelId,
    startedAt: now,
    lastActivityAt: now,
  });
  invalidateCache(key);
}

function recordActivity(agentPubkey: string, turnId: string | null) {
  if (!turnId) return;
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns) return;
  const turn = agentTurns.get(turnId);
  if (turn) {
    turn.lastActivityAt = Date.now();
  }
}

function endTurn(
  agentPubkey: string,
  turnId: string | null,
  channelId: string | null,
) {
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns) return;

  if (turnId) {
    agentTurns.delete(turnId);
  } else if (channelId) {
    // Fallback: remove by channelId if turnId not available
    for (const [tid, turn] of agentTurns) {
      if (turn.channelId === channelId) {
        agentTurns.delete(tid);
        break;
      }
    }
  }
  if (agentTurns.size === 0) {
    activeTurnsByAgent.delete(key);
  }
  invalidateCache(key);
}

function pruneExpired() {
  const now = Date.now();
  let changed = false;
  for (const [agentKey, agentTurns] of activeTurnsByAgent) {
    for (const [turnId, turn] of agentTurns) {
      if (now - turn.lastActivityAt > REMOVE_AFTER_MS) {
        agentTurns.delete(turnId);
        invalidateCache(agentKey);
        changed = true;
      }
    }
    if (agentTurns.size === 0) {
      activeTurnsByAgent.delete(agentKey);
    }
  }
  if (changed) {
    notifyListeners();
  }
}

// INVARIANT: events must be sorted by (timestamp, seq) ascending.
// syncAgentTurnsFromEvents receives sorted arrays from observerRelayStore.
// Calling with unsorted events will cause silent data loss.
function processEvent(agentPubkey: string, event: ObserverEvent) {
  const key = normalizePubkey(agentPubkey);

  // Gate every event kind on the watermark uniformly: process only events
  // strictly newer than the last one seen for this agent. With sorted buffers
  // (the documented invariant), this makes full-buffer replays a complete
  // no-op. Evictions must be gated too — replaying a stale turn_error/
  // agent_panic (emitted with a null turnId) would otherwise fall back to
  // deleting the first turn in the channel, killing the live turn. Resurrection
  // is not a concern: it would require reprocessing a stale start, which the
  // watermark already blocks.
  const last = lastProcessed.get(key);
  if (last && compareObserverEvents(event, last) <= 0) {
    return;
  }
  lastProcessed.set(key, event);

  switch (event.kind) {
    case "turn_started":
      if (event.channelId) {
        startTurn(
          agentPubkey,
          event.channelId,
          event.turnId ?? `seq-${event.seq}`,
          event.timestamp,
        );
        notifyListeners();
      }
      break;
    case "turn_completed":
    case "turn_error":
    case "agent_panic":
      endTurn(agentPubkey, event.turnId ?? null, event.channelId ?? null);
      notifyListeners();
      break;
    case "acp_read":
    case "acp_write":
      recordActivity(agentPubkey, event.turnId ?? null);
      break;
  }
}

function ensurePruneInterval() {
  if (pruneInterval) return;
  pruneInterval = setInterval(pruneExpired, PRUNE_INTERVAL_MS);
}

function stopPruneInterval() {
  if (pruneInterval) {
    clearInterval(pruneInterval);
    pruneInterval = null;
  }
}

// ─── Public API ──────────────────────────────────────────────────────────────

export function subscribeActiveAgentTurns(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    ensurePruneInterval();
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      stopPruneInterval();
    }
  };
}

/** Returns the set of channel IDs where the given agent has active turns. */
export function getActiveChannelsForAgent(
  agentPubkey: string | null | undefined,
): Set<string> {
  if (!agentPubkey) return EMPTY_SET;
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns || agentTurns.size === 0) return EMPTY_SET;

  const cached = cachedChannelSets.get(key);
  if (cached) return cached;

  const result = new Set([...agentTurns.values()].map((t) => t.channelId));
  cachedChannelSets.set(key, result);
  return result;
}

const EMPTY_SET: Set<string> = new Set();

/**
 * Synchronize the active-turns store with the latest observer events for a
 * given agent.
 */
export function syncAgentTurnsFromEvents(
  agentPubkey: string,
  events: ObserverEvent[],
) {
  for (const event of events) {
    processEvent(agentPubkey, event);
  }
}

/**
 * Hook: returns the set of channel IDs where the given agent is currently working.
 * Re-renders when the set changes.
 */
export function useActiveAgentTurns(
  agentPubkey: string | null | undefined,
): Set<string> {
  const getSnapshot = React.useCallback(
    () => getActiveChannelsForAgent(agentPubkey),
    [agentPubkey],
  );

  return React.useSyncExternalStore(subscribeActiveAgentTurns, getSnapshot);
}

/**
 * Bridge hook: processes observer events into the active-turns store.
 * Should be called by a parent component that has access to the observer events.
 */
export function useActiveAgentTurnsBridge(
  agents: readonly { pubkey: string; status: string }[],
) {
  React.useEffect(() => {
    function syncAll() {
      for (const agent of agents) {
        if (agent.status !== "running" && agent.status !== "deployed") continue;
        const snapshot = getAgentObserverSnapshot(agent.pubkey, true);
        syncAgentTurnsFromEvents(agent.pubkey, snapshot.events);
      }
    }

    syncAll();
    return subscribeAgentObserverStore(syncAll);
  }, [agents]);
}

export function resetActiveAgentTurnsStore() {
  activeTurnsByAgent.clear();
  lastProcessed.clear();
  cachedChannelSets.clear();
  notifyListeners();
}
