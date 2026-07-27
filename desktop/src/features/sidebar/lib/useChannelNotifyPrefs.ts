import * as React from "react";

import {
  nextTimedMuteExpiry,
  resolveChannelNotifyState,
  type ResolvedChannelNotifyState,
} from "@/features/notifications/lib/resolveChannelNotifyState";
import {
  scheduleTimedMuteRefresh,
  useTimedMuteVersion,
} from "@/features/notifications/lib/timedMuteTicker";
import { relayClient } from "@/shared/api/relayClient";
import {
  DEFAULT_STORE as DEFAULT_LEGACY_MUTE_STORE,
  type ChannelMuteStore,
} from "./channelMutesStorage";
import {
  DEFAULT_STORE,
  mergeStores,
  readChannelNotifyPrefsStore,
  setChannelEntry,
  storageKey,
  writeChannelNotifyPrefsStore,
  type ChannelNotifyEntry,
  type ChannelNotifyLevel,
  type ChannelNotifyPrefsStore,
} from "./channelNotifyPrefsStorage";
import {
  ChannelNotifyPrefsSyncManager,
  type RemoteNotifyPrefs,
} from "./channelNotifyPrefsSync";

/** Fields the advanced (per-channel) toggles can change. */
export type ChannelNotifyAdvancedPatch = {
  desktop?: boolean;
  followAllThreads?: boolean;
  broadcasts?: boolean;
};

export type UseChannelNotifyPrefs = {
  prefsStore: ChannelNotifyPrefsStore;
  /**
   * Resolved state for a channel. Reference-stable per channel until the store,
   * the legacy mutes, or a timed-mute expiry changes it, so consumers can pass
   * the result to memoized components.
   */
  resolveChannel: (channelId: string) => ResolvedChannelNotifyState;
  setChannelLevel: (channelId: string, level: ChannelNotifyLevel) => void;
  muteChannelUntil: (channelId: string, untilSeconds: number) => void;
  clearTimedMute: (channelId: string) => void;
  setChannelAdvanced: (
    channelId: string,
    patch: ChannelNotifyAdvancedPatch,
  ) => void;
};

function nowSeconds(): number {
  return Math.floor(Date.now() / 1_000);
}

/**
 * Owns the per-channel notification preferences blob (kind 30078, d-tag
 * `channel-notify-prefs`): local-first state, relay-scoped localStorage mirror,
 * cross-tab storage events, remote merge with a `(createdAt, eventId)`
 * watermark, and reconnect resync.
 *
 * This hook is deliberately single-purpose: it does **not** touch the legacy
 * `channel-mutes` blob. Callers that need the NIP-CN dual-write compose it
 * themselves — e.g. call `setChannelLevel(id, "mute")` alongside
 * `muteChannel(id)` from `useChannelMutes` (and `unmuteChannel` for the other
 * levels). Timed mutes are never dual-written: old clients cannot express them.
 *
 * `legacyMutes` is read-only input used for the interop half of resolution.
 */
export function useChannelNotifyPrefs(
  pubkey: string | undefined,
  relayUrl: string | undefined,
  legacyMutes: ChannelMuteStore = DEFAULT_LEGACY_MUTE_STORE,
): UseChannelNotifyPrefs {
  const [store, setStore] = React.useState<ChannelNotifyPrefsStore>(() =>
    pubkey && relayUrl
      ? readChannelNotifyPrefsStore(pubkey, relayUrl)
      : DEFAULT_STORE,
  );

  const managerRef = React.useRef<ChannelNotifyPrefsSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey || !relayUrl) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readChannelNotifyPrefsStore(pubkey, relayUrl));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ChannelNotifyPrefsSyncManager(pubkey);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    const key = storageKey(pubkey, relayUrl);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) return;
      setStore(readChannelNotifyPrefsStore(pubkey, relayUrl));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey, relayUrl]);

  const applyRemote = React.useCallback(
    (
      remote: RemoteNotifyPrefs,
    ): ((prev: ChannelNotifyPrefsStore) => ChannelNotifyPrefsStore) => {
      return (prev) => {
        if (!pubkey || !relayUrl) return prev;
        if (remote.createdAt < lastAppliedRemoteTs.current) return prev;
        if (
          remote.createdAt === lastAppliedRemoteTs.current &&
          remote.eventId <= lastAppliedEventId.current
        ) {
          return prev;
        }
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        // Hydration guard (#2947): an edit made before the first remote blob
        // arrived is still in the debounce window. Cancel that publish, but
        // re-publish the merged store so the local edit is not silently dropped.
        const pending = managerRef.current?.getPendingStore() ?? null;
        managerRef.current?.cancelPendingPublish();
        const merged = mergeStores(prev, remote.store);
        if (!writeChannelNotifyPrefsStore(pubkey, relayUrl, merged))
          return prev;
        if (pending) managerRef.current?.publishPrefs(merged);
        return merged;
      };
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    void managerRef.current?.fetchRemotePrefs().then((remote) => {
      if (cancelled) return;
      if (remote) {
        setStore(applyRemote(remote));
        return;
      }
      const local = readChannelNotifyPrefsStore(pubkey, relayUrl);
      if (Object.keys(local.channels).length > 0) {
        managerRef.current?.publishPrefs(local);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, relayUrl, applyRemote]);

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void managerRef.current
      ?.subscribeToPrefs((remote) => {
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

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    const unsub = relayClient.subscribeToReconnects(() => {
      void managerRef.current?.fetchRemotePrefs().then((remote) => {
        if (cancelled) return;
        if (remote) setStore(applyRemote(remote));
        const pending = managerRef.current?.getPendingStore();
        if (pending) managerRef.current?.publishPrefs(pending);
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, relayUrl, applyRemote]);

  const timedMuteVersion = useTimedMuteVersion();

  // biome-ignore lint/correctness/useExhaustiveDependencies: store/legacyMutes are read through their .channels maps, which are the real deps; timedMuteVersion re-arms the timer after an expiry
  React.useEffect(() => {
    scheduleTimedMuteRefresh(nextTimedMuteExpiry(store, nowSeconds()));
  }, [store.channels, timedMuteVersion]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: same as above — the cache is keyed to the channel maps and the expiry version, not the store object identities
  const resolveChannel = React.useMemo(() => {
    // Per-channel memo so repeated lookups return the same object reference
    // (React.memo consumers compare by identity).
    const cache = new Map<string, ResolvedChannelNotifyState>();
    return (channelId: string): ResolvedChannelNotifyState => {
      const cached = cache.get(channelId);
      if (cached) return cached;
      const resolved = resolveChannelNotifyState(
        channelId,
        store,
        legacyMutes,
        nowSeconds(),
      );
      cache.set(channelId, resolved);
      return resolved;
    };
  }, [store.channels, legacyMutes.channels, timedMuteVersion]);

  const updateEntry = React.useCallback(
    (
      channelId: string,
      mutate: (entry: ChannelNotifyEntry) => ChannelNotifyEntry,
    ) => {
      if (!pubkey || !relayUrl) return;
      setStore((prev) => {
        const current = prev.channels[channelId] ?? { updatedAt: 0 };
        const nextEntry: ChannelNotifyEntry = {
          ...mutate(current),
          updatedAt: nowSeconds(),
        };
        const next = setChannelEntry(prev, channelId, nextEntry);
        if (next === prev) return prev;
        if (!writeChannelNotifyPrefsStore(pubkey, relayUrl, next)) return prev;
        managerRef.current?.publishPrefs(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const setChannelLevel = React.useCallback(
    (channelId: string, level: ChannelNotifyLevel) => {
      updateEntry(channelId, (entry) => {
        // Picking any level clears a running timed mute.
        const { muteUntil: _dropped, ...rest } = entry;
        return { ...rest, level };
      });
    },
    [updateEntry],
  );

  const muteChannelUntil = React.useCallback(
    (channelId: string, untilSeconds: number) => {
      updateEntry(channelId, (entry) => ({
        ...entry,
        muteUntil: untilSeconds,
      }));
    },
    [updateEntry],
  );

  const clearTimedMute = React.useCallback(
    (channelId: string) => {
      updateEntry(channelId, (entry) => {
        const { muteUntil: _dropped, ...rest } = entry;
        return rest;
      });
    },
    [updateEntry],
  );

  const setChannelAdvanced = React.useCallback(
    (channelId: string, patch: ChannelNotifyAdvancedPatch) => {
      updateEntry(channelId, (entry) => ({ ...entry, ...patch }));
    },
    [updateEntry],
  );

  return {
    prefsStore: store,
    resolveChannel,
    setChannelLevel,
    muteChannelUntil,
    clearTimedMute,
    setChannelAdvanced,
  };
}
