import React from "react";

import { sendSpawnerPromptUpdate } from "@/shared/api/spawnerRelay";
import type { SpawnerPromptMaterial } from "@/shared/api/tauriSpawner";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";

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

function storageKey(): string {
  return `buzz:spawner-prompt-queue:${getCachedRelayOrigin() ?? ""}`;
}

const listeners = new Set<() => void>();

let queue: ReadonlyMap<string, QueueEntry> = readStored();

const EMPTY: ReadonlyMap<string, QueueEntry> = new Map();

function readStored(): ReadonlyMap<string, QueueEntry> {
  try {
    const raw = window.localStorage.getItem(storageKey());
    if (!raw) return new Map();
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return new Map();
    return new Map(Object.entries(parsed as Record<string, QueueEntry>));
  } catch {
    return new Map();
  }
}

function persist(): void {
  try {
    window.localStorage.setItem(
      storageKey(),
      JSON.stringify(Object.fromEntries(queue)),
    );
  } catch {
    // Keep the in-memory queue so this session still works.
  }
}

function dispatch(action: QueueAction): void {
  const next = queueReducer(queue, action);
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
  });
}

/**
 * Clear a pending entry once the spawner's status echoes back the matching
 * `prompt_hash`. A stale or mismatched hash (an older status revision, or a
 * hash from a since-superseded edit) leaves the entry pending.
 */
export function ackSpawnerPromptUpdate(
  spawnerPubkey: string,
  specSlug: string,
  promptHash: string | null | undefined,
): void {
  if (!promptHash) return;
  for (const [key, entry] of queue) {
    if (entry.spawnerPubkey === spawnerPubkey && entry.specSlug === specSlug) {
      dispatch({ type: "ack", key, promptHash });
      return;
    }
  }
}

/**
 * Resend every pending prompt update.
 *
 * Called when a spawner is seen alive again (status or announcement ingest),
 * since a send that failed while it was unreachable never got a chance to
 * land.
 */
export async function retryPendingSpawnerPromptUpdates(): Promise<void> {
  for (const [key, entry] of queue) {
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
      });
    } catch (error) {
      console.debug(
        "[spawner-prompt-queue] retry send failed, left pending:",
        error,
      );
    }
  }
}

/** Tear down the queue. Community-scoped: pending edits belong to that relay. */
export function resetSpawnerPromptUpdateQueue(): void {
  dispatch({ type: "reset" });
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ReadonlyMap<string, QueueEntry> {
  return queue;
}

function getServerSnapshot(): ReadonlyMap<string, QueueEntry> {
  return EMPTY;
}

/** Reactive pending state for one agent's prompt update, or null when none. */
export function usePendingSpawnerPromptUpdate(
  agentPubkey: string,
): { pending: boolean; queuedAt: number } | null {
  const snapshot = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    getServerSnapshot,
  );
  for (const entry of snapshot.values()) {
    if (entry.agentPubkey === agentPubkey) {
      return { pending: true, queuedAt: entry.queuedAt };
    }
  }
  return null;
}
