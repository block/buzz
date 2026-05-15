// Persists the in-session "latest message at" map per pubkey so unread
// indicators survive an app restart. See useUnreadChannels for the consumer.
//
// Without this, the sidebar unread comparison (`latest > readMarker`) has its
// `latest` side reset to empty on every startup — read markers persist via
// NIP-RS, but there is nothing to compare them against, and badges silently
// disappear until a new live message arrives.
//
// Stored shape: { [channelId]: unixSeconds }. Per-pubkey. Callers write
// monotonically; this module validates on read and doesn't trust the file.

const STORAGE_KEY_PREFIX = "sprout.channel-latest-message.v1";

// Cap entries written to localStorage to keep the blob bounded even for users
// in thousands of channels. Same order of magnitude as readStateFormat's
// MAX_CONTEXTS.
const MAX_ENTRIES = 10_000;

function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function readStoredLatestMessages(pubkey: string): Map<string, number> {
  const result = new Map<string, number>();
  let raw: string | null;
  try {
    raw = localStorage.getItem(storageKey(pubkey));
  } catch {
    return result;
  }
  if (!raw) return result;

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return result;
  }
  if (!isPlainRecord(parsed)) return result;

  for (const [channelId, value] of Object.entries(parsed)) {
    if (typeof channelId !== "string" || channelId.length === 0) continue;
    if (typeof value !== "number" || !Number.isInteger(value)) continue;
    if (value < 0 || value > 4_294_967_295) continue;
    result.set(channelId, value);
  }

  return result;
}

export function writeStoredLatestMessages(
  pubkey: string,
  latest: ReadonlyMap<string, number>,
): void {
  const state: Record<string, number> = {};
  let count = 0;
  for (const [channelId, timestamp] of latest) {
    if (count >= MAX_ENTRIES) break;
    state[channelId] = timestamp;
    count += 1;
  }

  try {
    localStorage.setItem(storageKey(pubkey), JSON.stringify(state));
  } catch {
    // Quota/serialization failure — non-fatal. Worst case is that one
    // restart loses badges, which is the status quo we're fixing.
  }
}
