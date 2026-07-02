/**
 * Per-relay cache of the last successfully fetched channel list.
 *
 * Each workspace mounts a fresh React-Query client, so switching workspaces
 * (or switching back to one just visited) starts cold and blocks the sidebar
 * on a multi-round-trip `get_channels()`. This module persists the last-known
 * channel list per relay so the sidebar can paint instantly from the snapshot
 * while the live fetch revalidates in the background.
 *
 * Keyed per relay URL (not workspace id) so equivalent URL formatting maps to
 * one slot and one relay's list never bleeds into another.
 */

import type { Channel } from "@/shared/api/types";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

const STORAGE_KEY_PREFIX = "buzz-channels.v1";

export function channelSnapshotKey(relayUrl: string): string {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}`;
}

function parseChannelSnapshot(json: unknown): Channel[] | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1 || !Array.isArray(obj.channels)) return null;
  return obj.channels as Channel[];
}

/**
 * Reads the cached channel list for a relay, or null when absent or malformed.
 */
export function readChannelSnapshot(relayUrl: string): Channel[] | null {
  try {
    const raw = window.localStorage.getItem(channelSnapshotKey(relayUrl));
    if (!raw) return null;
    return parseChannelSnapshot(JSON.parse(raw));
  } catch {
    return null;
  }
}

/**
 * Persists the channel list for a relay. Skips the write when unchanged so the
 * 60s background refetch does not re-serialize an identical list. Non-fatal on
 * storage failure (e.g. quota exceeded).
 */
export function writeChannelSnapshot(
  relayUrl: string,
  channels: Channel[],
): void {
  try {
    const key = channelSnapshotKey(relayUrl);
    const serialized = JSON.stringify({ version: 1, channels });
    if (window.localStorage.getItem(key) === serialized) return;
    window.localStorage.setItem(key, serialized);
  } catch {
    // Storage access failures are non-fatal.
  }
}

/**
 * Removes the channel snapshot for a relay. Called when a workspace is removed.
 */
export function removeChannelSnapshotForRelay(relayUrl: string): void {
  try {
    window.localStorage.removeItem(channelSnapshotKey(relayUrl));
  } catch {
    // Storage access failures are non-fatal.
  }
}
