import * as React from "react";

import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  type ActiveChannelTurnSummary,
  getActiveTurnsByChannel,
  getActiveTurnsForAgent,
  subscribeActiveAgentTurns,
} from "./activeAgentTurnsStore";

/**
 * Unified "agent is working" signal.
 *
 * Every surface that shows a working affordance (sidebar channel badges,
 * profile badges, agent rows, composer activity bar, activity panel header,
 * future thread ingresses) should read from this module instead of picking
 * one of the underlying pipes.
 *
 * Working = observer-derived active turns only (kind 24200 →
 * activeAgentTurnsStore). Those frames are emitted when a real harness turn
 * is running on a contentful event (kind 9 / other subscribed kinds).
 *
 * Bot typing indicators (kind 20002) are intentionally NOT a working signal.
 * Typing is empty UX ephemera — humans type while composing, and harnesses
 * may emit typing while already working. Folding typing into "is working…"
 * produced false spinners on empty typing pings and double-counted real turns.
 * Human/bot "is typing…" chrome still reads typing entries directly.
 *
 * Scope rule: with a channelId, "working" means working in that channel;
 * without one, "working" means any active work in any channel (the
 * all-channels rule the activity panel uses).
 */

export type AgentWorkingSource = "observer" | "typing" | "none";

export type AgentWorkingChannel = {
  channelId: string;
  /** Desktop-clock anchor for elapsed displays (turn start / first typing). */
  anchorAt: number;
  source: Exclude<AgentWorkingSource, "none">;
};

export type AgentWorkingState = {
  working: boolean;
  /** Strongest signal backing `working` for the requested scope. */
  source: AgentWorkingSource;
  /** Every channel the agent is working in (unscoped), observer-primary. */
  channels: AgentWorkingChannel[];
};

export type WorkingChannelSummary = ActiveChannelTurnSummary & {
  source: Exclude<AgentWorkingSource, "none">;
};

const IDLE_STATE: AgentWorkingState = {
  working: false,
  source: "none",
  channels: [],
};

// ── Typing registry (fallback input) ────────────────────────────────────────
// channelId → (normalized agent pubkey → first-seen ms). Fed by
// reportChannelBotTyping from the channel typing hooks; entries follow the
// typing TTL because the hooks re-report whenever their entries change.
const typingByChannel = new Map<string, Map<string, number>>();

const listeners = new Set<() => void>();
let unsubscribeTurns: (() => void) | null = null;

// Reference-stable snapshots for useSyncExternalStore. React reads a snapshot
// before it subscribes, so these must be stable even with no listeners yet.
const stateCache = new Map<string, AgentWorkingState>();
let channelsCache: WorkingChannelSummary[] | null = null;
const channelPubkeysCache = new Map<string, string[]>();

function invalidateCaches() {
  stateCache.clear();
  channelsCache = null;
  channelPubkeysCache.clear();
}

function notify() {
  invalidateCaches();
  for (const listener of listeners) {
    listener();
  }
}

export function subscribeAgentWorkingSignal(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1) {
    invalidateCaches();
    unsubscribeTurns = subscribeActiveAgentTurns(notify);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      unsubscribeTurns?.();
      unsubscribeTurns = null;
    }
  };
}

/**
 * Legacy no-op input path retained for call sites that still mirror bot typing.
 * Typing is no longer folded into the working signal (observer turns only).
 * Call sites may keep reporting; entries are ignored for `working` state.
 */
export function reportChannelBotTyping(
  channelId: string,
  pubkeys: readonly string[],
) {
  const current = typingByChannel.get(channelId);
  const next = new Map<string, number>();
  const now = Date.now();
  for (const pubkey of pubkeys) {
    const key = normalizePubkey(pubkey);
    next.set(key, current?.get(key) ?? now);
  }

  const unchanged =
    (current?.size ?? 0) === next.size &&
    [...next.keys()].every((key) => current?.has(key));
  if (unchanged) {
    return;
  }

  if (next.size === 0) {
    typingByChannel.delete(channelId);
  } else {
    typingByChannel.set(channelId, next);
  }
  notify();
}

function computeAgentWorkingState(
  agentPubkey: string,
  channelId: string | null,
): AgentWorkingState {
  const key = normalizePubkey(agentPubkey);
  const turns = getActiveTurnsForAgent(key);

  const channels: AgentWorkingChannel[] = turns.map((turn) => ({
    channelId: turn.channelId,
    anchorAt: turn.anchorAt,
    source: "observer" as const,
  }));

  if (channels.length === 0) {
    return IDLE_STATE;
  }

  channels.sort((a, b) => a.channelId.localeCompare(b.channelId));

  const scoped =
    channelId === null
      ? channels
      : channels.filter((channel) => channel.channelId === channelId);
  const source: AgentWorkingSource =
    scoped.length > 0 ? "observer" : "none";

  return { working: source !== "none", source, channels };
}

/**
 * Working state for one agent, optionally scoped to a channel. Returns a
 * reference-stable snapshot while subscribed (useSyncExternalStore-safe).
 */
export function getAgentWorkingState(
  agentPubkey: string | null | undefined,
  channelId: string | null = null,
): AgentWorkingState {
  if (!agentPubkey) {
    return IDLE_STATE;
  }
  const cacheKey = `${normalizePubkey(agentPubkey)}|${channelId ?? ""}`;
  const cached = stateCache.get(cacheKey);
  if (cached) {
    return cached;
  }
  const state = computeAgentWorkingState(agentPubkey, channelId);
  stateCache.set(cacheKey, state);
  return state;
}

/**
 * All channels with agent work in progress, aggregated across agents from
 * observer-derived active turns only.
 */
export function getWorkingChannels(): WorkingChannelSummary[] {
  if (channelsCache) {
    return channelsCache;
  }

  const byChannel = new Map<string, WorkingChannelSummary>();
  for (const summary of getActiveTurnsByChannel()) {
    byChannel.set(summary.channelId, { ...summary, source: "observer" });
  }

  const result = [...byChannel.values()].sort((a, b) =>
    a.channelId.localeCompare(b.channelId),
  );
  channelsCache = result;
  return result;
}

const EMPTY_PUBKEYS: string[] = [];

/**
 * Normalized pubkeys of every agent with an observer-backed active turn in
 * the given channel. Stable while subscribed.
 */
export function getWorkingAgentPubkeysForChannel(
  channelId: string | null | undefined,
): string[] {
  if (!channelId) {
    return EMPTY_PUBKEYS;
  }
  const cached = channelPubkeysCache.get(channelId);
  if (cached) {
    return cached;
  }
  const merged = new Set<string>();
  for (const summary of getActiveTurnsByChannel()) {
    if (summary.channelId !== channelId) {
      continue;
    }
    for (const pubkey of summary.agentPubkeys) {
      merged.add(normalizePubkey(pubkey));
    }
  }
  const result = merged.size === 0 ? EMPTY_PUBKEYS : [...merged].sort();
  channelPubkeysCache.set(channelId, result);
  return result;
}

// ── Hooks ────────────────────────────────────────────────────────────────────

/** Working state for one agent, optionally scoped to a channel. */
export function useAgentWorking(
  agentPubkey: string | null | undefined,
  channelId: string | null = null,
): AgentWorkingState {
  return React.useSyncExternalStore(subscribeAgentWorkingSignal, () =>
    getAgentWorkingState(agentPubkey, channelId),
  );
}

/** All channels with agent work in progress, across agents. */
export function useWorkingChannels(): WorkingChannelSummary[] {
  return React.useSyncExternalStore(
    subscribeAgentWorkingSignal,
    getWorkingChannels,
  );
}

/** Normalized pubkeys of agents working in a channel. */
export function useChannelWorkingAgentPubkeys(
  channelId: string | null | undefined,
): string[] {
  return React.useSyncExternalStore(subscribeAgentWorkingSignal, () =>
    getWorkingAgentPubkeysForChannel(channelId),
  );
}

/** Community-switch reset (see resetCommunityState in useCommunityInit). */
export function resetAgentWorkingSignal() {
  typingByChannel.clear();
  invalidateCaches();
  for (const listener of listeners) {
    listener();
  }
}
