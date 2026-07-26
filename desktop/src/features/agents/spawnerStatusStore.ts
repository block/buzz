import React from "react";

import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  parseSpawnerStatus,
  specSlugFromEvent,
  subscribeToSpawnerStatus,
  type SpawnerAgentStatus,
} from "@/shared/api/spawnerRelay";
import type { RelayEvent } from "@/shared/api/types";
import { ackSpawnerPromptUpdate } from "@/features/agents/spawnerPromptUpdateQueue";

/**
 * Live status for this owner's server-hosted agents, keyed by
 * `"<spawnerPubkey>/<slug>"`.
 *
 * # Why the key includes the spawner
 *
 * Kind 30179 is NIP-33 addressed by `(pubkey, kind, d_tag)`, so the *author* is
 * part of the identity of a status document. Keying on the slug alone would let
 * any pubkey that publishes a 30179 with a matching slug overwrite the real
 * spawner's status in this map — the relay correctly stores them at separate
 * addresses, and collapsing them here would undo that. Callers look up the
 * spawner they addressed the spec to.
 */

const listeners = new Set<() => void>();

let statuses: ReadonlyMap<string, SpawnerAgentStatus> = new Map();
let unsubscribeRelay: (() => Promise<void>) | null = null;
let startPromise: Promise<void> | null = null;

/**
 * Newest `created_at` seen per key.
 *
 * A reconnect can replay an older revision after a newer one has landed.
 * Without this guard a stale `pending_attestation` could clobber a live
 * `running`, and the UI would show an agent as stuck when it is fine.
 */
const latestCreatedAt = new Map<string, number>();

const EMPTY: ReadonlyMap<string, SpawnerAgentStatus> = new Map();

/** Compose the lookup key for a status entry. */
export function spawnerStatusKey(spawnerPubkey: string, slug: string): string {
  return `${spawnerPubkey}/${slug}`;
}

function notify(): void {
  for (const listener of listeners) listener();
}

function handleStatusEvent(event: RelayEvent): void {
  const slug = specSlugFromEvent(event);
  if (!slug) return;

  const key = spawnerStatusKey(event.pubkey, slug);
  const previous = latestCreatedAt.get(key);
  if (previous !== undefined && event.created_at <= previous) return;

  // An emptied replacement is the spawner's tombstone for a deleted agent.
  // Without honouring it the last real status — often `pending_attestation` —
  // persists forever and the UI shows a row for an agent that no longer
  // exists, offering buttons that act on nothing.
  const isTombstone = event.content.trim().length === 0;
  const status = isTombstone ? null : parseSpawnerStatus(event.content);
  // Unparseable content is not a tombstone: dropping a live agent because one
  // malformed event arrived would be worse than ignoring the event.
  if (!isTombstone && !status) return;

  latestCreatedAt.set(key, event.created_at);

  const next = new Map(statuses);
  if (status) {
    next.set(key, status);
  } else if (!next.delete(key)) {
    // Nothing was showing for this agent, so the tombstone changes nothing.
    return;
  }
  statuses = next;
  notify();

  // A `prompt_hash` echoed back on status is the spawner confirming it
  // applied the last prompt sent for this agent — clear the pending entry.
  if (status) ackSpawnerPromptUpdate(event.pubkey, slug, status.promptHash);
}

/** Open the status subscription. Idempotent. */
export async function ensureSpawnerStatusSubscription(): Promise<void> {
  if (unsubscribeRelay) return;
  if (startPromise) return startPromise;

  startPromise = (async () => {
    const identity = await getIdentity();
    if (!identity?.pubkey) return;
    const dispose = await subscribeToSpawnerStatus(
      identity.pubkey,
      handleStatusEvent,
    );
    // A reset during connect must not strand a subscription for the old identity.
    if (startPromise === null) {
      void dispose();
      return;
    }
    unsubscribeRelay = dispose;
  })();

  try {
    await startPromise;
  } finally {
    if (unsubscribeRelay) startPromise = null;
  }
}

/** Tear down the subscription and drop cached status. */
export function resetSpawnerStatusStore(): void {
  const dispose = unsubscribeRelay;
  unsubscribeRelay = null;
  startPromise = null;
  if (dispose) void dispose();

  latestCreatedAt.clear();
  if (statuses.size > 0) {
    statuses = new Map();
    notify();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ReadonlyMap<string, SpawnerAgentStatus> {
  return statuses;
}

function getServerSnapshot(): ReadonlyMap<string, SpawnerAgentStatus> {
  return EMPTY;
}

/** Reactive view of every known server-agent status. */
export function useSpawnerStatuses(): ReadonlyMap<string, SpawnerAgentStatus> {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

/** Reactive status for one server agent, or undefined if none has arrived. */
export function useSpawnerStatus(
  spawnerPubkey: string | undefined,
  slug: string | undefined,
): SpawnerAgentStatus | undefined {
  const statuses = useSpawnerStatuses();
  if (!spawnerPubkey || !slug) return undefined;
  return statuses.get(spawnerStatusKey(spawnerPubkey, slug));
}
