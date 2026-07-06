import * as React from "react";

import {
  DEFAULT_STORE,
  readChannelSortStore,
  sortModeForGroup,
  storageKey,
  stripOrphanedSectionModes,
  writeChannelSortStore,
  type ChannelSortGroupKey,
  type ChannelSortMode,
  type ChannelSortStore,
} from "./channelSortPreference";

/**
 * Persistent per-group sidebar sort preferences, scoped by pubkey + relay so
 * they don't bleed across identities or workspaces (same scoping as channel
 * sections). Each sidebar grouping (starred, channels, forums, dms, and each
 * custom section) carries its own saved Recent/A–Z mode; unset groups default
 * to A–Z. Mirrors changes made in other windows via the storage event.
 *
 * When `liveSectionIds` is provided, writes also prune `section:<id>` entries
 * whose custom section no longer exists, so deleted sections don't leave
 * stale keys in localStorage.
 */
export function useChannelSortPreference(
  pubkey: string | undefined,
  relayUrl?: string,
  liveSectionIds?: string[],
): {
  sortModeFor: (group: ChannelSortGroupKey) => ChannelSortMode;
  setSortModeFor: (group: ChannelSortGroupKey, mode: ChannelSortMode) => void;
} {
  const [store, setStore] = React.useState<ChannelSortStore>(() => {
    if (!pubkey) return DEFAULT_STORE;
    return readChannelSortStore(pubkey, relayUrl);
  });

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_STORE);
      return;
    }
    setStore(readChannelSortStore(pubkey, relayUrl));
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!pubkey) return;
    const key = storageKey(pubkey, relayUrl);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) return;
      setStore(readChannelSortStore(pubkey, relayUrl));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey, relayUrl]);

  const sortModeFor = React.useCallback(
    (group: ChannelSortGroupKey) => sortModeForGroup(store, group),
    [store],
  );

  const setSortModeFor = React.useCallback(
    (group: ChannelSortGroupKey, mode: ChannelSortMode) => {
      if (!pubkey) return;
      setStore((prev) => {
        const withUpdate: ChannelSortStore = {
          ...prev,
          groups: { ...prev.groups, [group]: mode },
        };
        // Prune sort modes left behind by deleted custom sections on write so
        // the stored map can't grow unboundedly with stale `section:` keys.
        const next = liveSectionIds
          ? stripOrphanedSectionModes(withUpdate, liveSectionIds)
          : withUpdate;
        if (!writeChannelSortStore(pubkey, next, relayUrl)) return prev;
        return next;
      });
    },
    [pubkey, relayUrl, liveSectionIds],
  );

  return { sortModeFor, setSortModeFor };
}
