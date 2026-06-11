import * as React from "react";

import {
  subscribeAgentObserverStore,
  getAgentObserverSnapshot,
} from "@/features/agents/observerRelayStore";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";

/** Mark a turn as possibly stale after 20s of no activity. */
const STALE_AFTER_MS = 20_000;
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

export type ActiveTurnInfo = {
  channelId: string;
  stale: boolean;
};

// Module-level state: agentPubkey → turnId → ActiveTurn
const activeTurnsByAgent = new Map<string, Map<string, ActiveTurn>>();
const listeners = new Set<() => void>();

// Track which observer events we've already processed (by seq per agent)
const lastProcessedSeq = new Map<string, number>();

let pruneInterval: ReturnType<typeof setInterval> | null = null;

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
}

function pruneExpired() {
  const now = Date.now();
  let changed = false;
  for (const [agentKey, agentTurns] of activeTurnsByAgent) {
    for (const [turnId, turn] of agentTurns) {
      if (now - turn.lastActivityAt > REMOVE_AFTER_MS) {
        agentTurns.delete(turnId);
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

function processEvent(agentPubkey: string, event: ObserverEvent) {
  const key = normalizePubkey(agentPubkey);
  const lastSeq = lastProcessedSeq.get(key) ?? 0;
  if (event.seq <= lastSeq) return;
  lastProcessedSeq.set(key, event.seq);

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

export function getActiveTurnsForAgent(
  agentPubkey: string | null | undefined,
): Map<string, ActiveTurnInfo> {
  if (!agentPubkey) return EMPTY_MAP;
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns || agentTurns.size === 0) return EMPTY_MAP;

  const now = Date.now();
  const result = new Map<string, ActiveTurnInfo>();
  for (const [turnId, turn] of agentTurns) {
    result.set(turnId, {
      channelId: turn.channelId,
      stale: now - turn.lastActivityAt > STALE_AFTER_MS,
    });
  }
  return result;
}

/** Convenience: returns just the set of channel IDs (ignoring stale flag). */
export function getActiveChannelsForAgent(
  agentPubkey: string | null | undefined,
): Set<string> {
  if (!agentPubkey) return EMPTY_SET;
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns || agentTurns.size === 0) return EMPTY_SET;
  return new Set([...agentTurns.values()].map((t) => t.channelId));
}

const EMPTY_MAP: Map<string, ActiveTurnInfo> = new Map();
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
 * Hook: returns a map of turnId → { channelId, stale } for the given agent.
 * Re-renders when the map changes. Use for detailed turn state display.
 */
export function useActiveAgentTurnsDetailed(
  agentPubkey: string | null | undefined,
): Map<string, ActiveTurnInfo> {
  const getSnapshot = React.useCallback(
    () => getActiveTurnsForAgent(agentPubkey),
    [agentPubkey],
  );

  return React.useSyncExternalStore(subscribeActiveAgentTurns, getSnapshot);
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
  lastProcessedSeq.clear();
  notifyListeners();
}
