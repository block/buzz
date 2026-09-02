/**
 * NIP-MR: tracks agent mentions that are waiting for an acknowledgement.
 *
 * An agent harness publishes a kind:44102 receipt the moment it decides what to
 * do with a mention — `accepted` when a turn is coming, `declined` with a reason
 * when it knowingly will not act. That covers every case where an agent is
 * *running*.
 *
 * It cannot cover the case where nothing is running. The relay is pure fan-out
 * with no delivery tracking: if the agent is not connected and subscribed at the
 * instant of fan-out, the mention is never delivered to anyone and no ack will
 * ever be published, by anyone, ever. Silence is the only observable.
 *
 * So the client waits. Every agent mention is registered here on send; acks
 * resolve it; and anything still unresolved after {@link MENTION_ACK_TIMEOUT_MS}
 * is reported as unacknowledged. That is the difference between "your message
 * went nowhere and you will never find out" and "Ada never picked this up".
 *
 * Community-scoped: reset via `resetPendingMentionAckStore()` in
 * `resetCommunityState()`. See AGENTS.md "Community Switching".
 */

import * as React from "react";

/**
 * How long to wait for an ack before calling a mention unacknowledged.
 *
 * The harness publishes its ack at queue-push time, so a live agent acks within
 * a round trip. This is deliberately far longer than that, because a managed
 * agent may still be *starting* when the mention is sent — the desktop launches
 * it as part of the send flow — and a premature "nobody picked this up" on an
 * agent that was merely booting is worse than a late one. Being slow and right
 * beats being fast and wrong here.
 */
export const MENTION_ACK_TIMEOUT_MS = 30_000;

/** Outcome for a single agent that was mentioned. */
export type MentionAckOutcome =
  | { kind: "accepted" }
  | { kind: "declined"; reason: string | null }
  /** No ack arrived before the timeout. */
  | { kind: "silent" };

export type PendingMentionState = {
  eventId: string;
  channelId: string;
  /** Agent pubkeys this message tagged. */
  agentPubkeys: string[];
  /** Resolved outcomes, keyed by agent pubkey. */
  outcomes: Map<string, MentionAckOutcome>;
  /** True once the timeout has fired. */
  settled: boolean;
};

type Entry = PendingMentionState & { timer: ReturnType<typeof setTimeout> };

export type MentionProblem = {
  declined: Array<{ pubkey: string; reason: string | null }>;
  silent: string[];
};

const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();

/**
 * Reference-stable snapshots for `useSyncExternalStore`.
 *
 * React compares snapshots by identity, so a getter that derives a fresh object
 * on every call re-renders forever. Problems are computed once per mutation and
 * cached here; the getter only reads. See AGENTS.md gotcha 7.
 */
const problemCache = new Map<string, MentionProblem | null>();

const EMPTY_IDS: string[] = [];

/**
 * Reference-stable per-channel lists of event ids that currently have a problem.
 *
 * Cached per channel rather than in a single slot: a one-slot cache keyed on the
 * last channel asked about hands back a fresh array whenever two channels
 * interleave calls, which under `useSyncExternalStore` is the "getSnapshot
 * should be cached" infinite render loop. Exactly one consumer exists today, so
 * a single slot happens to work — a second pane or thread panel would break it.
 * Derived from `problemCache`, so any change to that invalidates all of these.
 */
const problemIdsCache = new Map<string, string[]>();

function computeProblem(entry: Entry | undefined): MentionProblem | null {
  if (!entry?.settled) return null;
  if ([...entry.outcomes.values()].some((o) => o.kind === "accepted")) {
    return null;
  }

  const declined: Array<{ pubkey: string; reason: string | null }> = [];
  const silent: string[] = [];
  for (const pubkey of entry.agentPubkeys) {
    const outcome = entry.outcomes.get(pubkey);
    if (outcome?.kind === "declined") {
      declined.push({ pubkey, reason: outcome.reason });
    } else if (outcome?.kind === "silent") {
      silent.push(pubkey);
    }
  }
  if (declined.length === 0 && silent.length === 0) return null;
  return { declined, silent };
}

/** Recompute one entry's cached problem. Returns true if it changed. */
function refreshProblem(eventId: string): boolean {
  const next = computeProblem(entries.get(eventId));
  const prev = problemCache.get(eventId) ?? null;
  if (prev === null && next === null) return false;
  if (
    prev !== null &&
    next !== null &&
    prev.silent.length === next.silent.length &&
    prev.declined.length === next.declined.length &&
    prev.silent.every((p, i) => p === next.silent[i]) &&
    prev.declined.every(
      (d, i) =>
        d.pubkey === next.declined[i]?.pubkey &&
        d.reason === next.declined[i]?.reason,
    )
  ) {
    return false;
  }
  if (next === null) problemCache.delete(eventId);
  else problemCache.set(eventId, next);
  // The per-channel id lists are derived from problemCache, so any change to it
  // invalidates them. Cheap: recomputed lazily, and almost always empty.
  problemIdsCache.clear();
  return true;
}

function notify() {
  for (const listener of listeners) listener();
}

export function subscribePendingMentionAcks(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Record that `eventId` mentioned `agentPubkeys` and is awaiting acks.
 *
 * No-ops when `agentPubkeys` is empty — a mention of a human is not something
 * anyone is expected to acknowledge, and tracking it would produce a footer
 * telling the user their colleague did not respond within 30 seconds.
 */
export function registerPendingMention(
  eventId: string,
  channelId: string,
  agentPubkeys: readonly string[],
): void {
  if (agentPubkeys.length === 0) return;
  if (entries.has(eventId)) return;

  const timer = setTimeout(() => {
    const entry = entries.get(eventId);
    if (!entry) return;
    entry.settled = true;
    for (const pubkey of entry.agentPubkeys) {
      if (!entry.outcomes.has(pubkey)) {
        entry.outcomes.set(pubkey, { kind: "silent" });
      }
    }
    if (refreshProblem(eventId)) notify();
  }, MENTION_ACK_TIMEOUT_MS);

  entries.set(eventId, {
    eventId,
    channelId,
    agentPubkeys: [...agentPubkeys],
    outcomes: new Map(),
    settled: false,
    timer,
  });
  // A freshly registered mention is never a problem yet, so no notify: the
  // waiting state is invisible by design. Only outcomes are worth a render.
}

/**
 * Apply an incoming kind:44102 ack.
 *
 * `ackAuthor` is authenticated by the caller having verified the event
 * signature; this function additionally requires the author to be a pubkey the
 * message actually mentioned. The relay cannot check agent-ness, so any member
 * can publish a well-formed ack for someone else's message — requiring the
 * author to be in `agentPubkeys` makes such an ack inert rather than a way to
 * suppress another user's "no agent picked this up" warning.
 */
export function applyMentionAck(
  eventId: string,
  ackAuthor: string,
  status: string,
  reason: string | null,
): void {
  const entry = entries.get(eventId);
  if (!entry) return;
  if (!entry.agentPubkeys.includes(ackAuthor)) return;

  const outcome: MentionAckOutcome =
    status === "accepted" ? { kind: "accepted" } : { kind: "declined", reason };
  entry.outcomes.set(ackAuthor, outcome);

  // Every mentioned agent has reported — nothing left to wait for.
  const allReported = entry.outcomes.size >= entry.agentPubkeys.length;
  if (allReported) {
    clearTimeout(entry.timer);
    entry.settled = true;
  }
  const changed = refreshProblem(eventId);
  // Settled with nothing to show (someone accepted) — the entry can never
  // produce a problem again, so drop it rather than carrying it for the rest
  // of the session and re-scanning it on every timeline render.
  if (allReported && !problemCache.has(eventId)) {
    entries.delete(eventId);
  }
  if (changed) notify();
}

/**
 * Clear tracking for a message.
 *
 * Called when an agent actually replies: a reply is a stronger signal than any
 * ack, and a message that has been answered must never show an unacknowledged
 * footer regardless of what did or did not arrive on the ack path.
 */
export function clearPendingMention(eventId: string): void {
  const entry = entries.get(eventId);
  if (!entry) return;
  clearTimeout(entry.timer);
  entries.delete(eventId);
  const had = problemCache.delete(eventId);
  problemIdsCache.clear();
  if (had) notify();
}

export function getPendingMention(
  eventId: string,
): PendingMentionState | undefined {
  return entries.get(eventId);
}

/**
 * Agent pubkeys for `eventId` that need surfacing, or `null` when the message
 * is healthy.
 *
 * Returns `null` while still waiting and when at least one agent accepted —
 * an accepted mention is on its way and needs no UI. Only genuinely problematic
 * outcomes (declined, or silent past the timeout) produce a result.
 */
export function getMentionProblem(eventId: string): MentionProblem | null {
  return problemCache.get(eventId) ?? null;
}

export function getProblemEventIds(channelId: string): string[] {
  if (!channelId) return EMPTY_IDS;
  const cached = problemIdsCache.get(channelId);
  if (cached) return cached;

  const ids: string[] = [];
  for (const [eventId, entry] of entries) {
    if (entry.channelId === channelId && problemCache.get(eventId)) {
      ids.push(eventId);
    }
  }
  if (ids.length === 0) return EMPTY_IDS;
  problemIdsCache.set(channelId, ids);
  return ids;
}

export function useProblemEventIds(channelId: string): string[] {
  return React.useSyncExternalStore(subscribePendingMentionAcks, () =>
    getProblemEventIds(channelId),
  );
}

/** React binding for {@link getMentionProblem}. */
export function useMentionProblem(eventId: string) {
  return React.useSyncExternalStore(subscribePendingMentionAcks, () =>
    getMentionProblem(eventId),
  );
}

/**
 * Community-scoped reset. Wired into `resetCommunityState()`.
 *
 * Deliberately does not clear `listeners`: switching community remounts the
 * React tree by key, and subscribers remove themselves on unmount. Dropping the
 * listener set here would silently orphan any component that survives the
 * switch, leaving it subscribed to a store that can never notify it again.
 */
export function resetPendingMentionAckStore(): void {
  for (const entry of entries.values()) clearTimeout(entry.timer);
  entries.clear();
  problemCache.clear();
  problemIdsCache.clear();
  notify();
}
