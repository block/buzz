const STORAGE_KEY_PREFIX = "buzz-channel-mutes.v1";
export const MAX_CHANNEL_MUTE_ENTRIES = 500;

export type ChannelMuteEntry = {
  muted: boolean;
  updatedAt: number;
};

export type ChannelMuteStore = {
  version: 1;
  channels: Record<string, ChannelMuteEntry>;
};

export const DEFAULT_STORE: ChannelMuteStore = Object.freeze({
  version: 1,
  channels: {},
});

export function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

export function parseMutePayload(json: unknown): ChannelMuteStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const channels: Record<string, ChannelMuteEntry> =
    typeof obj.channels === "object" &&
    obj.channels !== null &&
    !Array.isArray(obj.channels)
      ? Object.fromEntries(
          Object.entries(obj.channels as Record<string, unknown>).filter(
            (entry): entry is [string, ChannelMuteEntry] => {
              const v = entry[1];
              return (
                typeof v === "object" &&
                v !== null &&
                typeof (v as Record<string, unknown>).muted === "boolean" &&
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
  return boundMuteStore({ version: 1, channels });
}

export function readChannelMutesStore(pubkey: string): ChannelMuteStore {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey));
    if (!raw) {
      return DEFAULT_STORE;
    }
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || parsed.version !== 1) {
      return DEFAULT_STORE;
    }
    return parseMutePayload(parsed) ?? DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function boundMuteStore(
  store: ChannelMuteStore,
  preservedKey?: string,
): ChannelMuteStore {
  const preservedEntry =
    preservedKey === undefined ? undefined : store.channels[preservedKey];
  const entries = Object.entries(store.channels).filter(
    ([channelId]) => channelId !== preservedKey,
  );
  if (entries.length + (preservedEntry ? 1 : 0) <= MAX_CHANNEL_MUTE_ENTRIES)
    return store;
  entries.sort(([leftId, left], [rightId, right]) => {
    if (left.updatedAt !== right.updatedAt)
      return left.updatedAt - right.updatedAt;
    return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
  });
  const retainedEntries = entries.slice(
    -(MAX_CHANNEL_MUTE_ENTRIES - (preservedEntry ? 1 : 0)),
  );
  if (preservedEntry && preservedKey !== undefined) {
    retainedEntries.push([preservedKey, preservedEntry]);
  }
  return {
    ...store,
    channels: Object.fromEntries(retainedEntries),
  };
}

export function writeChannelMutesStore(
  pubkey: string,
  store: ChannelMuteStore,
): boolean {
  try {
    window.localStorage.setItem(
      storageKey(pubkey),
      JSON.stringify(boundMuteStore(store)),
    );
    return true;
  } catch {
    return false;
  }
}

export function mergeStores(
  local: ChannelMuteStore,
  remote: ChannelMuteStore,
): ChannelMuteStore {
  const allIds = new Set([
    ...Object.keys(local.channels),
    ...Object.keys(remote.channels),
  ]);
  const merged: Record<string, ChannelMuteEntry> = {};
  for (const id of allIds) {
    const l = local.channels[id];
    const r = remote.channels[id];
    if (l && r) {
      merged[id] = l.updatedAt >= r.updatedAt ? l : r;
    } else {
      merged[id] = (l ?? r) as ChannelMuteEntry;
    }
  }
  return boundMuteStore({ version: 1, channels: merged });
}

export function mutedChannelIdsFromStore(store: ChannelMuteStore): Set<string> {
  return new Set(
    Object.entries(store.channels)
      .filter(([, entry]) => entry.muted)
      .map(([id]) => id),
  );
}
