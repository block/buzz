const STORAGE_KEY_PREFIX = "buzz-bookmarks.v1";

/**
 * A single saved message. `bookmarked: false` is a tombstone kept in the store
 * so that un-bookmarking merges last-write-wins across devices (identical to the
 * channel-stars `starred` flag). The snapshot fields are denormalized so the
 * Saved view can render an entry without refetching the original message (and
 * survive the message later becoming inaccessible).
 */
export type BookmarkEntry = {
  bookmarked: boolean;
  updatedAt: number;
  /** Channel UUID the message lives in — for jump-to-context + refetch. */
  channelId: string;
  /** Denormalized snapshot captured at save time. */
  authorPubkey?: string;
  authorName?: string;
  preview?: string;
  createdAt?: number;
  /** Thread root id when the saved message is a reply — for jump-to-context. */
  threadRootId?: string;
};

export type BookmarkStore = {
  version: 1;
  bookmarks: Record<string, BookmarkEntry>;
};

export const DEFAULT_STORE: BookmarkStore = Object.freeze({
  version: 1,
  bookmarks: {},
});

/** Longest preview we persist; keeps the encrypted blob small. */
export const PREVIEW_MAX_LENGTH = 280;

export function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

function isNonNegativeFinite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function parseEntry(value: unknown): BookmarkEntry | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  if (typeof v.bookmarked !== "boolean") return null;
  if (!isNonNegativeFinite(v.updatedAt)) return null;
  if (typeof v.channelId !== "string" || v.channelId.length === 0) return null;
  const entry: BookmarkEntry = {
    bookmarked: v.bookmarked,
    updatedAt: v.updatedAt,
    channelId: v.channelId,
  };
  if (typeof v.authorPubkey === "string") entry.authorPubkey = v.authorPubkey;
  if (typeof v.authorName === "string") entry.authorName = v.authorName;
  if (typeof v.preview === "string") {
    entry.preview = v.preview.slice(0, PREVIEW_MAX_LENGTH);
  }
  if (isNonNegativeFinite(v.createdAt)) entry.createdAt = v.createdAt;
  if (typeof v.threadRootId === "string" && v.threadRootId.length > 0) {
    entry.threadRootId = v.threadRootId;
  }
  return entry;
}

export function parseBookmarkPayload(json: unknown): BookmarkStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const bookmarks: Record<string, BookmarkEntry> =
    typeof obj.bookmarks === "object" &&
    obj.bookmarks !== null &&
    !Array.isArray(obj.bookmarks)
      ? Object.fromEntries(
          Object.entries(obj.bookmarks as Record<string, unknown>)
            .map(([id, value]) => [id, parseEntry(value)] as const)
            .filter(
              (entry): entry is [string, BookmarkEntry] => entry[1] !== null,
            ),
        )
      : {};
  return { version: 1, bookmarks };
}

export function readBookmarkStore(pubkey: string): BookmarkStore {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey));
    if (!raw) return DEFAULT_STORE;
    const parsed = JSON.parse(raw);
    return parseBookmarkPayload(parsed) ?? DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function writeBookmarkStore(
  pubkey: string,
  store: BookmarkStore,
): boolean {
  try {
    window.localStorage.setItem(storageKey(pubkey), JSON.stringify(store));
    return true;
  } catch {
    return false;
  }
}

/** Per-key last-write-wins by `updatedAt` (mirror of channel-stars merge). */
export function mergeStores(
  local: BookmarkStore,
  remote: BookmarkStore,
): BookmarkStore {
  const allIds = new Set([
    ...Object.keys(local.bookmarks),
    ...Object.keys(remote.bookmarks),
  ]);
  const merged: Record<string, BookmarkEntry> = {};
  for (const id of allIds) {
    const l = local.bookmarks[id];
    const r = remote.bookmarks[id];
    if (l && r) {
      merged[id] = l.updatedAt >= r.updatedAt ? l : r;
    } else {
      merged[id] = (l ?? r) as BookmarkEntry;
    }
  }
  return { version: 1, bookmarks: merged };
}

/** Event ids of currently-saved messages (tombstones excluded). */
export function bookmarkedIdsFromStore(store: BookmarkStore): Set<string> {
  return new Set(
    Object.entries(store.bookmarks)
      .filter(([, entry]) => entry.bookmarked)
      .map(([id]) => id),
  );
}

/** Saved entries, newest-first, tombstones excluded — for the Saved view. */
export function savedEntriesFromStore(
  store: BookmarkStore,
): Array<{ eventId: string; entry: BookmarkEntry }> {
  return Object.entries(store.bookmarks)
    .filter(([, entry]) => entry.bookmarked)
    .map(([eventId, entry]) => ({ eventId, entry }))
    .sort((a, b) => b.entry.updatedAt - a.entry.updatedAt);
}
