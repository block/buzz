/**
 * Per-channel persisted message snapshots.
 *
 * A channel revisited after its React-Query cache entry is gone (app restart,
 * gcTime expiry, community remount) goes fully cold and holds a skeleton for a
 * relay round trip. This module persists the newest slice of each channel's
 * timeline so a revisit can paint instantly from the snapshot while the
 * history fetch revalidates behind it — the same stale-then-revalidate pattern
 * the sidebar's channelSnapshot uses for the channel list.
 *
 * Keyed per relay URL + channel id so one relay's messages never bleed into
 * another. Bounded two ways: only the newest MAX_EVENTS_PER_SNAPSHOT events
 * per channel, and only the MAX_CHANNELS_PER_RELAY most recently written
 * channels per relay (older ones are evicted LRU on write).
 */

import { mergeTimelineHistoryMessages } from "@/features/messages/lib/messageQueryKeys";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import type { RelayEvent } from "@/shared/api/types";

const STORAGE_KEY_PREFIX = "buzz-channel-messages.v1";

// Newest events kept per channel. The trailing slice of the sorted timeline
// cache, so recent auxiliary events (reactions/edits) ride along with the
// content rows they decorate.
const MAX_EVENTS_PER_SNAPSHOT = 80;

const MAX_CHANNELS_PER_RELAY = 20;

export function messageSnapshotKey(relayUrl: string, channelId: string) {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}:${channelId}`;
}

type SnapshotPayload = {
  version: 1;
  updatedAt: number;
  events: RelayEvent[];
};

function parseSnapshotPayload(json: unknown): SnapshotPayload | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1 || !Array.isArray(obj.events)) return null;
  const updatedAt =
    typeof obj.updatedAt === "number" && Number.isFinite(obj.updatedAt)
      ? obj.updatedAt
      : 0;
  return { version: 1, updatedAt, events: obj.events as RelayEvent[] };
}

/**
 * Reads the persisted message snapshot for a channel, or null when absent or
 * malformed.
 */
export function readMessageSnapshot(
  relayUrl: string,
  channelId: string,
): RelayEvent[] | null {
  try {
    const raw = window.localStorage.getItem(
      messageSnapshotKey(relayUrl, channelId),
    );
    if (!raw) return null;
    const parsed = parseSnapshotPayload(JSON.parse(raw));
    if (!parsed || parsed.events.length === 0) return null;
    return parsed.events;
  } catch {
    return null;
  }
}

function relayPrefix(relayUrl: string) {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}:`;
}

function collectKeysWithPrefix(prefix: string): string[] {
  const keys: string[] = [];
  for (let i = 0; i < window.localStorage.length; i++) {
    const key = window.localStorage.key(i);
    if (key?.startsWith(prefix)) {
      keys.push(key);
    }
  }
  return keys;
}

function evictOldestSnapshots(prefix: string, keepingKey: string) {
  const others = collectKeysWithPrefix(prefix).filter(
    (key) => key !== keepingKey,
  );
  if (others.length < MAX_CHANNELS_PER_RELAY) {
    return;
  }

  const byAge = others
    .map((key) => {
      let updatedAt = 0;
      try {
        const parsed = parseSnapshotPayload(
          JSON.parse(window.localStorage.getItem(key) ?? ""),
        );
        updatedAt = parsed?.updatedAt ?? 0;
      } catch {
        // Malformed entries sort oldest and get evicted first.
      }
      return { key, updatedAt };
    })
    .sort((a, b) => a.updatedAt - b.updatedAt);

  for (const { key } of byAge.slice(
    0,
    others.length - (MAX_CHANNELS_PER_RELAY - 1),
  )) {
    window.localStorage.removeItem(key);
  }
}

/**
 * Persists the newest slice of a channel's timeline. Pending optimistic events
 * are dropped (they have no relay identity to revalidate against). Skips the
 * write when unchanged so live-append churn does not re-serialize an identical
 * snapshot. Non-fatal on storage failure (e.g. quota exceeded).
 */
export function writeMessageSnapshot(
  relayUrl: string,
  channelId: string,
  events: RelayEvent[],
): void {
  try {
    const persistable = events
      .filter((event) => !event.pending)
      .slice(-MAX_EVENTS_PER_SNAPSHOT);
    if (persistable.length === 0) {
      return;
    }

    const key = messageSnapshotKey(relayUrl, channelId);
    const previous = window.localStorage.getItem(key);
    if (previous) {
      const parsed = parseSnapshotPayload(JSON.parse(previous));
      if (
        parsed &&
        JSON.stringify(parsed.events) === JSON.stringify(persistable)
      ) {
        return;
      }
    }

    evictOldestSnapshots(relayPrefix(relayUrl), key);
    window.localStorage.setItem(
      key,
      JSON.stringify({
        version: 1,
        updatedAt: Date.now(),
        events: persistable,
      } satisfies SnapshotPayload),
    );
  } catch {
    // Storage access failures are non-fatal.
  }
}

/**
 * Merge a fresh history window over the in-memory cache — or, when cold, over
 * the persisted snapshot — and pick the window aux backfill must cover.
 *
 * The snapshot can hold events older than the fetch window; dropping them on
 * settle would visibly shrink an already-painted timeline, so the merge keeps
 * them. But a kept snapshot row deleted/edited while the app was closed never
 * reappears in any history fetch (the relay soft-deletes), so its tombstone or
 * edit is only reachable by `#e` over that row's id — cold snapshot loads must
 * therefore backfill over the merged timeline, not just the fresh window.
 * Otherwise the ghost paints, and the post-settle snapshot rewrite persists it
 * forever.
 */
export function mergeHistoryOverSnapshot(input: {
  cached: RelayEvent[] | undefined;
  snapshot: RelayEvent[] | null;
  history: RelayEvent[];
}): { merged: RelayEvent[]; auxBackfillWindow: RelayEvent[] } {
  const usedSnapshot = !input.cached && input.snapshot !== null;
  const merged = mergeTimelineHistoryMessages(
    input.cached ?? input.snapshot ?? [],
    input.history,
  );
  return { merged, auxBackfillWindow: usedSnapshot ? merged : input.history };
}

/**
 * Removes every channel message snapshot for a relay. Called when a community
 * is removed.
 */
export function removeMessageSnapshotsForRelay(relayUrl: string): void {
  try {
    for (const key of collectKeysWithPrefix(relayPrefix(relayUrl))) {
      window.localStorage.removeItem(key);
    }
  } catch {
    // Storage access failures are non-fatal.
  }
}
