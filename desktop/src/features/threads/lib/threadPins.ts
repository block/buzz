/**
 * Pinned threads are a personal view preference, not shared thread metadata,
 * so they never touch the title artifact. The store is mirrored to
 * localStorage and synced across the user's devices as a self-encrypted
 * NIP-78 blob (see `threadPinsSync.ts`). Unpins are kept as tombstones so a
 * per-root last-writer-wins merge cannot resurrect them.
 */
const THREAD_PINS_PREFIX = "buzz.threads.pins.v1";
/** Upper bound on stored roots, pins and tombstones together. */
export const MAX_THREAD_PIN_ENTRIES = 500;

export type ThreadPinEntry = { pinned: boolean; updatedAt: number };

export type ThreadPinStore = {
  version: 1;
  /** Thread root id (lowercase hex) → latest pin state. */
  roots: Record<string, ThreadPinEntry>;
};

export const EMPTY_THREAD_PIN_STORE: ThreadPinStore = Object.freeze({
  version: 1,
  roots: Object.freeze({}),
}) as ThreadPinStore;

const ROOT_ID_RE = /^[0-9a-f]{64}$/;

export function threadPinsStorageKey(relayOrigin: string, pubkey: string) {
  return `${THREAD_PINS_PREFIX}.${encodeURIComponent(relayOrigin)}.${pubkey.toLowerCase()}`;
}

export function parseThreadPinPayload(value: unknown): ThreadPinStore | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const { version, roots } = value as Record<string, unknown>;
  if (version !== 1) return null;
  if (!roots || typeof roots !== "object" || Array.isArray(roots)) return null;
  return {
    version: 1,
    roots: Object.fromEntries(
      Object.entries(roots as Record<string, unknown>).filter(
        (entry): entry is [string, ThreadPinEntry] => {
          const pin = entry[1] as Record<string, unknown> | null;
          return (
            ROOT_ID_RE.test(entry[0]) &&
            typeof pin === "object" &&
            pin !== null &&
            typeof pin.pinned === "boolean" &&
            typeof pin.updatedAt === "number" &&
            Number.isFinite(pin.updatedAt) &&
            pin.updatedAt >= 0
          );
        },
      ),
    ),
  };
}

/** Keeps the store bounded: tombstones go first, then the oldest pins. */
function capEntries(roots: Record<string, ThreadPinEntry>) {
  const entries = Object.entries(roots);
  if (entries.length <= MAX_THREAD_PIN_ENTRIES) return roots;
  entries.sort(
    ([, left], [, right]) =>
      Number(right.pinned) - Number(left.pinned) ||
      right.updatedAt - left.updatedAt,
  );
  return Object.fromEntries(entries.slice(0, MAX_THREAD_PIN_ENTRIES));
}

export function togglePin(
  store: ThreadPinStore,
  rootId: string,
  nowMs: number,
): ThreadPinStore {
  const root = rootId.toLowerCase();
  const current = store.roots[root];
  // Strictly after the previous write, so a clock that stepped backwards
  // still produces a toggle that wins the merge.
  const updatedAt = Math.max(nowMs, (current?.updatedAt ?? 0) + 1);
  return {
    version: 1,
    roots: capEntries({
      ...store.roots,
      [root]: { pinned: !current?.pinned, updatedAt },
    }),
  };
}

/** Per-root last-writer-wins; an exact tie resolves to unpinned. */
export function mergeThreadPinStores(
  local: ThreadPinStore,
  remote: ThreadPinStore,
): ThreadPinStore {
  const roots: Record<string, ThreadPinEntry> = { ...local.roots };
  for (const [root, remoteEntry] of Object.entries(remote.roots)) {
    const localEntry = roots[root];
    if (!localEntry || remoteEntry.updatedAt > localEntry.updatedAt) {
      roots[root] = remoteEntry;
    } else if (
      remoteEntry.updatedAt === localEntry.updatedAt &&
      remoteEntry.pinned !== localEntry.pinned
    ) {
      roots[root] = { pinned: false, updatedAt: localEntry.updatedAt };
    }
  }
  return { version: 1, roots: capEntries(roots) };
}

export function threadPinStoresEqual(
  left: ThreadPinStore,
  right: ThreadPinStore,
): boolean {
  const leftKeys = Object.keys(left.roots);
  if (leftKeys.length !== Object.keys(right.roots).length) return false;
  return leftKeys.every((root) => {
    const rightEntry = right.roots[root];
    return (
      rightEntry !== undefined &&
      rightEntry.pinned === left.roots[root].pinned &&
      rightEntry.updatedAt === left.roots[root].updatedAt
    );
  });
}

export function pinnedRoots(store: ThreadPinStore): ReadonlySet<string> {
  return new Set(
    Object.entries(store.roots)
      .filter(([, entry]) => entry.pinned)
      .map(([root]) => root),
  );
}

/**
 * Pinned threads first, then the rest. Both groups keep the incoming order,
 * which is already most-recent-activity-first.
 */
export function partitionByPin<T extends { rootId: string }>(
  entries: readonly T[],
  pinned: ReadonlySet<string>,
): { pinned: T[]; unpinned: T[] } {
  const pinnedEntries: T[] = [];
  const unpinned: T[] = [];
  for (const entry of entries) {
    (pinned.has(entry.rootId.toLowerCase()) ? pinnedEntries : unpinned).push(
      entry,
    );
  }
  return { pinned: pinnedEntries, unpinned };
}

export function readThreadPinStore(
  relayOrigin: string,
  pubkey: string,
): ThreadPinStore {
  try {
    const raw = globalThis.localStorage?.getItem(
      threadPinsStorageKey(relayOrigin, pubkey),
    );
    if (!raw) return EMPTY_THREAD_PIN_STORE;
    return parseThreadPinPayload(JSON.parse(raw)) ?? EMPTY_THREAD_PIN_STORE;
  } catch {
    return EMPTY_THREAD_PIN_STORE;
  }
}

/** Best-effort mirror; the caller's in-memory state stays authoritative. */
export function writeThreadPinStore(
  relayOrigin: string,
  pubkey: string,
  store: ThreadPinStore,
): void {
  try {
    globalThis.localStorage?.setItem(
      threadPinsStorageKey(relayOrigin, pubkey),
      JSON.stringify(store),
    );
  } catch {
    // The pin still applies for this session and syncs via the relay.
  }
}
