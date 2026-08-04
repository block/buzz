import type { ChannelSortGroupKey } from "./channelSortPreference";

export type ChannelManualOrderStore = {
  version: 1;
  groups: Record<string, string[]>;
  manualGroups: string[];
};

export const DEFAULT_MANUAL_ORDER_STORE: ChannelManualOrderStore =
  Object.freeze({
    version: 1,
    groups: {},
    manualGroups: [],
  });

const MAX_MANUAL_ORDER_GROUPS = 1_000;
const MAX_CHANNELS_PER_GROUP = 10_000;

function uniqueStrings(values: unknown[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (typeof value !== "string" || seen.has(value)) continue;
    seen.add(value);
    result.push(value);
    if (result.length >= MAX_CHANNELS_PER_GROUP) break;
  }
  return result;
}

export function parseChannelManualOrderPayload(
  json: unknown,
): ChannelManualOrderStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const groups =
    typeof obj.groups === "object" &&
    obj.groups !== null &&
    !Array.isArray(obj.groups)
      ? Object.fromEntries(
          Object.entries(obj.groups as Record<string, unknown>)
            .slice(0, MAX_MANUAL_ORDER_GROUPS)
            .flatMap(([key, value]) =>
              Array.isArray(value) ? [[key, uniqueStrings(value)]] : [],
            ),
        )
      : {};
  const manualGroups = Array.isArray(obj.manualGroups)
    ? uniqueStrings(obj.manualGroups).slice(0, MAX_MANUAL_ORDER_GROUPS)
    : [];
  return { version: 1, groups, manualGroups };
}

export function normalizeManualOrder(
  savedIds: readonly string[] | undefined,
  liveIds: readonly string[],
): string[] {
  const live = new Set(liveIds);
  const ordered = uniqueStrings([...(savedIds ?? [])]).filter((id) =>
    live.has(id),
  );
  const known = new Set(ordered);
  for (const id of liveIds) {
    if (ordered.length >= MAX_CHANNELS_PER_GROUP) break;
    if (!known.has(id)) {
      known.add(id);
      ordered.push(id);
    }
  }
  return ordered;
}

export function orderIdsForGroup(
  store: ChannelManualOrderStore,
  group: ChannelSortGroupKey,
  liveIds: readonly string[],
): string[] {
  return normalizeManualOrder(store.groups[group], liveIds);
}

export function setManualGroupOrder(
  store: ChannelManualOrderStore,
  group: ChannelSortGroupKey,
  orderedIds: readonly string[],
): ChannelManualOrderStore {
  return {
    ...store,
    groups: {
      ...store.groups,
      [group]: uniqueStrings([...orderedIds]),
    },
  };
}

export function setManualGroupEnabled(
  store: ChannelManualOrderStore,
  group: ChannelSortGroupKey,
  enabled: boolean,
): ChannelManualOrderStore {
  const manualGroups = new Set(store.manualGroups);
  if (enabled) manualGroups.add(group);
  else manualGroups.delete(group);
  return { ...store, manualGroups: [...manualGroups] };
}

export function moveManualChannel(
  store: ChannelManualOrderStore,
  input: {
    channelId: string;
    sourceGroup: ChannelSortGroupKey;
    targetGroup: ChannelSortGroupKey;
    overChannelId?: string;
    sourceLiveIds: readonly string[];
    targetLiveIds: readonly string[];
  },
): ChannelManualOrderStore {
  const {
    channelId,
    sourceGroup,
    targetGroup,
    overChannelId,
    sourceLiveIds,
    targetLiveIds,
  } = input;
  const sourceWithActive = normalizeManualOrder(
    store.groups[sourceGroup],
    sourceLiveIds,
  );
  // First user reorder (or cross-group move) enables Manual for the
  // affected groups so the preference persists without a mount-time seed.
  const enableManual = (groups: string[]) => {
    const next = new Set(store.manualGroups);
    for (const group of groups) next.add(group);
    return [...next];
  };

  if (sourceGroup === targetGroup) {
    const oldIndex = sourceWithActive.indexOf(channelId);
    const newIndex = overChannelId
      ? sourceWithActive.indexOf(overChannelId)
      : sourceWithActive.length - 1;
    if (oldIndex === -1 || newIndex === -1 || oldIndex === newIndex) {
      return store;
    }
    const target = [...sourceWithActive];
    target.splice(oldIndex, 1);
    target.splice(newIndex, 0, channelId);
    return {
      ...store,
      groups: { ...store.groups, [targetGroup]: target },
      manualGroups: enableManual([targetGroup]),
    };
  }
  const source = sourceWithActive.filter((id) => id !== channelId);
  const targetBase = normalizeManualOrder(
    store.groups[targetGroup],
    targetLiveIds,
  ).filter((id) => id !== channelId);
  const target = [...targetBase];
  const overIndex = overChannelId
    ? target.indexOf(overChannelId)
    : target.length;
  target.splice(overIndex === -1 ? target.length : overIndex, 0, channelId);

  const groups = { ...store.groups, [targetGroup]: target };
  groups[sourceGroup] = source;
  return {
    ...store,
    groups,
    manualGroups: enableManual([sourceGroup, targetGroup]),
  };
}

export function mergeDeletedSectionOrder(
  store: ChannelManualOrderStore,
  sectionId: string,
  sectionChannelIds: readonly string[],
  channelIds: readonly string[],
): ChannelManualOrderStore {
  const sectionGroup = `section:${sectionId}` as const;
  const groups = { ...store.groups };
  delete groups[sectionGroup];
  groups.channels = uniqueStrings([...channelIds, ...sectionChannelIds]);
  return {
    ...store,
    groups,
    manualGroups: store.manualGroups.filter((group) => group !== sectionGroup),
  };
}

export function pruneManualOrderGroups(
  store: ChannelManualOrderStore,
  liveSectionIds: readonly string[],
): ChannelManualOrderStore {
  const liveGroups = new Set<string>([
    "channels",
    ...liveSectionIds.map((id) => `section:${id}`),
  ]);
  const groups = Object.fromEntries(
    Object.entries(store.groups).filter(([key]) => liveGroups.has(key)),
  );
  const manualGroups = store.manualGroups.filter((key) => liveGroups.has(key));
  if (
    Object.keys(groups).length === Object.keys(store.groups).length &&
    manualGroups.length === store.manualGroups.length
  ) {
    return store;
  }
  return { ...store, groups, manualGroups };
}
