import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import type { Channel } from "@/shared/api/types";

const STORAGE_KEY_PREFIX = "buzz-channel-sort.v1";

export type ChannelSortMode = "alpha" | "recent";

export type ChannelSortStore = {
  version: 1;
  mode: ChannelSortMode;
};

export const DEFAULT_STORE: ChannelSortStore = Object.freeze({
  version: 1,
  mode: "alpha",
});

/**
 * Returns the localStorage key for the sidebar channel sort preference.
 *
 * When `relayUrl` is provided the key is scoped to that relay (normalized via
 * the same `normalizeRelayUrl` used by all relay-scoped local stores) so the
 * preference doesn't bleed across workspaces/relays.
 */
export function storageKey(pubkey: string, relayUrl?: string): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  const normalized = normalizeRelayUrl(relayUrl);
  // Encode the normalized relay so it can't contain the `:` delimiter.
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalized)}`;
}

export function parseChannelSortPayload(
  json: unknown,
): ChannelSortStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  if (obj.mode !== "alpha" && obj.mode !== "recent") return null;
  return { version: 1, mode: obj.mode };
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
