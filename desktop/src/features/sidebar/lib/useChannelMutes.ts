import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  boundMuteStore,
  DEFAULT_STORE,
  mergeApplyingRemote,
  mergeStores,
  mutedChannelIdsFromStore,
  readChannelMutesStore,
  storageKey,
  writeChannelMutesStore,
  type ChannelMuteEntry,
  type ChannelMuteStore,
} from "./channelMutesStorage";
import { ChannelMuteSyncManager } from "./channelMutesSync";
import type { RemoteMutes } from "./channelMutesSync";

export function useChannelMutes(
  pubkey: string | undefined,
  relayUrl?: string,
): {
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

  const managerRef = React.useRef<ChannelMuteSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey || !relayUrl) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readChannelMutesStore(pubkey));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ChannelMuteSyncManager(pubkey, relayUrl);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

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
        // Equal timestamps: the relay/database break ties by `id ASC` — the
        // LOWEST event id is the canonical winner. Apply a strictly-lower id and
        // ignore any id >= the last applied, so the UI converges on the same
        // event the relay stored rather than the largest id seen.
        if (
          remote.createdAt === lastAppliedRemoteTs.current &&
          remote.eventId >= lastAppliedEventId.current
        )
          return prev;
        // A canonical supersession corrects an already-applied same-timestamp
        // LARGER-id head with the true winner: only here may the incoming blob's
        // per-entry values win an equal-`updatedAt` tie, and only here does the
        // pending publish (which reflected the superseded head) get cancelled.
        // Any other application (bootstrap / live / newer timestamp) merges over
        // optimistic local state with local-wins `mergeStores` and must NOT
        // cancel a pending local publish — otherwise a later same-second local
        // click (integer-second `updatedAt`) loses to an older remote entry that
        // decrypts late, and its publish is silently dropped.
        const isCanonicalSupersession =
          remote.createdAt === lastAppliedRemoteTs.current &&
          lastAppliedEventId.current !== "" &&
          remote.eventId < lastAppliedEventId.current;
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        const merged = isCanonicalSupersession
          ? mergeApplyingRemote(prev, remote.store)
          : mergeStores(prev, remote.store);
        if (isCanonicalSupersession) {
          managerRef.current?.cancelPendingMutePublish();
        }
        if (!writeChannelMutesStore(pubkey, merged)) return prev;
        return merged;
      };
    },
    [pubkey],
  );

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    const local = readChannelMutesStore(pubkey);
    void managerRef.current?.bootstrap(local).then((result) => {
      if (cancelled) return;
      if (result.action === "apply-remote") {
        setStore(applyRemote(result.data));
      }
      // "hold": seed already performed by bootstrap (if first-sync), or blocked.
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, relayUrl, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: relayUrl is intentional — rebinds subscription when the active relay changes even though it is not used inside the effect body directly (the manager via managerRef.current carries it)
  React.useEffect(() => {
    if (!pubkey) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void managerRef.current
      ?.subscribeToMutes((remote) => {
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
  }, [pubkey, relayUrl, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: relayUrl is intentional — rebinds reconnect listener when the active relay changes (community switch) even though it is not referenced directly inside the effect body
  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    const unsub = relayClient.subscribeToReconnects(() => {
      void managerRef.current?.fetchRemoteMutes().then((result) => {
        if (cancelled) return;
        if (result.status === "found") {
          setStore(applyRemote(result.data));
        }
        const pending = managerRef.current?.getPendingMuteStore();
        if (pending) {
          managerRef.current?.publishMutes(pending);
        }
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, relayUrl, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: store.channels is the relevant dep — the outer store identity can change without channels changing (e.g., on reconnect writes)
  const mutedChannelIds = React.useMemo(
    () => mutedChannelIdsFromStore(store),
    [store.channels],
  );

  const setMuteState = React.useCallback(
    (channelId: string, muted: boolean) => {
      if (!pubkey) return;
      const entry: ChannelMuteEntry = {
        muted,
        updatedAt: Math.floor(Date.now() / 1000),
      };
      setStore((prev) => {
        const next = boundMuteStore(
          {
            version: 1,
            channels: { ...prev.channels, [channelId]: entry },
          },
          channelId,
        );
        if (!writeChannelMutesStore(pubkey, next)) return prev;
        managerRef.current?.publishMutes(next);
        return next;
      });
    },
    [pubkey],
  );

  const muteChannel = React.useCallback(
    (channelId: string) => setMuteState(channelId, true),
    [setMuteState],
  );
  const unmuteChannel = React.useCallback(
    (channelId: string) => setMuteState(channelId, false),
    [setMuteState],
  );

  return {
    mutedChannelIds,
    muteChannel,
    unmuteChannel,
  };
}
