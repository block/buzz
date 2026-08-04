/**
 * Disposable projection cache for the sidebar's observed-unread candidate set.
 *
 * The sidebar computes unread channel badges from two in-memory refs:
 *   - observedUnreadEventsByChannelRef  (per-event metadata)
 *   - latestByChannelRef               (max createdAt per channel, derived)
 *
 * Both refs are wiped on webview reload. The boot catch-up REQ can only fetch
 * events *newer than* each channel's NIP-RS frontier, so thread replies that
 * arrived before the frontier was advanced (the common case) are never
 * re-discovered. This module persists the per-event map so reload restores
 * candidate evidence that the relay query structurally cannot recover.
 *
 * Design constraints:
 *  - Storage key: "buzz-observed-unread.v1:<normalizedRelayUrl>:<normalizedPubkey>"
 *    (relay-scoped to prevent cross-community leakage, matching threadActivityStorage).
 *  - Registered in PURE_CACHE_KEY_PREFIXES so the 2 MiB LRU eviction budget applies.
 *  - Payload includes `updatedAt` (ms timestamp) for LRU ordering across scopes.
 *  - Field-level validation: malformed or non-finite values are discarded silently.
 *  - Age pruning: events older than READ_STATE_HORIZON_SECONDS (7 days) are dropped.
 *  - Per-channel cap: newest CATCH_UP_LIMIT events per channel.
 *  - Global cap: newest GLOBAL_OBSERVED_UNREAD_CAP events across all channels.
 *  - Write failure is non-fatal; callers degrade to session-only behavior.
 *  - NOT a source of truth: NIP-RS markers remain authoritative for read state.
 *    The projection memo re-evaluates every retained event against current markers.
 */

import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { READ_STATE_HORIZON_SECONDS } from "@/features/channels/readState/readStateFormat";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";
import {
  observedUnreadEventReadAt,
  type ObservedUnreadEvent,
} from "@/features/channels/unreadChannelCounts";

export const OBSERVED_UNREAD_STORAGE_PREFIX = "buzz-observed-unread.v1";

// Per-channel cap: matches CATCH_UP_LIMIT in useUnreadChannels.ts.
const PER_CHANNEL_CAP = 1000;
// Global cap across all channels in a scope bucket.
const GLOBAL_OBSERVED_UNREAD_CAP = 5000;

export type ObservedUnreadScope = {
  pubkey: string;
  relayUrl: string;
};

export function observedUnreadStorageKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${OBSERVED_UNREAD_STORAGE_PREFIX}:${normalizeRelayUrl(relayUrl)}:${pubkey.toLowerCase()}`;
}

type PersistedPayload = {
  updatedAt: number;
  eventsByChannel: Record<string, ObservedUnreadEvent[]>;
};

function isValidEvent(v: unknown): v is ObservedUnreadEvent {
  if (typeof v !== "object" || v === null) return false;
  const e = v as Record<string, unknown>;
  return (
    typeof e.id === "string" &&
    typeof e.createdAt === "number" &&
    Number.isFinite(e.createdAt) &&
    (e.rootId === null || typeof e.rootId === "string") &&
    typeof e.highPriority === "boolean" &&
    typeof e.countsTowardBadge === "boolean" &&
    typeof e.countsTowardAppBadge === "boolean"
  );
}

function nowUnixSeconds(): number {
  return Math.floor(Date.now() / 1_000);
}

/**
 * Prune a per-channel event map in place:
 *   1. Drop events older than READ_STATE_HORIZON_SECONDS.
 *   2. Per-channel cap: keep newest PER_CHANNEL_CAP events.
 *   3. Global cap: if total exceeds GLOBAL_OBSERVED_UNREAD_CAP, keep the
 *      newest events across all channels.
 *
 * Returns the pruned map (same reference, mutated).
 */
function pruneEventsByChannel(
  eventsByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
  nowSeconds: number,
): Map<string, Map<string, ObservedUnreadEvent>> {
  const cutoff = nowSeconds - READ_STATE_HORIZON_SECONDS;

  // Age prune and per-channel cap.
  for (const [channelId, eventsById] of eventsByChannel) {
    for (const [id, event] of eventsById) {
      if (event.createdAt <= cutoff) {
        eventsById.delete(id);
      }
    }
    if (eventsById.size === 0) {
      eventsByChannel.delete(channelId);
      continue;
    }
    if (eventsById.size > PER_CHANNEL_CAP) {
      const sorted = [...eventsById.values()].sort(
        (a, b) => a.createdAt - b.createdAt,
      );
      const toRemove = sorted.slice(0, eventsById.size - PER_CHANNEL_CAP);
      for (const e of toRemove) {
        eventsById.delete(e.id);
      }
    }
  }

  // Global cap: if total events exceed limit, evict oldest across channels.
  let total = 0;
  for (const eventsById of eventsByChannel.values()) {
    total += eventsById.size;
  }
  if (total > GLOBAL_OBSERVED_UNREAD_CAP) {
    // Collect all events with their channel id, sort by createdAt ascending.
    const all: Array<{ channelId: string; event: ObservedUnreadEvent }> = [];
    for (const [channelId, eventsById] of eventsByChannel) {
      for (const event of eventsById.values()) {
        all.push({ channelId, event });
      }
    }
    all.sort((a, b) => a.event.createdAt - b.event.createdAt);
    const excess = total - GLOBAL_OBSERVED_UNREAD_CAP;
    for (let i = 0; i < excess; i++) {
      const entry = all[i];
      if (!entry) break;
      const eventsById = eventsByChannel.get(entry.channelId);
      if (eventsById) {
        eventsById.delete(entry.event.id);
        if (eventsById.size === 0) {
          eventsByChannel.delete(entry.channelId);
        }
      }
    }
  }

  return eventsByChannel;
}

/**
 * Read and decode the persisted observed-unread map for a scope.
 * Returns null when no data exists or decoding fails.
 * Applies age pruning and caps before returning.
 */
export function readObservedUnreadFromStorage(
  pubkey: string,
  relayUrl: string,
): Map<string, Map<string, ObservedUnreadEvent>> | null {
  try {
    const key = observedUnreadStorageKey(pubkey, relayUrl);
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return null;
    }

    const payload = parsed as Record<string, unknown>;
    const eventsByChannelRaw = payload.eventsByChannel;
    if (
      typeof eventsByChannelRaw !== "object" ||
      eventsByChannelRaw === null ||
      Array.isArray(eventsByChannelRaw)
    ) {
      return null;
    }

    const result = new Map<string, Map<string, ObservedUnreadEvent>>();
    for (const [channelId, events] of Object.entries(
      eventsByChannelRaw as Record<string, unknown>,
    )) {
      if (!Array.isArray(events)) continue;
      const eventsById = new Map<string, ObservedUnreadEvent>();
      for (const e of events) {
        if (isValidEvent(e)) {
          eventsById.set(e.id, e);
        }
      }
      if (eventsById.size > 0) {
        result.set(channelId, eventsById);
      }
    }

    if (result.size === 0) return null;

    pruneEventsByChannel(result, nowUnixSeconds());
    return result.size > 0 ? result : null;
  } catch {
    return null;
  }
}

/**
 * Serialize and write the observed-unread map for a scope to localStorage.
 * Prunes before writing. Write failure is non-fatal.
 * Returns true when the write succeeded.
 */
export function writeObservedUnreadToStorage(
  pubkey: string,
  relayUrl: string,
  eventsByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
): boolean {
  try {
    // Prune on a shallow copy so we don't mutate the live refs.
    const copy = new Map<string, Map<string, ObservedUnreadEvent>>();
    for (const [channelId, eventsById] of eventsByChannel) {
      copy.set(channelId, new Map(eventsById));
    }
    pruneEventsByChannel(copy, nowUnixSeconds());

    if (copy.size === 0) {
      // Nothing worth persisting — remove any stale entry.
      try {
        window.localStorage.removeItem(
          observedUnreadStorageKey(pubkey, relayUrl),
        );
      } catch {
        // Ignore.
      }
      return true;
    }

    const payload: PersistedPayload = {
      updatedAt: Date.now(),
      eventsByChannel: {},
    };
    for (const [channelId, eventsById] of copy) {
      payload.eventsByChannel[channelId] = [...eventsById.values()];
    }

    const key = observedUnreadStorageKey(pubkey, relayUrl);
    return setLocalStorageItemWithRecovery(key, JSON.stringify(payload));
  } catch {
    return false;
  }
}

/**
 * Delete the entire observed-unread storage bucket for a scope.
 * Used on mark-all-read or when all channels are cleared.
 */
export function clearObservedUnreadStorage(
  pubkey: string,
  relayUrl: string,
): void {
  try {
    window.localStorage.removeItem(observedUnreadStorageKey(pubkey, relayUrl));
  } catch {
    // Ignore.
  }
}

/**
 * Derive the latestByChannel map from the eventsByChannel map.
 * This is used at hydration time to repopulate latestByChannelRef
 * without storing a redundant aggregate.
 */
export function deriveLatestByChannel(
  eventsByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
): Map<string, number> {
  const result = new Map<string, number>();
  for (const [channelId, eventsById] of eventsByChannel) {
    let max = 0;
    for (const event of eventsById.values()) {
      if (event.createdAt > max) max = event.createdAt;
    }
    if (max > 0) result.set(channelId, max);
  }
  return result;
}

// ─── Persistence writer (scope-safe debounce + flush) ─────────────────────────
//
// Factory used by useUnreadChannels. Returns schedule/flush bound to the
// provided refs. Kept here so the hook stays slim.

export type ObservedUnreadRefs = {
  eventsRef: { current: Map<string, Map<string, ObservedUnreadEvent>> };
  scopeLoadedRef: { current: string };
  timerRef: { current: ReturnType<typeof setTimeout> | null };
};

/**
 * Decompose a scope key ("pubkey:normalizedRelayUrl") into its components.
 * Returns null when the scope is empty or malformed.
 */
function decomposeScope(
  scope: string,
): { pubkey: string; relayUrl: string } | null {
  if (!scope) return null;
  const colonIdx = scope.indexOf(":");
  if (colonIdx === -1) return null;
  const pubkey = scope.slice(0, colonIdx);
  const relayUrl = scope.slice(colonIdx + 1);
  return pubkey && relayUrl ? { pubkey, relayUrl } : null;
}

/**
 * Schedule a debounced 1-second write of the observed-unread refs for `scope`.
 * A pending timer for any scope is replaced. The timer closure owns an
 * immutable {scope, snapshot} captured at call time — a late A-scope timer
 * can never read B's mutable refs or write under B's key.
 */
export function scheduleObservedUnreadWrite(
  scope: string,
  refs: ObservedUnreadRefs,
): void {
  if (!scope || refs.scopeLoadedRef.current !== scope) return;
  if (refs.timerRef.current !== null) clearTimeout(refs.timerRef.current);

  const parts = decomposeScope(scope);
  if (!parts) return;
  const { pubkey, relayUrl } = parts;

  // Capture an immutable deep clone of the events map at this moment.
  // The timer callback owns this snapshot and never touches the live refs.
  const snapshot = new Map<string, Map<string, ObservedUnreadEvent>>();
  for (const [channelId, eventsById] of refs.eventsRef.current) {
    snapshot.set(channelId, new Map(eventsById));
  }

  refs.timerRef.current = setTimeout(() => {
    refs.timerRef.current = null;
    writeObservedUnreadToStorage(pubkey, relayUrl, snapshot);
  }, 1_000);
}

/**
 * Synchronously flush any pending debounce write and cancel the timer.
 * Called on pagehide and before scope reset to close the Cmd+R timing gap.
 */
export function flushObservedUnreadWrite(refs: ObservedUnreadRefs): void {
  if (refs.timerRef.current !== null) {
    clearTimeout(refs.timerRef.current);
    refs.timerRef.current = null;
  }
  const scope = refs.scopeLoadedRef.current;
  const parts = decomposeScope(scope);
  if (!parts) return;
  writeObservedUnreadToStorage(
    parts.pubkey,
    parts.relayUrl,
    refs.eventsRef.current,
  );
}

/**
 * Prune persisted observed events that are now covered by read markers.
 * Uses the same observedUnreadEventReadAt() the projection memo uses so the
 * two are always in sync. Rederives per-channel latest after pruning.
 *
 * Returns true when any event was removed (caller should persist + bump).
 */
export function pruneObservedUnreadByMarkers(
  eventsByChannel: Map<string, Map<string, ObservedUnreadEvent>>,
  latestByChannel: Map<string, number>,
  getChannelReadAt: (channelId: string) => number | null,
  getOwnTimestamp: (contextId: string) => number | null,
): boolean {
  let changed = false;
  for (const [channelId, eventsById] of eventsByChannel) {
    const channelReadAt = getChannelReadAt(channelId);
    const toDelete: string[] = [];
    for (const event of eventsById.values()) {
      const readAt = observedUnreadEventReadAt(
        event,
        channelReadAt,
        (rootId) => getOwnTimestamp(`thread:${rootId}`),
        (messageId) => getOwnTimestamp(`msg:${messageId}`),
      );
      if (readAt !== null && event.createdAt <= readAt) toDelete.push(event.id);
    }
    for (const id of toDelete) {
      eventsById.delete(id);
      changed = true;
    }
    if (eventsById.size === 0) {
      eventsByChannel.delete(channelId);
    } else {
      let max = 0;
      for (const e of eventsById.values()) {
        if (e.createdAt > max) max = e.createdAt;
      }
      if (max > 0) latestByChannel.set(channelId, max);
      else latestByChannel.delete(channelId);
    }
  }
  for (const channelId of latestByChannel.keys()) {
    if (!eventsByChannel.has(channelId)) {
      latestByChannel.delete(channelId);
      changed = true;
    }
  }
  return changed;
}
