const STORAGE_KEY_PREFIX = "sprout-channel-stars.v1";

export type ChannelStarEntry = {
  starred: boolean;
  updatedAt: number;
};

export type ChannelStarStore = {
  version: 1;
  channels: Record<string, ChannelStarEntry>;
};

export const DEFAULT_STORE: ChannelStarStore = Object.freeze({
  version: 1,
  channels: {},
});

export function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

export function parseStarPayload(json: unknown): ChannelStarStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const channels: Record<string, ChannelStarEntry> =
    typeof obj.channels === "object" &&
    obj.channels !== null &&
    !Array.isArray(obj.channels)
      ? Object.fromEntries(
          Object.entries(obj.channels as Record<string, unknown>).filter(
            (entry): entry is [string, ChannelStarEntry] => {
              const v = entry[1];
              return (
                typeof v === "object" &&
                v !== null &&
                typeof (v as Record<string, unknown>).starred === "boolean" &&
                typeof (v as Record<string, unknown>).updatedAt === "number" &&
                Number.isFinite(
                  (v as Record<string, unknown>).updatedAt as number,
                ) &&
                ((v as Record<string, unknown>).updatedAt as number) >= 0
              );
            },
          ),
        )
      : {};
  return { version: 1, channels };
}

export function readChannelStarsStore(pubkey: string): ChannelStarStore {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey));
    if (!raw) {
      return DEFAULT_STORE;
    }
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || parsed.version !== 1) {
      return DEFAULT_STORE;
    }
    return parseStarPayload(parsed) ?? DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function writeChannelStarsStore(
  pubkey: string,
  store: ChannelStarStore,
): boolean {
  try {
    window.localStorage.setItem(storageKey(pubkey), JSON.stringify(store));
    return true;
  } catch {
    return false;
  }
}

export function mergeStores(
  local: ChannelStarStore,
  remote: ChannelStarStore,
): ChannelStarStore {
  const allIds = new Set([
    ...Object.keys(local.channels),
    ...Object.keys(remote.channels),
  ]);
  const merged: Record<string, ChannelStarEntry> = {};
  for (const id of allIds) {
    const l = local.channels[id];
    const r = remote.channels[id];
    if (l && r) {
      merged[id] = l.updatedAt >= r.updatedAt ? l : r;
    } else {
      merged[id] = (l ?? r) as ChannelStarEntry;
    }
  }
  return { version: 1, channels: merged };
}

export function starredChannelIdsFromStore(
  store: ChannelStarStore,
): Set<string> {
  return new Set(
    Object.entries(store.channels)
      .filter(([, entry]) => entry.starred)
      .map(([id]) => id),
  );
}
