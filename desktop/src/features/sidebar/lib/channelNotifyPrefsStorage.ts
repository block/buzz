import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

const STORAGE_KEY_PREFIX = "buzz-channel-notify-prefs.v1";

/** Per-channel notification level. Absent on an entry means inherit "all". */
export type ChannelNotifyLevel = "all" | "mentions" | "mute";

/**
 * One channel's notification preferences. Every field except `updatedAt` is
 * optional — an absent field means "use the default" (level "all", no timed
 * mute, desktop on, follow-all-threads off, broadcasts on). The whole entry
 * moves under a single `updatedAt` (per-entry LWW, never per-field).
 *
 * Entries may carry fields this client does not know about (e.g. the `mobile`
 * field reserved by NIP-CN for the mobile follow-up). Those are preserved
 * verbatim on parse and merge so a newer client's data survives our writes.
 */
export type ChannelNotifyEntry = {
  level?: ChannelNotifyLevel;
  /** Absolute Unix seconds; effective level is "mute" while in the future. */
  muteUntil?: number;
  desktop?: boolean;
  followAllThreads?: boolean;
  broadcasts?: boolean;
  updatedAt: number;
};

export type ChannelNotifyPrefsStore = {
  version: 1;
  channels: Record<string, ChannelNotifyEntry>;
};

export const DEFAULT_STORE: ChannelNotifyPrefsStore = Object.freeze({
  version: 1,
  channels: {},
});

const KNOWN_ENTRY_KEYS = new Set([
  "level",
  "muteUntil",
  "desktop",
  "followAllThreads",
  "broadcasts",
  "updatedAt",
]);

/**
 * localStorage key for the notify prefs mirror, scoped to both the relay and
 * the pubkey so preferences from different communities never bleed across each
 * other (the pubkey-only key used by `channelMutesStorage` is a known defect).
 */
export function storageKey(pubkey: string, relayUrl: string): string {
  // Encode the normalized relay so it can't contain the `:` delimiter.
  const normalized = encodeURIComponent(normalizeRelayUrl(relayUrl));
  return `${STORAGE_KEY_PREFIX}:${normalized}:${pubkey}`;
}

function isValidTimestamp(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

/**
 * Coerce one raw entry, dropping malformed known fields but preserving any
 * unknown fields. Returns null when the entry has no usable `updatedAt` (it
 * cannot participate in LWW merges, so it is not worth keeping).
 */
export function parseNotifyEntry(value: unknown): ChannelNotifyEntry | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const {
    level,
    muteUntil,
    desktop,
    followAllThreads,
    broadcasts,
    updatedAt,
    ...unknownFields
  } = value as Record<string, unknown>;
  if (!isValidTimestamp(updatedAt)) return null;
  // Cast: `unknownFields` is intentionally opaque (forward-compat fields we
  // pass through untouched); the known fields are validated below.
  const entry = { ...unknownFields, updatedAt } as ChannelNotifyEntry;
  if (level === "all" || level === "mentions" || level === "mute") {
    entry.level = level;
  }
  if (isValidTimestamp(muteUntil) && muteUntil > 0) entry.muteUntil = muteUntil;
  if (typeof desktop === "boolean") entry.desktop = desktop;
  if (typeof followAllThreads === "boolean") {
    entry.followAllThreads = followAllThreads;
  }
  if (typeof broadcasts === "boolean") entry.broadcasts = broadcasts;
  return entry;
}

export function parseNotifyPrefsPayload(
  json: unknown,
): ChannelNotifyPrefsStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const channels: Record<string, ChannelNotifyEntry> = {};
  if (
    typeof obj.channels === "object" &&
    obj.channels !== null &&
    !Array.isArray(obj.channels)
  ) {
    for (const [channelId, value] of Object.entries(
      obj.channels as Record<string, unknown>,
    )) {
      const entry = parseNotifyEntry(value);
      if (entry) channels[channelId] = entry;
    }
  }
  return { version: 1, channels };
}

/**
 * True when an entry carries no divergence from the defaults — and no unknown
 * fields, which must never be discarded by our pruning.
 */
export function isDefaultEntry(entry: ChannelNotifyEntry): boolean {
  for (const key of Object.keys(entry)) {
    if (!KNOWN_ENTRY_KEYS.has(key)) return false;
  }
  return (
    (entry.level === undefined || entry.level === "all") &&
    entry.muteUntil === undefined &&
    (entry.desktop === undefined || entry.desktop) &&
    (entry.followAllThreads === undefined || !entry.followAllThreads) &&
    (entry.broadcasts === undefined || entry.broadcasts)
  );
}

/**
 * Assign one channel's entry, keeping the store sparse.
 *
 * A default-valued entry is only materialized when it has to override an
 * existing non-default entry: LWW merge unions keys, so a deleted key would be
 * resurrected by our own older blob (or another device's) and silently re-apply
 * the level the user just cleared. Once the explicit default row is the newest
 * one everywhere, the next default-valued write drops it — resurrecting a
 * default row is harmless.
 */
export function setChannelEntry(
  store: ChannelNotifyPrefsStore,
  channelId: string,
  entry: ChannelNotifyEntry,
): ChannelNotifyPrefsStore {
  const prior = store.channels[channelId];
  if (isDefaultEntry(entry) && (prior === undefined || isDefaultEntry(prior))) {
    if (prior === undefined) return store;
    const channels = { ...store.channels };
    delete channels[channelId];
    return { version: 1, channels };
  }
  return { version: 1, channels: { ...store.channels, [channelId]: entry } };
}

export function readChannelNotifyPrefsStore(
  pubkey: string,
  relayUrl: string,
): ChannelNotifyPrefsStore {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey, relayUrl));
    if (!raw) return DEFAULT_STORE;
    return parseNotifyPrefsPayload(JSON.parse(raw)) ?? DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function writeChannelNotifyPrefsStore(
  pubkey: string,
  relayUrl: string,
  store: ChannelNotifyPrefsStore,
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

/** Per-channel max-`updatedAt` LWW merge over the union of keys; local wins ties. */
export function mergeStores(
  local: ChannelNotifyPrefsStore,
  remote: ChannelNotifyPrefsStore,
): ChannelNotifyPrefsStore {
  const channels: Record<string, ChannelNotifyEntry> = { ...local.channels };
  for (const [channelId, remoteEntry] of Object.entries(remote.channels)) {
    const localEntry = channels[channelId];
    if (!localEntry || remoteEntry.updatedAt > localEntry.updatedAt) {
      channels[channelId] = remoteEntry;
    }
  }
  return { version: 1, channels };
}

/**
 * Full-field entry equality (including unknown fields). Publish dedup must
 * compare everything — `channelMutesSync` comparing only `muted`/`updatedAt` is
 * a known trap that suppresses legitimate republishes.
 */
export function entriesEqual(
  a: ChannelNotifyEntry,
  b: ChannelNotifyEntry,
): boolean {
  const aRecord = a as Record<string, unknown>;
  const bRecord = b as Record<string, unknown>;
  const aKeys = Object.keys(aRecord);
  if (aKeys.length !== Object.keys(bRecord).length) return false;
  return aKeys.every((key) => aRecord[key] === bRecord[key]);
}

export function storesEqual(
  a: ChannelNotifyPrefsStore,
  b: ChannelNotifyPrefsStore,
): boolean {
  const aKeys = Object.keys(a.channels);
  if (aKeys.length !== Object.keys(b.channels).length) return false;
  return aKeys.every((channelId) => {
    const other = b.channels[channelId];
    return other !== undefined && entriesEqual(a.channels[channelId], other);
  });
}
