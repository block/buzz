import { getStorageItem } from "@/shared/lib/safeStorage";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

/** Most distinct emoji the recent/frequent cache keeps. */
export const EMOJI_USAGE_CAP = 48;
const STORAGE_KEY_PREFIX = "buzz-emoji-usage.v1";

type UsageEntry = CustomEmoji & { count: number; lastUsed: number };

let scopeKey: string | null = null;
let entries: ReadonlyArray<UsageEntry> = [];
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) listener();
}

function parseEntries(raw: string | null): UsageEntry[] {
  if (!raw) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const out: UsageEntry[] = [];
  for (const item of parsed) {
    if (typeof item !== "object" || item === null) continue;
    const { shortcode, url, count, lastUsed } = item as Record<string, unknown>;
    if (
      typeof shortcode === "string" &&
      typeof url === "string" &&
      typeof count === "number" &&
      typeof lastUsed === "number"
    ) {
      out.push({ shortcode, url, count, lastUsed });
    }
  }
  return out.sort((a, b) => b.lastUsed - a.lastUsed).slice(0, EMOJI_USAGE_CAP);
}

/** Load the cache for a community. Call after `resetEmojiUsage` on a community switch. */
export function loadEmojiUsage(relayUrl: string): void {
  const key = `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}`;
  if (key === scopeKey) return;
  scopeKey = key;
  entries = parseEntries(getStorageItem(key));
  notify();
}

/** Record one use: one in-memory update and one persisted write. */
export function recordEmojiUse(emoji: CustomEmoji, now = Date.now()): void {
  const existing = entries.find((entry) => entry.shortcode === emoji.shortcode);
  const next: UsageEntry = {
    shortcode: emoji.shortcode,
    url: emoji.url,
    count: (existing?.count ?? 0) + 1,
    lastUsed: now,
  };
  entries = [
    next,
    ...entries.filter((entry) => entry.shortcode !== emoji.shortcode),
  ].slice(0, EMOJI_USAGE_CAP);
  if (scopeKey !== null) {
    if (!setLocalStorageItemWithRecovery(scopeKey, JSON.stringify(entries))) {
      console.warn("Could not save recent emoji; keeping them in memory.");
    }
  }
  notify();
}

/** Newest first. */
export function recentEmoji(limit: number): CustomEmoji[] {
  return entries
    .slice(0, limit)
    .map(({ shortcode, url }) => ({ shortcode, url }));
}

/** Most used first; ties go to the newer use. */
export function frequentEmoji(limit: number): CustomEmoji[] {
  return [...entries]
    .sort((a, b) => b.count - a.count || b.lastUsed - a.lastUsed)
    .slice(0, limit)
    .map(({ shortcode, url }) => ({ shortcode, url }));
}

export function subscribeEmojiUsage(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable until the next change, for `useSyncExternalStore`. */
export function getEmojiUsageSnapshot(): ReadonlyArray<CustomEmoji> {
  return entries;
}

export function resetEmojiUsage(): void {
  scopeKey = null;
  entries = [];
  notify();
}
