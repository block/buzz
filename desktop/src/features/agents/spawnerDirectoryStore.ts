import React from "react";

import {
  parseSpawnerAnnouncement,
  subscribeToSpawnerAnnouncements,
  type SpawnerAnnouncement,
} from "@/shared/api/spawnerRelay";
import type { RelayEvent } from "@/shared/api/types";
import { retryPendingSpawnerPromptUpdates } from "@/features/agents/spawnerPromptUpdateQueue";

/**
 * Spawners that have announced themselves in this community, keyed by pubkey.
 *
 * # This is a phone book, not an allowlist
 *
 * Anyone can publish a kind:10180. Appearing here means only "some pubkey
 * claims to be a spawner" — every field is self-reported and unverified.
 * Connecting is still an explicit user action, and running an agent still
 * requires the owner to sign a per-agent NIP-OA attestation. Nothing in this
 * store may be treated as a security property.
 */

const listeners = new Set<() => void>();

let announcements: ReadonlyMap<string, SpawnerAnnouncement> = new Map();
let unsubscribeRelay: (() => Promise<void>) | null = null;
let startPromise: Promise<void> | null = null;

/**
 * Newest `created_at` per pubkey.
 *
 * Kind 10180 is replaceable and the spawner republishes on every reconcile, so
 * a reconnect can redeliver an older revision after a newer one. Without this
 * guard a stale capacity count could overwrite the current one.
 */
const latestCreatedAt = new Map<string, number>();

const EMPTY: ReadonlyMap<string, SpawnerAnnouncement> = new Map();

function notify(): void {
  for (const listener of listeners) listener();
}

function handleAnnouncementEvent(event: RelayEvent): void {
  const announcement = parseSpawnerAnnouncement(event);
  if (!announcement) return;

  const previous = latestCreatedAt.get(event.pubkey);
  if (previous !== undefined && event.created_at <= previous) return;
  latestCreatedAt.set(event.pubkey, event.created_at);

  const next = new Map(announcements);
  next.set(event.pubkey, announcement);
  announcements = next;
  notify();

  // An announcement means this spawner is alive right now — resend anything
  // that queued while it may have been unreachable.
  void retryPendingSpawnerPromptUpdates();
}

/** Open the announcement subscription. Idempotent. */
export async function ensureSpawnerDirectorySubscription(): Promise<void> {
  if (unsubscribeRelay) return;
  if (startPromise) return startPromise;

  startPromise = (async () => {
    const dispose = await subscribeToSpawnerAnnouncements(
      handleAnnouncementEvent,
    );
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

/** Tear down the subscription and drop the directory. */
export function resetSpawnerDirectoryStore(): void {
  const dispose = unsubscribeRelay;
  unsubscribeRelay = null;
  startPromise = null;
  if (dispose) void dispose();

  latestCreatedAt.clear();
  if (announcements.size > 0) {
    announcements = new Map();
    notify();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ReadonlyMap<string, SpawnerAnnouncement> {
  return announcements;
}

function getServerSnapshot(): ReadonlyMap<string, SpawnerAnnouncement> {
  return EMPTY;
}

/** Reactive view of every announced spawner, keyed by pubkey. */
export function useSpawnerDirectory(): ReadonlyMap<
  string,
  SpawnerAnnouncement
> {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
