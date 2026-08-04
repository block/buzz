import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { ChannelSortGroupKey } from "./channelSortPreference";
import {
  DEFAULT_MANUAL_ORDER_STORE,
  mergeDeletedSectionOrder,
  moveManualChannel,
  orderIdsForGroup,
  pruneManualOrderGroups,
  setManualGroupEnabled,
  setManualGroupOrder,
  type ChannelManualOrderStore,
} from "./channelManualOrder";
import {
  channelManualOrderStorageKey,
  readChannelManualOrderStore,
  writeChannelManualOrderStore,
} from "./channelManualOrderStorage";
import {
  ChannelManualOrderSyncManager,
  type RemoteManualOrder,
} from "./channelManualOrderSync";

export function useChannelManualOrder(
  pubkey: string | undefined,
  relayUrl: string | undefined,
  liveSectionIds: readonly string[],
): {
  orderIds: (
    group: ChannelSortGroupKey,
    liveIds: readonly string[],
  ) => string[];
  seedOrder: (
    group: ChannelSortGroupKey,
    visibleIds: readonly string[],
  ) => void;
  isManual: (group: ChannelSortGroupKey) => boolean;
  setManualMode: (group: ChannelSortGroupKey, enabled: boolean) => void;
  mergeDeletedSection: (
    sectionId: string,
    sectionChannelIds: readonly string[],
    channelIds: readonly string[],
  ) => void;
  moveChannel: (input: {
    channelId: string;
    sourceGroup: ChannelSortGroupKey;
    targetGroup: ChannelSortGroupKey;
    overChannelId?: string;
    sourceLiveIds: readonly string[];
    targetLiveIds: readonly string[];
  }) => void;
} {
  const [store, setStore] = React.useState<ChannelManualOrderStore>(() =>
    pubkey
      ? readChannelManualOrderStore(pubkey, relayUrl)
      : DEFAULT_MANUAL_ORDER_STORE,
  );
  const managerRef = React.useRef<ChannelManualOrderSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_MANUAL_ORDER_STORE);
      return;
    }
    setStore(readChannelManualOrderStore(pubkey, relayUrl));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ChannelManualOrderSyncManager(pubkey);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!pubkey) return;
    const key = channelManualOrderStorageKey(pubkey, relayUrl);
    const handleStorage = (event: StorageEvent) => {
      if (event.key === key) {
        setStore(readChannelManualOrderStore(pubkey, relayUrl));
      }
    };
    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, [pubkey, relayUrl]);

  const applyRemote = React.useCallback(
    (remote: RemoteManualOrder) => {
      if (!pubkey) return;
      if (managerRef.current?.shouldApplyRemote(remote) === false) return;
      if (remote.createdAt < lastAppliedRemoteTs.current) return;
      if (
        remote.createdAt === lastAppliedRemoteTs.current &&
        remote.eventId <= lastAppliedEventId.current
      )
        return;
      lastAppliedRemoteTs.current = remote.createdAt;
      lastAppliedEventId.current = remote.eventId;
      if (writeChannelManualOrderStore(pubkey, remote.store, relayUrl)) {
        setStore(remote.store);
      }
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    let unsubscribe: (() => Promise<void>) | null = null;
    void managerRef.current?.fetchRemote().then((remote) => {
      if (cancelled) return;
      if (remote) {
        applyRemote(remote);
      } else {
        const local = readChannelManualOrderStore(pubkey, relayUrl);
        if (Object.keys(local.groups).length > 0) {
          managerRef.current?.publish(local);
        }
      }
    });
    void managerRef.current?.subscribe(applyRemote).then((dispose) => {
      if (cancelled) void dispose();
      else unsubscribe = dispose;
    });
    const unsubscribeReconnect = relayClient.subscribeToReconnects(() => {
      void managerRef.current?.fetchRemote().then((remote) => {
        if (!cancelled && remote) applyRemote(remote);
        const pending = managerRef.current?.getPendingStore();
        if (!cancelled && pending) managerRef.current?.publish(pending);
      });
    });
    return () => {
      cancelled = true;
      unsubscribeReconnect();
      if (unsubscribe) void unsubscribe();
    };
  }, [pubkey, relayUrl, applyRemote]);

  const commit = React.useCallback(
    (
      updater: (current: ChannelManualOrderStore) => ChannelManualOrderStore,
    ) => {
      if (!pubkey) return;
      setStore((current) => {
        const next = pruneManualOrderGroups(updater(current), liveSectionIds);
        if (!writeChannelManualOrderStore(pubkey, next, relayUrl)) {
          return current;
        }
        managerRef.current?.publish(next);
        return next;
      });
    },
    [pubkey, relayUrl, liveSectionIds],
  );

  const orderIds = React.useCallback(
    (group: ChannelSortGroupKey, liveIds: readonly string[]) =>
      orderIdsForGroup(store, group, liveIds),
    [store],
  );

  React.useEffect(() => {
    if (!pubkey) return;
    setStore((current) => {
      const next = pruneManualOrderGroups(current, liveSectionIds);
      if (next === current) return current;
      if (!writeChannelManualOrderStore(pubkey, next, relayUrl)) {
        return current;
      }
      managerRef.current?.publish(next);
      return next;
    });
  }, [pubkey, relayUrl, liveSectionIds]);

  const seedOrder = React.useCallback(
    (group: ChannelSortGroupKey, visibleIds: readonly string[]) => {
      commit((current) => setManualGroupOrder(current, group, visibleIds));
    },
    [commit],
  );

  const isManual = React.useCallback(
    (group: ChannelSortGroupKey) => store.manualGroups.includes(group),
    [store.manualGroups],
  );

  const setManualMode = React.useCallback(
    (group: ChannelSortGroupKey, enabled: boolean) => {
      commit((current) => setManualGroupEnabled(current, group, enabled));
    },
    [commit],
  );

  const moveChannel = React.useCallback(
    (input: Parameters<typeof moveManualChannel>[1]) => {
      commit((current) => moveManualChannel(current, input));
    },
    [commit],
  );

  const mergeDeletedSection = React.useCallback(
    (
      sectionId: string,
      sectionChannelIds: readonly string[],
      channelIds: readonly string[],
    ) => {
      commit((current) =>
        mergeDeletedSectionOrder(
          current,
          sectionId,
          sectionChannelIds,
          channelIds,
        ),
      );
    },
    [commit],
  );

  return React.useMemo(
    () => ({
      orderIds,
      seedOrder,
      isManual,
      setManualMode,
      mergeDeletedSection,
      moveChannel,
    }),
    [
      isManual,
      mergeDeletedSection,
      moveChannel,
      orderIds,
      seedOrder,
      setManualMode,
    ],
  );
}
