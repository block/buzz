import React from "react";

import { sendSpawnerPromptUpdate } from "@/shared/api/spawnerRelay";
import type { SpawnerPromptMaterial } from "@/shared/api/tauriSpawner";
import {
  getCachedRelayOrigin,
  subscribeRelayOrigin,
} from "@/shared/lib/mediaUrl";

/**
 * Pending prompt updates for server-hosted agents, keyed by
 * `"<spawnerPubkey>:<agentPubkey>"`.
 *
 * A prompt edit is sent immediately, but the spawner only confirms it applied
 * by echoing `prompt_hash` back on its next kind:30179 status — the WebSocket
 * publish acknowledges delivery to the relay, not that the agent picked up the
 * new prompt. Until that echo arrives (or the send itself fails), the entry
 * stays here so the UI can show "pending" and so a spawner that was briefly
 * offline gets the update resent once it is seen again.
 *
 * Module-level singleton, matching `spawnerStatusStore`/`spawnerAttestationStore`:
 * this has to survive navigation and outlive any one component.
 */

/** One prompt update awaiting confirmation. */
export type QueueEntry = {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  prompt: SpawnerPromptMaterial;
  /**
   * Hash of the last-sent prompt material, or `""` when the send itself
   * failed and no hash was ever returned. An empty hash can never match a
   * real `prompt_hash` echoed by a spawner, so the entry simply stays pending
   * until `retryPendingSpawnerPromptUpdates` sends it again.
   */
  promptHash: string;
  queuedAt: number;
  /**
   * When the last send attempt for this entry was made (successful or not).
   * Used to hold off resending a *delivered* entry — one with a non-empty
   * `promptHash` — while its status ack is merely still in flight. Without
   * this floor, every spawner announcement (which the Rust side republishes
   * as part of its own reconcile loop after applying a prompt update) would
   * retrigger a resend before the confirming status has had a chance to
   * arrive, forcing a needless repeat container restart each time.
   */
  lastSentAt: number;
};

export type QueueAction =
  | ({ type: "enqueue"; key: string } & Omit<QueueEntry, never>)
  | { type: "ack"; key: string; promptHash: string | null | undefined }
  | { type: "reset" };

/** Pure reducer over the pending-queue map. Exported so tests avoid Tauri. */
export function queueReducer(
  state: ReadonlyMap<string, QueueEntry>,
  action: QueueAction,
): ReadonlyMap<string, QueueEntry> {
  switch (action.type) {
    case "enqueue": {
      const { type: _type, key, ...entry } = action;
      const next = new Map(state);
      next.set(key, entry as QueueEntry);
      return next;
    }
    case "ack": {
      if (!action.promptHash) return state;
      const existing = state.get(action.key);
      if (!existing || existing.promptHash !== action.promptHash) return state;
      const next = new Map(state);
      next.delete(action.key);
      return next;
    }
    case "reset":
      return state.size === 0 ? state : new Map();
    default:
      return state;
  }
}

function queueKey(spawnerPubkey: string, agentPubkey: string): string {
  return `${spawnerPubkey}:${agentPubkey}`;
}

/**
 * Minimum time a *delivered* (non-empty `promptHash`) entry must sit unacked
 * before it is eligible for resend. Deliberately simple — no backoff curve —
 * this only needs to outlast the gap between "send succeeds" and "spawner's
 * next status publish echoes the hash back", which is normally seconds.
 */
const REDELIVER_FLOOR_MS = 3 * 60 * 1000;

/**
 * Whether a pending entry should be resent right now.
 *
 * A failed send (`promptHash === ""`) always qualifies — it never reached the
 * spawner. A delivered entry only qualifies once it has been unacked longer
 * than {@link REDELIVER_FLOOR_MS}, so a spawner's own reconcile-triggered
 * re-announcement (which fires right after a successful send, before the
 * confirming status can land) does not cause an immediate, needless resend.
 */
export function shouldRetryPromptUpdate(
  entry: QueueEntry,
  now: number,
): boolean {
  if (!entry.promptHash) return true;
  return now - entry.lastSentAt >= REDELIVER_FLOOR_MS;
}

/**
 * Storage key for the *current* relay.
 *
 * Resolved on every read and write, never captured: the cached relay origin is
 * still null at module-import time, so a key computed once at import would read
 * from a bare, community-less key and write to the real one — pending updates
 * would never survive a restart. Only ever called with a resolved origin (see
 * {@link currentQueue} and {@link persist}), so the empty-origin key — which
 * every community would share — is never touched.
 */
function storageKey(origin: string): string {
  return `buzz:spawner-prompt-queue:${origin}`;
}

const listeners = new Set<() => void>();

let queue: ReadonlyMap<string, QueueEntry> = new Map();
/**
 * Whether {@link queue} reflects storage for the current relay. Cleared by
 * {@link resetSpawnerPromptUpdateQueue} so the next access rehydrates from the
 * new community's key rather than carrying the old one's entries.
 */
let hydrated = false;

const EMPTY: ReadonlyMap<string, QueueEntry> = new Map();

/**
 * Merge storage into whatever is already in memory, newest-write-wins.
 *
 * Anything queued before the relay origin resolved was held in memory only (it
 * had nowhere safe to persist to), so hydration must not clobber it with the
 * stored map. Pure, and exported for tests.
 */
export function mergeHydrated(
  stored: ReadonlyMap<string, QueueEntry>,
  inMemory: ReadonlyMap<string, QueueEntry>,
): ReadonlyMap<string, QueueEntry> {
  if (inMemory.size === 0) return stored;
  if (stored.size === 0) return inMemory;
  const merged = new Map(stored);
  for (const [key, entry] of inMemory) merged.set(key, entry);
  return merged;
}

/**
 * The live queue, hydrating from storage on first access after a reset.
 *
 * Hydration is skipped — and crucially `hydrated` is *not* latched — while the
 * relay origin is unknown. `resetCommunityState` nulls the cached origin right
 * after resetting this queue and only refreshes it asynchronously, as does a
 * cold start; latching there would bind the session to the shared empty-origin
 * key, so the new community's persisted entries would never load and the next
 * write would overwrite the real key with a map built from the wrong one.
 */
function currentQueue(): ReadonlyMap<string, QueueEntry> {
  if (!hydrated) {
    const origin = getCachedRelayOrigin();
    if (origin === null) return queue;
    queue = mergeHydrated(readStored(origin), queue);
    hydrated = true;
  }
  return queue;
}

function readStored(origin: string): ReadonlyMap<string, QueueEntry> {
  try {
    const raw = window.localStorage.getItem(storageKey(origin));
    if (!raw) return new Map();
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return new Map();
    return new Map(Object.entries(parsed as Record<string, QueueEntry>));
  } catch {
    return new Map();
  }
}

function persist(): void {
  const origin = getCachedRelayOrigin();
  // No resolved origin means no key that belongs to this community. The
  // entries stay in memory and are flushed once the origin arrives.
  if (origin === null) return;
  try {
    window.localStorage.setItem(
      storageKey(origin),
      JSON.stringify(Object.fromEntries(queue)),
    );
  } catch {
    // Keep the in-memory queue so this session still works.
  }
}

// Hydrate (and flush anything queued in the meantime) as soon as the origin
// resolves, rather than waiting for the next queue access.
subscribeRelayOrigin(() => {
  if (hydrated || getCachedRelayOrigin() === null) return;
  const before = queue;
  if (currentQueue() === before && before.size === 0) return;
  persist();
  for (const listener of listeners) listener();
});

function dispatch(action: QueueAction): void {
  const next = queueReducer(currentQueue(), action);
  if (next === queue) return;
  queue = next;
  persist();
  for (const listener of listeners) listener();
}

/**
 * Enqueue and immediately send a prompt update.
 *
 * Latest-write-wins per `(spawnerPubkey, agentPubkey)`: a second edit before
 * the first is confirmed simply replaces the pending entry, since only the
 * newest prompt material matters.
 *
 * A send failure is swallowed here (no toast) — the entry is left pending so
 * `retryPendingSpawnerPromptUpdates` can resend it once the spawner is next
 * seen alive.
 */
export async function enqueueSpawnerPromptUpdate(input: {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  prompt: SpawnerPromptMaterial;
}): Promise<void> {
  const key = queueKey(input.spawnerPubkey, input.agentPubkey);
  const queuedAt = Date.now();

  let promptHash = "";
  try {
    promptHash = await sendSpawnerPromptUpdate(input);
  } catch (error) {
    console.debug(
      "[spawner-prompt-queue] send failed, left pending for retry:",
      error,
    );
  }

  dispatch({
    type: "enqueue",
    key,
    spawnerPubkey: input.spawnerPubkey,
    specSlug: input.specSlug,
    agentPubkey: input.agentPubkey,
    prompt: input.prompt,
    promptHash,
    queuedAt,
    lastSentAt: Date.now(),
  });
}

/**
 * Which pending entry a status event is acking.
 *
 * Matched on the agent pubkey the spawner reports, because that is what the
 * spawner itself routes prompt updates by. The slug is only a fallback for a
 * status that carries no agent pubkey yet: the slug a client queued under can
 * legitimately differ from the spawner's (it falls back to a name-derived slug
 * when the spec has not loaded, and a rename changes it), and matching on it
 * alone lets an ack be missed forever — which means an endless resend and a
 * container restart every few minutes.
 *
 * Exported for tests; pure over the queue map.
 */
export function findAckKey(
  state: ReadonlyMap<string, QueueEntry>,
  spawnerPubkey: string,
  agentPubkey: string | null | undefined,
  specSlug: string,
): string | null {
  let slugMatch: string | null = null;
  for (const [key, entry] of state) {
    if (entry.spawnerPubkey !== spawnerPubkey) continue;
    if (agentPubkey && entry.agentPubkey === agentPubkey) return key;
    if (slugMatch === null && entry.specSlug === specSlug) slugMatch = key;
  }
  return agentPubkey ? null : slugMatch;
}

/**
 * Clear a pending entry once the spawner's status echoes back the matching
 * `prompt_hash`. A stale or mismatched hash (an older status revision, or a
 * hash from a since-superseded edit) leaves the entry pending.
 */
export function ackSpawnerPromptUpdate(
  spawnerPubkey: string,
  agentPubkey: string | null | undefined,
  specSlug: string,
  promptHash: string | null | undefined,
): void {
  if (!promptHash) return;
  const key = findAckKey(currentQueue(), spawnerPubkey, agentPubkey, specSlug);
  if (key) dispatch({ type: "ack", key, promptHash });
}

/**
 * Resend prompt updates that actually need it.
 *
 * Called when a spawner is seen alive again (status or announcement ingest),
 * since a send that failed while it was unreachable never got a chance to
 * land. Entries that were already delivered and are merely awaiting their
 * status ack are skipped (see {@link shouldRetryPromptUpdate}) — the Rust
 * side republishes its kind:10180 announcement as part of applying a prompt
 * update, and resending on every one of those before the confirming status
 * arrives would force a repeat container restart each time.
 */
export async function retryPendingSpawnerPromptUpdates(): Promise<void> {
  const now = Date.now();
  for (const [key, entry] of currentQueue()) {
    if (!shouldRetryPromptUpdate(entry, now)) continue;
    try {
      const promptHash = await sendSpawnerPromptUpdate({
        spawnerPubkey: entry.spawnerPubkey,
        specSlug: entry.specSlug,
        agentPubkey: entry.agentPubkey,
        prompt: entry.prompt,
      });
      dispatch({
        type: "enqueue",
        key,
        ...entry,
        promptHash,
        lastSentAt: Date.now(),
      });
    } catch (error) {
      console.debug(
        "[spawner-prompt-queue] retry send failed, left pending:",
        error,
      );
    }
  }
}

/**
 * Tear down the in-memory queue at a community boundary.
 *
 * Deliberately does *not* persist: the pending entries belong to the relay
 * being left, and writing an empty map under its key would delete edits that
 * still need delivering when the user switches back. Storage is left untouched
 * and the next access rehydrates from whichever relay is then current.
 */
export function resetSpawnerPromptUpdateQueue(): void {
  hydrated = false;
  if (queue.size === 0) return;
  queue = new Map();
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ReadonlyMap<string, QueueEntry> {
  return currentQueue();
}

function getServerSnapshot(): ReadonlyMap<string, QueueEntry> {
  return EMPTY;
}

/**
 * Reactive pending state for one agent's prompt update, or null when none.
 *
 * `delivered` distinguishes the normal "sent, awaiting the spawner's status
 * echo" window from an entry whose send never left this device (`promptHash`
 * is `""`), which is the only case the UI may describe as the server being
 * unreachable.
 */
export function usePendingSpawnerPromptUpdate(
  agentPubkey: string,
): { pending: boolean; delivered: boolean; queuedAt: number } | null {
  const snapshot = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    getServerSnapshot,
  );
  for (const entry of snapshot.values()) {
    if (entry.agentPubkey === agentPubkey) {
      return {
        pending: true,
        delivered: entry.promptHash !== "",
        queuedAt: entry.queuedAt,
      };
    }
  }
  return null;
}
