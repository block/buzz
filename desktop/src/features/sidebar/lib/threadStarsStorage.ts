import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

const STORAGE_KEY_PREFIX = "buzz-thread-stars.v1";
export const MAX_THREAD_STAR_ENTRIES = 500;

export type StarredThreadEntry = {
  rootId: string;
  channelId: string;
  /** Short label for the sidebar row (usually a root-message excerpt). */
  title: string;
  channelName: string;
  starredAt: number;
};

export type ThreadStarStore = {
  version: 1;
  threads: Record<string, StarredThreadEntry>;
};

export const DEFAULT_THREAD_STAR_STORE: ThreadStarStore = Object.freeze({
  version: 1,
  threads: {},
});

export function threadStarsStorageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

function isStarredThreadEntry(value: unknown): value is StarredThreadEntry {
  if (typeof value !== "object" || value === null) return false;
  const entry = value as Record<string, unknown>;
  return (
    typeof entry.rootId === "string" &&
    entry.rootId.length > 0 &&
    typeof entry.channelId === "string" &&
    entry.channelId.length > 0 &&
    typeof entry.title === "string" &&
    typeof entry.channelName === "string" &&
    typeof entry.starredAt === "number" &&
    Number.isFinite(entry.starredAt) &&
    entry.starredAt >= 0
  );
}

export function parseThreadStarPayload(json: unknown): ThreadStarStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const threads: Record<string, StarredThreadEntry> =
    typeof obj.threads === "object" &&
    obj.threads !== null &&
    !Array.isArray(obj.threads)
      ? Object.fromEntries(
          Object.entries(obj.threads as Record<string, unknown>).filter(
            (entry): entry is [string, StarredThreadEntry] => {
              if (!isStarredThreadEntry(entry[1])) return false;
              // Key must match the entry's rootId so lookups stay consistent.
              return entry[0] === entry[1].rootId;
            },
          ),
        )
      : {};
  return boundThreadStarStore({ version: 1, threads });
}

export function readThreadStarsStore(pubkey: string): ThreadStarStore {
  const raw = getStorageItem(threadStarsStorageKey(pubkey));
  if (!raw) {
    return DEFAULT_THREAD_STAR_STORE;
  }
  try {
    return parseThreadStarPayload(JSON.parse(raw)) ?? DEFAULT_THREAD_STAR_STORE;
  } catch {
    return DEFAULT_THREAD_STAR_STORE;
  }
}

export function boundThreadStarStore(
  store: ThreadStarStore,
  preservedKey?: string,
): ThreadStarStore {
  const preservedEntry =
    preservedKey === undefined ? undefined : store.threads[preservedKey];
  const entries = Object.entries(store.threads).filter(
    ([rootId]) => rootId !== preservedKey,
  );
  if (entries.length + (preservedEntry ? 1 : 0) <= MAX_THREAD_STAR_ENTRIES) {
    return store;
  }
  entries.sort(([leftId, left], [rightId, right]) => {
    if (left.starredAt !== right.starredAt) {
      return left.starredAt - right.starredAt;
    }
    return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
  });
  const retainedEntries = entries.slice(
    -(MAX_THREAD_STAR_ENTRIES - (preservedEntry ? 1 : 0)),
  );
  if (preservedEntry && preservedKey !== undefined) {
    retainedEntries.push([preservedKey, preservedEntry]);
  }
  return {
    ...store,
    threads: Object.fromEntries(retainedEntries),
  };
}

export function writeThreadStarsStore(
  pubkey: string,
  store: ThreadStarStore,
): boolean {
  return setStorageItem(
    threadStarsStorageKey(pubkey),
    JSON.stringify(boundThreadStarStore(store)),
  );
}

export function starredThreadEntriesFromStore(
  store: ThreadStarStore,
): StarredThreadEntry[] {
  return Object.values(store.threads)
    .slice()
    .sort((left, right) => {
      if (left.starredAt !== right.starredAt) {
        return right.starredAt - left.starredAt;
      }
      return left.rootId < right.rootId
        ? -1
        : left.rootId > right.rootId
          ? 1
          : 0;
    });
}

export function starredThreadIdsFromStore(store: ThreadStarStore): Set<string> {
  return new Set(Object.keys(store.threads));
}
