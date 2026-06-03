import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  DEFAULT_STORE,
  mutedChannelIdsFromStore,
  mergeStores,
  readChannelMutesStore,
  storageKey,
  writeChannelMutesStore,
  type ChannelMuteEntry,
  type ChannelMuteStore,
} from "./channelMutesStorage";
import {
  cancelPendingMutePublish,
  fetchRemoteMutes,
  getPendingMuteStore,
  publishMutes,
  resetMuteSyncState,
  subscribeToMutes,
} from "./channelMutesSync";
import type { RemoteMutes } from "./channelMutesSync";

export function useChannelMutes(pubkey: string | undefined): {
  mutedChannelIds: Set<string>;
  muteChannel: (channelId: string) => void;
  unmuteChannel: (channelId: string) => void;
} {
  const [store, setStore] = React.useState<ChannelMuteStore>(() => {
    if (!pubkey) {
      return DEFAULT_STORE;
    }
    return readChannelMutesStore(pubkey);
  });

  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readChannelMutesStore(pubkey));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    return () => {
      resetMuteSyncState();
    };
  }, [pubkey]);

  React.useEffect(() => {
    if (!pubkey) {
      return;
    }
    const key = storageKey(pubkey);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) {
        return;
      }
      setStore(readChannelMutesStore(pubkey));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey]);

  const applyRemote = React.useCallback(
    (remote: RemoteMutes): ((prev: ChannelMuteStore) => ChannelMuteStore) => {
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
        cancelPendingMutePublish();
        const merged = mergeStores(prev, remote.store);
        if (!writeChannelMutesStore(pubkey, merged)) return prev;
        return merged;
      };
    },
    [pubkey],
  );

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    void fetchRemoteMutes(pubkey).then((remote) => {
      if (cancelled) return;
      if (remote) {
        setStore(applyRemote(remote));
      } else {
        const local = readChannelMutesStore(pubkey);
        if (Object.keys(local.channels).length > 0) {
          publishMutes(local);
        }
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, applyRemote]);

  React.useEffect(() => {
    if (!pubkey) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void subscribeToMutes(pubkey, (remote) => {
      if (cancelled) return;
      setStore(applyRemote(remote));
    }).then((dispose) => {
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
      void fetchRemoteMutes(pubkey).then((remote) => {
        if (cancelled) return;
        if (remote) {
          setStore(applyRemote(remote));
        }
        const pending = getPendingMuteStore();
        if (pending) {
          publishMutes(pending);
        }
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: store.channels is the relevant dep — the outer store identity can change without channels changing (e.g., on reconnect writes)
  const mutedChannelIds = React.useMemo(
    () => mutedChannelIdsFromStore(store),
    [store.channels],
  );

  const muteChannel = React.useCallback(
    (channelId: string) => {
      if (!pubkey) return;
      const entry: ChannelMuteEntry = {
        muted: true,
        updatedAt: Math.floor(Date.now() / 1000),
      };
      setStore((prev) => {
        const next: ChannelMuteStore = mergeStores(prev, {
          version: 1,
          channels: { [channelId]: entry },
        });
        if (!writeChannelMutesStore(pubkey, next)) return prev;
        publishMutes(next);
        return next;
      });
    },
    [pubkey],
  );

  const unmuteChannel = React.useCallback(
    (channelId: string) => {
      if (!pubkey) return;
      const entry: ChannelMuteEntry = {
        muted: false,
        updatedAt: Math.floor(Date.now() / 1000),
      };
      setStore((prev) => {
        const next: ChannelMuteStore = mergeStores(prev, {
          version: 1,
          channels: { [channelId]: entry },
        });
        if (!writeChannelMutesStore(pubkey, next)) return prev;
        publishMutes(next);
        return next;
      });
    },
    [pubkey],
  );

  return {
    mutedChannelIds,
    muteChannel,
    unmuteChannel,
  };
}
