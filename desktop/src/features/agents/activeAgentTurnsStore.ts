import * as React from "react";

import {
  subscribeAgentObserverStore,
  getAgentObserverSnapshot,
  compareObserverEvents,
} from "@/features/agents/observerRelayStore";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";

/** Harness emits turn_liveness every ~10s (BUZZ_ACP_TURN_LIVENESS_SECS). */
const LIVENESS_INTERVAL_MS = 10_000;
/** Remove a turn after this long with no activity. Tolerates one fully dropped
 * liveness ping plus slack before pruning a turn whose host died without
 * unwinding (kill -9 / crash) — the only case that reaches this bound, since
 * graceful exits clear via turn_completed and working turns refresh on every
 * stream event. Derived from the interval so it tracks if the interval changes. */
const REMOVE_AFTER_MS = LIVENESS_INTERVAL_MS * 2.5;
/** Pause pruning once EVERY tracked turn has gone this long without activity —
 * the "all at once" signature of a relay drop (flaky VPN), where liveness frames
 * stop arriving for all agents simultaneously. Set below REMOVE_AFTER_MS so the
 * pause engages before the 25s prune would wipe the badges. */
const FRAME_GAP_PAUSE_MS = LIVENESS_INTERVAL_MS * 2;
/** Maximum concurrent active turns tracked per agent (matches pool size). */
const MAX_TURNS_PER_AGENT = 4;
/** Cap on per-agent terminal tombstones (A's resurrection guard). Only the
 * most recently completed turns can be raced by a late liveness frame; older
 * ones are already below the watermark, so a small multiple of the live cap is
 * ample and keeps the map from growing across a long session. */
const MAX_TERMINAL_TOMBSTONES = MAX_TURNS_PER_AGENT * 4;
/** Interval for pruning stale/expired turns. */
const PRUNE_INTERVAL_MS = 5_000;

type ActiveTurn = {
  turnId: string;
  channelId: string;
  startedAt: number;
  lastActivityAt: number;
};

/** One working channel surfaced to the UI, anchored to the desktop clock. */
export type ActiveTurnSummary = {
  channelId: string;
  anchorAt: number;
};

/** One channel with active agent work, aggregated across agents. */
export type ActiveChannelTurnSummary = {
  channelId: string;
  anchorAt: number;
  agentCount: number;
  agentPubkeys: string[];
  agentNames?: string[];
};

// Module-level state: agentPubkey → turnId → ActiveTurn
const activeTurnsByAgent = new Map<string, Map<string, ActiveTurn>>();
const listeners = new Set<() => void>();

// Per-agent clock offset: the desktop clock minus the agent-host clock, in
// milliseconds. Estimated as the running minimum of
// (Date.now() - Date.parse(event.timestamp)) across that agent's events. The
// minimum converges on true skew minus the smallest network/processing delay
// seen — a monotonically tightening estimate immune to per-event jitter. While
// true skew is constant or shrinking it is conservative: elapsed under-reports
// by the minimum delay and never inflates. The minimum never loosens, so under
// GROWING skew (an NTP step forward, or the host clock drifting further behind
// mid-session) the stored estimate goes stale-too-small and elapsed can over-
// report — bounded by how far the skew grows, sub-second over a session. A
// turn's badge anchor is startedAt + offset: the agent's own start, translated
// into desktop-clock terms. Anchors are derived at read time so a later, tighter
// offset retroactively corrects every live turn — distinct agent starts then
// yield distinct anchors (no lockstep) and a turn started long ago anchors into
// the past (large elapsed) instead of resetting to Date.now().
const clockOffsetByAgent = new Map<string, number>();

// Cached snapshots for useSyncExternalStore reference stability.
// Only regenerated when the underlying turn map for an agent actually changes.
const cachedTurnSummaries = new Map<string, ActiveTurnSummary[]>();
let cachedChannelTurnSummaries: ActiveChannelTurnSummary[] | null = null;

// Composite watermark per agent: the newest observer event processed, by
// (timestamp, seq) ordering. An event is processed only if it is strictly
// newer than this — making full-buffer replays idempotent and post-restart
// streams (seq resets to 1, timestamp keeps climbing) handled for free.
const lastProcessed = new Map<string, ObserverEvent>();

// Per-agent record of when each turn terminally ended (turnId →
// terminal-event timestamp, in agent-host clock ms). endTurn hard-deletes a
// turn with no surviving record, so without this a late liveness frame for an
// already-completed turn would resurrect a dead badge. Resurrection (A) checks
// this: a turn is revived only if the recovered liveness is strictly newer
// than its recorded terminal timestamp.
const terminalAtByAgent = new Map<string, Map<string, number>>();

let pruneInterval: ReturnType<typeof setInterval> | null = null;

function invalidateCache(agentKey: string) {
  cachedTurnSummaries.delete(agentKey);
  cachedChannelTurnSummaries = null;
}

function notifyListeners() {
  for (const listener of listeners) {
    listener();
  }
}

/**
 * Refine this agent's clock-offset estimate from one observer event. Samples
 * Date.now() - Date.parse(timestamp) and keeps the running minimum. When the
 * minimum tightens, every live anchor for the agent shifts, so the cache is
 * invalidated. Events with an unparseable timestamp contribute no sample.
 * Returns true when the offset changed.
 */
function sampleClockOffset(agentKey: string, timestamp: string): boolean {
  const sample = Date.now() - Date.parse(timestamp);
  if (Number.isNaN(sample)) return false;
  const prior = clockOffsetByAgent.get(agentKey);
  if (prior !== undefined && sample >= prior) return false;
  clockOffsetByAgent.set(agentKey, sample);
  invalidateCache(agentKey);
  return true;
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

  const startedAt = Date.parse(timestamp) || Date.now();
  agentTurns.set(turnId, {
    turnId,
    channelId,
    startedAt,
    lastActivityAt: Date.now(),
  });
  invalidateCache(key);
}

function recordActivity(agentPubkey: string, turnId: string | null): boolean {
  if (!turnId) return false;
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns) return false;
  const turn = agentTurns.get(turnId);
  if (turn) {
    turn.lastActivityAt = Date.now();
    return true;
  }
  return false;
}

/**
 * A — resurrect a badge that was pruned out from under a still-running turn.
 * A recovered liveness/acp frame for a turn no longer in the live map recreates
 * it, UNLESS C's tombstone shows the turn already terminally ended at or after
 * this frame's time (a stale frame must not revive a completed turn). The frame
 * carries no record of the original start, so the badge re-anchors to this
 * frame's timestamp — it resumes counting from recovery, which is the honest
 * floor for a turn whose true start is unrecoverable. Returns true on revive.
 */
function resurrectTurn(agentPubkey: string, event: ObserverEvent): boolean {
  if (!event.turnId || !event.channelId) return false;
  const key = normalizePubkey(agentPubkey);
  const terminalAt = terminalAtByAgent.get(key)?.get(event.turnId);
  const frameAt = Date.parse(event.timestamp);
  // Only revive when this frame is strictly newer than the recorded terminal.
  if (
    terminalAt !== undefined &&
    (!Number.isFinite(frameAt) || frameAt <= terminalAt)
  ) {
    return false;
  }
  startTurn(agentPubkey, event.channelId, event.turnId, event.timestamp);
  return true;
}

function recordTerminal(agentKey: string, turnId: string, terminalAt: number) {
  if (!Number.isFinite(terminalAt)) return;
  let terminals = terminalAtByAgent.get(agentKey);
  if (!terminals) {
    terminals = new Map();
    terminalAtByAgent.set(agentKey, terminals);
  }
  terminals.set(turnId, terminalAt);
  // Bound the tombstone map: only recently-completed turns can be the target of
  // a racing late liveness frame (older ones are already below the watermark).
  // Evict the oldest terminal once past the cap so the map can't grow unbounded
  // across a long session. Insertion order tracks completion order closely
  // enough; the first key is the oldest survivor.
  if (terminals.size > MAX_TERMINAL_TOMBSTONES) {
    const oldest = terminals.keys().next().value;
    if (oldest !== undefined) terminals.delete(oldest);
  }
}

function endTurn(
  agentPubkey: string,
  turnId: string | null,
  channelId: string | null,
  terminalAt: number,
) {
  const key = normalizePubkey(agentPubkey);
  // Tombstone the terminal time so a late liveness frame can't resurrect a
  // completed turn (A's guard). With an explicit turnId this is recorded even
  // when the turn was already pruned and the agent's live map is gone — the
  // completion is authoritative and must outlive the active record.
  if (turnId) {
    recordTerminal(key, turnId, terminalAt);
  }

  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns) return;

  if (turnId) {
    agentTurns.delete(turnId);
  } else if (channelId) {
    // Fallback: remove by channelId if turnId not available. Tombstone the
    // resolved turn so a later stale liveness for it can't resurrect a badge.
    for (const [tid, turn] of agentTurns) {
      if (turn.channelId === channelId) {
        agentTurns.delete(tid);
        recordTerminal(key, tid, terminalAt);
        break;
      }
    }
  }
  if (agentTurns.size === 0) {
    activeTurnsByAgent.delete(key);
  }
  invalidateCache(key);
}

/** True when every tracked turn across every agent is simultaneously stale —
 * no turn has had activity within FRAME_GAP_PAUSE_MS. With no tracked turns
 * there is nothing to prune, so it returns false (never pause). */
function shouldPausePrune(now: number): boolean {
  let maxActivity = 0;
  for (const agentTurns of activeTurnsByAgent.values())
    for (const turn of agentTurns.values())
      if (turn.lastActivityAt > maxActivity) maxActivity = turn.lastActivityAt;
  return maxActivity > 0 && now - maxActivity > FRAME_GAP_PAUSE_MS;
}

function pruneExpired() {
  const now = Date.now();
  // Pause pruning when ALL tracked turns are simultaneously stale — the "all
  // at once" signature of a relay drop, where every agent's liveness stops in
  // the same instant. Gating on the MAX lastActivityAt (not a global frame
  // clock) is what keeps this from over-pausing: a single live sibling turn
  // keeps the max fresh, so a genuinely dead turn still prunes at 25s — no
  // regression for the multi-agent crash case. Residual: a LONE turn kill -9'd
  // under a HEALTHY relay (it was the only active turn) keeps its badge until
  // the next frame instead of clearing at 25s, since local-only sensing cannot
  // distinguish that from a drop. The badge self-heals the instant any frame
  // arrives. Accepted tradeoff to keep badges visible through transient drops.
  if (shouldPausePrune(now)) {
    return;
  }
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
  // (the turn_liveness/acp case below) is gated here too: it runs only for a
  // frame that passes the watermark, so replayed stale frames cannot revive a
  // pruned turn, and the per-turn terminal tombstone blocks reviving a turn
  // that already completed.
  const last = lastProcessed.get(key);
  if (last && compareObserverEvents(event, last) <= 0) {
    return;
  }
  lastProcessed.set(key, event);

  // Refine the clock offset from every fresh event. A tighter offset shifts
  // every live anchor for this agent, so a change must reach the UI even when
  // the event itself surfaces no new turn.
  const offsetChanged = sampleClockOffset(key, event.timestamp);

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
        return;
      }
      break;
    case "turn_completed":
    case "turn_error":
    case "agent_panic":
      endTurn(
        agentPubkey,
        event.turnId ?? null,
        event.channelId ?? null,
        Date.parse(event.timestamp),
      );
      notifyListeners();
      return;
    case "acp_read":
    case "acp_write":
    // turn_liveness keeps a quiet-but-alive turn from being pruned; same
    // refresh-only path as stream activity — no surfaced summary change on its
    // own, so it only notifies when the offset above actually moved. If the
    // turn was pruned out from under a still-running host (a transient drop
    // raced the pause, or the lone-crash residual self-healed), resurrect it.
    case "turn_liveness": {
      const refreshed = recordActivity(agentPubkey, event.turnId ?? null);
      if (!refreshed && resurrectTurn(agentPubkey, event)) {
        notifyListeners();
        return;
      }
      break;
    }
  }

  if (offsetChanged) {
    notifyListeners();
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

/**
 * Returns the channels where the given agent has active turns, sorted by
 * channelId, each anchored to the earliest `anchorAt` for that channel.
 * The array reference is cached and stable until the turn map mutates — a
 * requirement for `useSyncExternalStore`.
 */
export function getActiveTurnsForAgent(
  agentPubkey: string | null | undefined,
): ActiveTurnSummary[] {
  if (!agentPubkey) return EMPTY_TURNS;
  const key = normalizePubkey(agentPubkey);
  const agentTurns = activeTurnsByAgent.get(key);
  if (!agentTurns || agentTurns.size === 0) return EMPTY_TURNS;

  const cached = cachedTurnSummaries.get(key);
  if (cached) return cached;

  const offset = clockOffsetByAgent.get(key) ?? 0;

  // Collapse multiple turns in one channel to the earliest start — the badge
  // should count from when the channel's oldest live turn began. Anchors are
  // derived here (startedAt + offset) so the latest skew estimate applies.
  const earliestByChannel = new Map<string, number>();
  for (const turn of agentTurns.values()) {
    const prior = earliestByChannel.get(turn.channelId);
    if (prior === undefined || turn.startedAt < prior) {
      earliestByChannel.set(turn.channelId, turn.startedAt);
    }
  }

  const result = [...earliestByChannel.entries()]
    .map(([channelId, startedAt]) => ({
      channelId,
      anchorAt: startedAt + offset,
    }))
    .sort((a, b) => a.channelId.localeCompare(b.channelId));
  cachedTurnSummaries.set(key, result);
  return result;
}

const EMPTY_TURNS: ActiveTurnSummary[] = [];
const EMPTY_CHANNEL_TURNS: ActiveChannelTurnSummary[] = [];

/**
 * Returns active working channels across all tracked agents, sorted by
 * channelId and anchored to the earliest live turn in each channel.
 */
export function getActiveTurnsByChannel(): ActiveChannelTurnSummary[] {
  if (cachedChannelTurnSummaries) return cachedChannelTurnSummaries;
  if (activeTurnsByAgent.size === 0) return EMPTY_CHANNEL_TURNS;

  const summaries = new Map<
    string,
    { anchorAt: number; agentPubkeys: Set<string> }
  >();

  for (const [agentKey, agentTurns] of activeTurnsByAgent) {
    if (agentTurns.size === 0) continue;
    const offset = clockOffsetByAgent.get(agentKey) ?? 0;

    for (const turn of agentTurns.values()) {
      const anchorAt = turn.startedAt + offset;
      const summary = summaries.get(turn.channelId);
      if (!summary) {
        summaries.set(turn.channelId, {
          anchorAt,
          agentPubkeys: new Set([agentKey]),
        });
        continue;
      }

      summary.agentPubkeys.add(agentKey);
      if (anchorAt < summary.anchorAt) {
        summary.anchorAt = anchorAt;
      }
    }
  }

  const result = [...summaries.entries()]
    .map(([channelId, summary]) => ({
      channelId,
      anchorAt: summary.anchorAt,
      agentCount: summary.agentPubkeys.size,
      agentPubkeys: [...summary.agentPubkeys].sort(),
    }))
    .sort((a, b) => a.channelId.localeCompare(b.channelId));
  cachedChannelTurnSummaries = result;
  return result;
}

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
 * Hook: returns the channels where the given agent is currently working, each
 * with the desktop-clock `anchorAt` to anchor a live elapsed counter.
 * Re-renders when the set of channels changes — not when the clock ticks.
 */
export function useActiveAgentTurns(
  agentPubkey: string | null | undefined,
): ActiveTurnSummary[] {
  const getSnapshot = React.useCallback(
    () => getActiveTurnsForAgent(agentPubkey),
    [agentPubkey],
  );

  return React.useSyncExternalStore(subscribeActiveAgentTurns, getSnapshot);
}

/**
 * Hook: returns channels with active agent work across all tracked agents.
 * Re-renders when the channel set changes — not when the clock ticks.
 */
export function useActiveAgentTurnsByChannel(): ActiveChannelTurnSummary[] {
  return React.useSyncExternalStore(
    subscribeActiveAgentTurns,
    getActiveTurnsByChannel,
  );
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
  clockOffsetByAgent.clear();
  cachedTurnSummaries.clear();
  cachedChannelTurnSummaries = null;
  terminalAtByAgent.clear();
  notifyListeners();
}
