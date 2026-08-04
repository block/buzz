import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import type { Channel } from "@/shared/api/types";

const STORAGE_KEY_PREFIX = "buzz-channel-sort.v1";

export type ChannelSortMode = "alpha" | "recent" | "manual";
type PersistedChannelSortMode = Exclude<ChannelSortMode, "manual">;

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
  groups: Record<string, PersistedChannelSortMode>;
};

/** Default for non-stream groups (starred / forums / dms). */
export const DEFAULT_SORT_MODE: ChannelSortMode = "alpha";

/**
 * Default for stream groups (Channels + custom categories) when the user has
 * never saved an explicit A–Z / Recent preference. Manual is implicit — it
 * does not write sort prefs or seed the order store until the user reorders
 * or chooses Manual explicitly.
 */
export const DEFAULT_STREAM_SORT_MODE: ChannelSortMode = "manual";

export const DEFAULT_STORE: ChannelSortStore = Object.freeze({
  version: 1,
  groups: {},
});

export function sectionSortGroupKey(sectionId: string): ChannelSortGroupKey {
  return `section:${sectionId}`;
}

/** Stream sidebar groups that default to Manual when unset. */
export function isStreamSortGroup(group: ChannelSortGroupKey): boolean {
  return group === "channels" || group.startsWith("section:");
}

/** True when the user (or a remote peer) has saved A–Z or Recent for the group. */
export function hasExplicitSortPreference(
  store: ChannelSortStore,
  group: ChannelSortGroupKey,
): boolean {
  return Object.hasOwn(store.groups, group);
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

export function parseChannelSortPayload(
  json: unknown,
): ChannelSortStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const groups: Record<string, PersistedChannelSortMode> =
    typeof obj.groups === "object" &&
    obj.groups !== null &&
    !Array.isArray(obj.groups)
      ? Object.fromEntries(
          Object.entries(obj.groups as Record<string, unknown>).filter(
            (entry): entry is [string, PersistedChannelSortMode] =>
              entry[1] === "alpha" || entry[1] === "recent",
          ),
        )
      : {};
  return { version: 1, groups };
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
      JSON.stringify(store),
    );
    return true;
  } catch {
    return false;
  }
}

/**
 * Effective sort mode for a group from the persisted alpha/recent map only.
 * Does not consult the separate manual-order store — callers that track
 * explicit Manual via `manualGroups` should treat that as higher priority.
 *
 * Unset stream groups return Manual; unset non-stream groups return A–Z.
 * Explicit A–Z is distinguishable from unset via {@link hasExplicitSortPreference}.
 */
export function sortModeForGroup(
  store: ChannelSortStore,
  group: ChannelSortGroupKey,
): ChannelSortMode {
  const explicit = store.groups[group];
  if (explicit === "alpha" || explicit === "recent") return explicit;
  if (isStreamSortGroup(group)) return DEFAULT_STREAM_SORT_MODE;
  return DEFAULT_SORT_MODE;
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
  if (mode === "alpha" || mode === "manual") {
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
