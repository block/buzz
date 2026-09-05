import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import type { Channel } from "@/shared/api/types";
import {
  clearOwnOutbox,
  markLegacyConsumed,
  reclaimOutbox,
  resumeWholeBlobOutbox,
  writeOwnOutbox,
} from "./sidebarSyncWatermark";

const STORAGE_KEY_PREFIX = "buzz-channel-sort.v1";
export const MAX_CHANNEL_SORT_GROUPS = 104;

export type ChannelSortMode = "alpha" | "recent";

/**
 * Key identifying a sidebar grouping that carries its own sort preference.
 * Fixed groups use their name; custom sections use `section:<sectionId>`.
 */
export type ChannelSortGroupKey =
  | "starred"
  | "channels"
  | "forums"
  | "dms"
  | `section:${string}`;

export type ChannelSortStore = {
  version: 1;
  groups: Record<string, ChannelSortMode>;
};

export const DEFAULT_SORT_MODE: ChannelSortMode = "alpha";

export const DEFAULT_STORE: ChannelSortStore = Object.freeze({
  version: 1,
  groups: {},
});

export function sectionSortGroupKey(sectionId: string): ChannelSortGroupKey {
  return `section:${sectionId}`;
}

/**
 * Returns the localStorage key for the sidebar channel sort preferences.
 *
 * When `relayUrl` is provided the key is scoped to that relay (normalized via
 * the same `normalizeRelayUrl` used by all relay-scoped local stores) so
 * preferences don't bleed across communities/relays.
 */
export function storageKey(pubkey: string, relayUrl?: string): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  const normalized = normalizeRelayUrl(relayUrl);
  // Encode the normalized relay so it can't contain the `:` delimiter.
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalized)}`;
}

/**
 * Drops per-section sort modes whose custom section no longer exists so
 * deleted sections don't leave stale `section:<id>` keys in localStorage
 * forever. Fixed group keys (starred/channels/forums/dms) are always kept.
 * Returns the same store reference when nothing needs stripping.
 */
export function stripOrphanedSectionModes(
  store: ChannelSortStore,
  liveSectionIds: Iterable<string>,
): ChannelSortStore {
  const liveKeys = new Set<string>(
    [...liveSectionIds].map((id) => sectionSortGroupKey(id)),
  );
  const kept = Object.entries(store.groups).filter(
    ([key]) => !key.startsWith("section:") || liveKeys.has(key),
  );
  if (kept.length === Object.keys(store.groups).length) return store;
  return { ...store, groups: Object.fromEntries(kept) };
}

export function boundChannelSortStore(
  store: ChannelSortStore,
): ChannelSortStore {
  const entries = Object.entries(store.groups);
  if (entries.length <= MAX_CHANNEL_SORT_GROUPS) return store;
  const isFixedGroup = (key: string) =>
    key === "starred" ||
    key === "channels" ||
    key === "forums" ||
    key === "dms";
  const fixed = entries.filter(([key]) => isFixedGroup(key));
  const custom = entries
    .filter(([key]) => !isFixedGroup(key))
    .slice(-(MAX_CHANNEL_SORT_GROUPS - fixed.length));
  return { ...store, groups: Object.fromEntries([...fixed, ...custom]) };
}

export function parseChannelSortPayload(
  json: unknown,
): ChannelSortStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const groups: Record<string, ChannelSortMode> =
    typeof obj.groups === "object" &&
    obj.groups !== null &&
    !Array.isArray(obj.groups)
      ? Object.fromEntries(
          Object.entries(obj.groups as Record<string, unknown>).filter(
            (entry): entry is [string, ChannelSortMode] =>
              entry[1] === "alpha" || entry[1] === "recent",
          ),
        )
      : {};
  return boundChannelSortStore({ version: 1, groups });
}

export function readChannelSortStore(
  pubkey: string,
  relayUrl?: string,
): ChannelSortStore {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey, relayUrl));
    if (!raw) return DEFAULT_STORE;
    return parseChannelSortPayload(JSON.parse(raw)) ?? DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function writeChannelSortStore(
  pubkey: string,
  store: ChannelSortStore,
  relayUrl?: string,
): boolean {
  try {
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify(boundChannelSortStore(store)),
    );
    return true;
  } catch {
    return false;
  }
}

const OUTBOX_KEY_PREFIX = "buzz-channel-sort-outbox.v1";

// The single shared key written by builds before the outbox was keyed
// per-window. Enumerated as one more record so an edit persisted by a prior
// build still resumes, and reclaimed by the same relay-gated rule.
function legacyOutboxKey(pubkey: string, relayUrl: string): string {
  return `${OUTBOX_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

/**
 * Persist this window's unpublished sort edit under its own outbox key. Written
 * synchronously on every edit as a single unconditional `setItem` (no shared-
 * key read-modify-write); resumed on next mount so an edit made <2s before
 * quit/community-switch is never dropped. `queuedAt` stamps the write so resume
 * replays only the newest queued blob (whole-blob LWW).
 */
export function writeChannelSortOutbox(
  pubkey: string,
  store: ChannelSortStore,
  relayUrl: string,
  nowSecs?: number,
): boolean {
  return writeOwnOutbox(
    OUTBOX_KEY_PREFIX,
    pubkey,
    relayUrl,
    boundChannelSortStore(store),
    nowSecs,
  );
}

/**
 * The whole-blob outbox record to resume on boot, or null when none exists.
 * Whole-blob LWW: only the max-`queuedAt` record is replayed. Returns the
 * winning store plus, when that winner is a not-yet-consumed legacy blob, the
 * raw string the caller marks consumed (via `markChannelSortLegacyConsumed`)
 * once it has durably re-queued the intent — the legacy key is never deleted,
 * so this one-shot marker is what stops it republishing above the head forever.
 */
export function readChannelSortOutbox(
  pubkey: string,
  relayUrl: string,
): {
  store: ChannelSortStore;
  legacyRawToConsume: string | null;
  queuedAt: number;
} | null {
  return resumeWholeBlobOutbox(
    OUTBOX_KEY_PREFIX,
    legacyOutboxKey(pubkey, relayUrl),
    pubkey,
    relayUrl,
    parseChannelSortPayload,
  );
}

/**
 * Mark a replayed legacy sort blob consumed so it is not resumed again on a
 * later boot. Call only AFTER the intent is durably held in this window's own
 * v2 key (its synchronous publish path), so a crash before this write replays
 * the legacy blob once more rather than losing it.
 */
export function markChannelSortLegacyConsumed(
  pubkey: string,
  relayUrl: string,
  raw: string,
): void {
  markLegacyConsumed(OUTBOX_KEY_PREFIX, pubkey, relayUrl, raw);
}

/** Clear this window's own outbox key (its edit published or is a no-op). */
export function clearChannelSortOutbox(pubkey: string, relayUrl: string): void {
  clearOwnOutbox(OUTBOX_KEY_PREFIX, pubkey, relayUrl);
}

/**
 * Reclaim foreign outbox keys the relay head itself STRICTLY supersedes: a
 * whole-blob record queued strictly before the durable head's `created_at`
 * (`queuedAt` < `headCreatedAt`) lost LWW to a blob the relay already holds, so
 * dropping it matches the relay's own resolution. A same-second record
 * (`queuedAt` == `headCreatedAt`) is kept — one-second clock granularity cannot
 * prove it lost, so it drains only when a strictly-newer head lands. A record
 * queued after the head is live intent and is likewise kept. Records are
 * write-once so the delete needs no recheck; never touches this window's own
 * keys or the legacy shared key. Call only after a successful fetch.
 */
export function reclaimSupersededSortOutbox(
  pubkey: string,
  relayUrl: string,
  headCreatedAt: number,
): void {
  reclaimOutbox(
    OUTBOX_KEY_PREFIX,
    legacyOutboxKey(pubkey, relayUrl),
    pubkey,
    relayUrl,
    parseChannelSortPayload,
    (record) => record.queuedAt < headCreatedAt,
  );
}

export function sortModeForGroup(
  store: ChannelSortStore,
  group: ChannelSortGroupKey,
): ChannelSortMode {
  return store.groups[group] ?? DEFAULT_SORT_MODE;
}

function channelRecencyMs(channel: Channel): number | null {
  if (!channel.lastMessageAt) return null;
  const ms = Date.parse(channel.lastMessageAt);
  return Number.isFinite(ms) ? ms : null;
}

function compareCodeUnits(left: string, right: string): number {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

export function compareChannelsByName(left: Channel, right: Channel): number {
  return (
    compareCodeUnits(left.name.toLowerCase(), right.name.toLowerCase()) ||
    compareCodeUnits(left.id, right.id)
  );
}

/**
 * Sorts a single sidebar grouping's channels by the selected mode.
 *
 * `alpha` orders by name (id tie-breaker). `recent` orders by last message
 * time, newest first; channels without any message activity sink to the
 * bottom in alphabetical order so quiet channels stay stable and findable.
 */
export function sortChannelsForSidebar(
  channels: Channel[],
  mode: ChannelSortMode,
): Channel[] {
  if (mode === "alpha") {
    return [...channels].sort(compareChannelsByName);
  }
  return [...channels].sort((left, right) => {
    const leftMs = channelRecencyMs(left);
    const rightMs = channelRecencyMs(right);
    if (leftMs !== null && rightMs !== null && leftMs !== rightMs) {
      return rightMs - leftMs;
    }
    if (leftMs !== null && rightMs === null) return -1;
    if (leftMs === null && rightMs !== null) return 1;
    return compareChannelsByName(left, right);
  });
}
