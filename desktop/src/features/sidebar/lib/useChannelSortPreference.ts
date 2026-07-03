import * as React from "react";

import {
  DEFAULT_STORE,
  readChannelSortStore,
  storageKey,
  writeChannelSortStore,
  type ChannelSortMode,
  type ChannelSortStore,
} from "./channelSortPreference";

/**
 * Persistent sidebar channel sort preference, scoped by pubkey + relay so it
 * doesn't bleed across identities or workspaces (same scoping as channel
 * sections). Mirrors changes made in other windows via the storage event.
 */
export function useChannelSortPreference(
  pubkey: string | undefined,
  relayUrl?: string,
): {
  sortMode: ChannelSortMode;
  setSortMode: (mode: ChannelSortMode) => void;
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

  const setSortMode = React.useCallback(
    (mode: ChannelSortMode) => {
      if (!pubkey) return;
      setStore((prev) => {
        const next: ChannelSortStore = { ...prev, mode };
        if (!writeChannelSortStore(pubkey, next, relayUrl)) return prev;
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  return { sortMode: store.mode, setSortMode };
}
