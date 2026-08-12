/**
 * Per-relay cache of the last successfully fetched channel list.
 *
 * Each community mounts a fresh React-Query client, so switching communities
 * (or switching back to one just visited) starts cold and blocks the sidebar
 * on a multi-round-trip `get_channels()`. This module persists the last-known
 * channel list per relay so the sidebar can paint instantly from the snapshot
 * while the live fetch revalidates in the background.
 *
 * Keyed per relay URL (not community id) so equivalent URL formatting maps to
 * one slot and one relay's list never bleeds into another.
 */

import type { Channel } from "@/shared/api/types";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const STORAGE_KEY_PREFIX = "buzz-channels.v1";

export type ChannelSnapshot = {
  channels: Channel[];
  hash: string;
};

export type ChannelSnapshotDiagnostics = {
  ageMs: number;
  channelCount: number;
  presence: "absent" | "invalid" | "present";
  serializedBytes: number;
};

export type ChannelSnapshotReadResult = {
  diagnostics: ChannelSnapshotDiagnostics;
  snapshot: ChannelSnapshot | null;
};

type StoredChannelSnapshot = ChannelSnapshot & {
  version: 2;
  updatedAt: number;
  ownerPubkey: string;
  integrity: string;
};

export function channelSnapshotKey(relayUrl: string): string {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}`;
}

function snapshotIntegrity(
  ownerPubkey: string,
  hash: string,
  channels: Channel[],
): string {
  const value = JSON.stringify([ownerPubkey.toLowerCase(), hash, channels]);
  let result = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 0x01000193);
  }
  return (result >>> 0).toString(16).padStart(8, "0");
}

function parseChannelSnapshot(
  json: unknown,
  ownerPubkey: string,
): ChannelSnapshot | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;

  // Version 1 did not record either the list hash or the owning identity. It
  // cannot safely seed version 2: painting it could expose another identity's
  // channels, and pairing it with any hash could suppress the corrective read.
  if (
    obj.version !== 2 ||
    !Array.isArray(obj.channels) ||
    typeof obj.hash !== "string" ||
    obj.hash.length === 0 ||
    typeof obj.updatedAt !== "number" ||
    !Number.isFinite(obj.updatedAt) ||
    typeof obj.ownerPubkey !== "string" ||
    obj.ownerPubkey.toLowerCase() !== ownerPubkey.toLowerCase()
  ) {
    return null;
  }

  const channels = obj.channels as Channel[];
  if (
    typeof obj.integrity !== "string" ||
    obj.integrity !== snapshotIntegrity(ownerPubkey, obj.hash, channels)
  ) {
    return null;
  }

  return { channels, hash: obj.hash };
}

function serializedBytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

/**
 * Reads the atomic snapshot and reports why it can or cannot seed the sidebar.
 * Invalid includes malformed, legacy, hashless, partial, and wrong-owner data.
 */
export function inspectChannelSnapshot(
  relayUrl: string,
  ownerPubkey: string,
): ChannelSnapshotReadResult {
  const absent: ChannelSnapshotDiagnostics = {
    ageMs: 0,
    channelCount: 0,
    presence: "absent",
    serializedBytes: 0,
  };

  try {
    const raw = window.localStorage.getItem(channelSnapshotKey(relayUrl));
    if (!raw) return { diagnostics: absent, snapshot: null };

    const parsedJson = JSON.parse(raw) as Record<string, unknown>;
    const snapshot = parseChannelSnapshot(parsedJson, ownerPubkey);
    const updatedAt =
      typeof parsedJson.updatedAt === "number" &&
      Number.isFinite(parsedJson.updatedAt)
        ? parsedJson.updatedAt
        : null;
    const diagnostics: ChannelSnapshotDiagnostics = {
      ageMs: updatedAt === null ? 0 : Math.max(0, Date.now() - updatedAt),
      channelCount: snapshot?.channels.length ?? 0,
      presence: snapshot ? "present" : "invalid",
      serializedBytes: serializedBytes(raw),
    };
    return { diagnostics, snapshot };
  } catch {
    const raw = window.localStorage.getItem(channelSnapshotKey(relayUrl));
    return {
      diagnostics: {
        ...absent,
        presence: raw === null ? "absent" : "invalid",
        serializedBytes: raw === null ? 0 : serializedBytes(raw),
      },
      snapshot: null,
    };
  }
}

/**
 * Reads the atomic channel-list/hash snapshot for an identity and relay, or
 * null when absent, malformed, legacy, or owned by another identity.
 */
export function readChannelSnapshot(
  relayUrl: string,
  ownerPubkey: string,
): ChannelSnapshot | null {
  return inspectChannelSnapshot(relayUrl, ownerPubkey).snapshot;
}

/**
 * Persists the complete last successfully fetched channel list and the hash
 * that describes it as one document. Atomic replacement prevents a stale hash
 * from ever being paired with a newer list (or vice versa). Non-fatal on
 * storage failure (e.g. quota exceeded).
 */
export function writeChannelSnapshot(
  relayUrl: string,
  ownerPubkey: string,
  channels: Channel[],
  hash: string,
): void {
  try {
    if (!ownerPubkey || !hash) return;

    const key = channelSnapshotKey(relayUrl);
    const previous = window.localStorage.getItem(key);
    if (previous) {
      try {
        const parsed = parseChannelSnapshot(JSON.parse(previous), ownerPubkey);
        if (
          parsed &&
          parsed.hash === hash &&
          JSON.stringify(parsed.channels) === JSON.stringify(channels)
        ) {
          return;
        }
      } catch {
        // Malformed snapshots are replaced below.
      }
    }

    const snapshot: StoredChannelSnapshot = {
      version: 2,
      updatedAt: Date.now(),
      ownerPubkey: ownerPubkey.toLowerCase(),
      hash,
      channels,
      integrity: snapshotIntegrity(ownerPubkey, hash, channels),
    };
    setLocalStorageItemWithRecovery(key, JSON.stringify(snapshot));
  } catch {
    // Storage access failures are non-fatal.
  }
}

/**
 * Removes the channel snapshot for a relay. Called when a community is removed.
 */
export function removeChannelSnapshotForRelay(relayUrl: string): void {
  try {
    window.localStorage.removeItem(channelSnapshotKey(relayUrl));
  } catch {
    // Storage access failures are non-fatal.
  }
}
