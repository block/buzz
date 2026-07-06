import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import type { Channel } from "@/shared/api/types";

const STORAGE_KEY_PREFIX = "buzz-channel-sort.v1";

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
 * preferences don't bleed across workspaces/relays.
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

export function compareChannelsByName(left: Channel, right: Channel): number {
  return left.name.localeCompare(right.name) || left.id.localeCompare(right.id);
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
