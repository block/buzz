/**
 * Per-relay cache of the last successfully fetched channel list.
 *
 * Each community mounts a fresh React-Query client, so switching communities
 * (or switching back to one just visited) starts cold and blocks the sidebar
 * on a multi-round-trip `get_channels()`. This module persists the last-known
 * channel list per relay so the sidebar can paint instantly from the snapshot
 * while the live fetch revalidates in the background.
 *
 * Keyed per normalized relay URL and authoritative identity so identities that
 * share a relay retain independent snapshots without exposing either list to
 * the other.
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

function channelSnapshotRelayPrefix(relayUrl: string): string {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}:`;
}

export function channelSnapshotKey(
  relayUrl: string,
  ownerPubkey: string,
): string {
  return `${channelSnapshotRelayPrefix(relayUrl)}${ownerPubkey.toLowerCase()}`;
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

type ParsedChannelSnapshotCacheEntry = {
  presence: "invalid" | "present";
  raw: string;
  serializedBytes: number;
  snapshot: ChannelSnapshot | null;
  updatedAt: number | null;
};

const parsedSnapshotCache = new Map<string, ParsedChannelSnapshotCacheEntry>();

function snapshotResultFromRaw(
  key: string,
  raw: string,
  ownerPubkey: string,
): ChannelSnapshotReadResult {
  let cached = parsedSnapshotCache.get(key);
  if (!cached || cached.raw !== raw) {
    let parsedJson: Record<string, unknown> | null = null;
    try {
      parsedJson = JSON.parse(raw) as Record<string, unknown>;
    } catch {
      // Malformed snapshots are invalid cache entries, never fatal reads.
    }
    const snapshot = parsedJson
      ? parseChannelSnapshot(parsedJson, ownerPubkey)
      : null;
    const updatedAt =
      parsedJson &&
      typeof parsedJson.updatedAt === "number" &&
      Number.isFinite(parsedJson.updatedAt)
        ? parsedJson.updatedAt
        : null;
    cached = {
      presence: snapshot ? "present" : "invalid",
      raw,
      serializedBytes: serializedBytes(raw),
      snapshot,
      updatedAt,
    };
    parsedSnapshotCache.set(key, cached);
  }

  return {
    diagnostics: {
      ageMs:
        cached.updatedAt === null
          ? 0
          : Math.max(0, Date.now() - cached.updatedAt),
      channelCount: cached.snapshot?.channels.length ?? 0,
      presence: cached.presence,
      serializedBytes: cached.serializedBytes,
    },
    snapshot: cached.snapshot,
  };
}

/**
 * Reads the atomic snapshot and reports why it can or cannot seed the sidebar.
 * Invalid includes malformed, legacy, hashless, partial, and wrong-owner data.
 * Parsing and integrity validation are memoized by storage key and raw value so
 * repeated hook consumers do not rescan the same multi-megabyte document.
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
  const key = channelSnapshotKey(relayUrl, ownerPubkey);
  let raw: string | null = null;

  try {
    raw = window.localStorage.getItem(key);
    if (!raw) return { diagnostics: absent, snapshot: null };
    return snapshotResultFromRaw(key, raw, ownerPubkey);
  } catch {
    // `getItem` itself can throw when WebKit denies storage access. Never retry
    // that read from the catch path: snapshot hydration is optional and must
    // degrade to a live fetch rather than escaping into the render tree.
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

    const key = channelSnapshotKey(relayUrl, ownerPubkey);
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
    const prefix = channelSnapshotRelayPrefix(relayUrl);
    const keys: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (key?.startsWith(prefix)) keys.push(key);
    }
    for (const key of keys) window.localStorage.removeItem(key);
    // Also clear the superseded relay-only v1/v2 slot.
    window.localStorage.removeItem(prefix.slice(0, -1));
  } catch {
    // Storage access failures are non-fatal.
  }
}
