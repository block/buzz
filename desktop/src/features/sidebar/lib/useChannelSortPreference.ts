import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
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
import { ChannelSortSyncManager } from "./channelSortSync";
import type { RemoteSortPrefs } from "./channelSortSync";

/**
 * Persistent per-group sidebar sort preferences, scoped by pubkey + relay so
 * they don't bleed across identities or workspaces (same scoping as channel
 * sections). Each sidebar grouping (starred, channels, forums, dms, and each
 * custom section) carries its own saved Recent/A–Z mode; unset groups default
 * to A–Z. Mirrors changes made in other windows via the storage event.
 *
 * Preferences sync across clients via encrypted NIP-78 app data (kind 30078,
 * d-tag `channel-sort`), following the channel-sections pattern: localStorage
 * stays the instant/offline cache, the relay blob is the cross-client source
 * of truth, and conflicts resolve with whole-blob last-write-wins.
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

  const managerRef = React.useRef<ChannelSortSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readChannelSortStore(pubkey, relayUrl));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ChannelSortSyncManager(pubkey);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
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

  const applyRemote = React.useCallback(
    (
      remote: RemoteSortPrefs,
    ): ((prev: ChannelSortStore) => ChannelSortStore) => {
      return (prev) => {
        if (!pubkey) return prev;
        if (remote.createdAt < lastAppliedRemoteTs.current) return prev;
        if (
          remote.createdAt === lastAppliedRemoteTs.current &&
          remote.eventId <= lastAppliedEventId.current
        )
          return prev;
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        managerRef.current?.cancelPendingPublish();
        if (!writeChannelSortStore(pubkey, remote.store, relayUrl)) return prev;
        return remote.store;
      };
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    void managerRef.current?.fetchRemoteSortPrefs().then((remote) => {
      if (cancelled) return;
      if (remote) {
        setStore(applyRemote(remote));
      } else {
        const local = readChannelSortStore(pubkey, relayUrl);
        if (Object.keys(local.groups).length > 0) {
          managerRef.current?.publishSortPrefs(local);
        }
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, relayUrl, applyRemote]);

  React.useEffect(() => {
    if (!pubkey) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void managerRef.current
      ?.subscribeToSortPrefs((remote) => {
        if (cancelled) return;
        setStore(applyRemote(remote));
      })
      .then((dispose) => {
        if (cancelled) {
          void dispose();
        } else {
          unsub = dispose;
        }
      });
    return () => {
      cancelled = true;
      if (unsub) void unsub();
    };
  }, [pubkey, applyRemote]);

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    const unsub = relayClient.subscribeToReconnects(() => {
      void managerRef.current?.fetchRemoteSortPrefs().then((remote) => {
        if (cancelled) return;
        if (remote) {
          setStore(applyRemote(remote));
        }
        const pending = managerRef.current?.getPendingStore();
        if (pending) {
          managerRef.current?.publishSortPrefs(pending);
        }
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, applyRemote]);

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
        managerRef.current?.publishSortPrefs(next);
        return next;
      });
    },
    [pubkey, relayUrl, liveSectionIds],
  );

  return { sortModeFor, setSortModeFor };
}
