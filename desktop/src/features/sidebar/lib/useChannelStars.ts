import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  boundStarStore,
  clearChannelStarsOutbox,
  DEFAULT_STORE,
  isStarsStoreSubsumedBy,
  mergeStores,
  readChannelStarsOutboxWithMeta,
  readChannelStarsStore,
  reclaimSubsumedStarsOutbox,
  starredChannelIdsFromStore,
  storageKey,
  writeChannelStarsStore,
  type ChannelStarEntry,
  type ChannelStarStore,
} from "./channelStarsStorage";
import { ChannelStarSyncManager } from "./channelStarsSync";
import type { RemoteStars } from "./channelStarsSync";

// Reconciliation cadence. Steady interval re-fetches the head on a healthy
// socket so a silently-lost publish converges without waiting for a reconnect
// that may never fire; the retry window backs off while the fetch keeps failing.
const RECONCILE_STEADY_MS = 60_000;
const RECONCILE_RETRY_BASE_MS = 3_000;
const RECONCILE_RETRY_MAX_MS = 60_000;

export function useChannelStars(
  pubkey: string | undefined,
  relayUrl?: string,
): {
  starredChannelIds: Set<string>;
  starChannel: (channelId: string) => void;
  unstarChannel: (channelId: string) => void;
} {
  const [store, setStore] = React.useState<ChannelStarStore>(() => {
    if (!pubkey) {
      return DEFAULT_STORE;
    }
    return readChannelStarsStore(pubkey, relayUrl);
  });

  const managerRef = React.useRef<ChannelStarSyncManager | null>(null);

  React.useEffect(() => {
    if (!pubkey || !relayUrl) {
      setStore(DEFAULT_STORE);
      return;
    }
    setStore(readChannelStarsStore(pubkey, relayUrl));
    managerRef.current = new ChannelStarSyncManager(pubkey, relayUrl);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

  // Cross-window sync: another window/tab wrote the shared store. Ingest it into
  // the high-water and max-merge it into this window's state, so a click that
  // follows sees the peer's revs/timestamps and no window's edit is clobbered.
  React.useEffect(() => {
    if (!pubkey) {
      return;
    }
    const key = storageKey(pubkey, relayUrl);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) {
        return;
      }
      const incoming = readChannelStarsStore(pubkey, relayUrl);
      managerRef.current?.observe(incoming);
      setStore((prev) => mergeStores(prev, incoming));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey, relayUrl]);

  // Every remote payload is observed by the manager before it reaches here
  // (fetch/subscribe paths call observe() internally; the storage handler
  // observes above), so this is a pure max-merge with no ordering or ownership
  // overlay — "later" lives in the (updatedAt, rev) tuple.
  const applyRemote = React.useCallback(
    (remote: RemoteStars): ((prev: ChannelStarStore) => ChannelStarStore) => {
      return (prev) => {
        if (!pubkey) return prev;
        // Read-merge-write folds the head into whatever a peer window has
        // persisted since; use the returned store so a concurrent click there
        // is carried into this window's state rather than lost.
        const persisted = writeChannelStarsStore(
          pubkey,
          mergeStores(prev, remote.store),
          relayUrl,
        );
        if (!persisted) return prev;
        return persisted;
      };
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    const local = readChannelStarsStore(pubkey, relayUrl);
    void managerRef.current?.bootstrap(local).then((result) => {
      if (cancelled) return;
      if (result.action === "apply-remote") {
        setStore(applyRemote(result.data));
      }
      // Resume every window's edit persisted to the durable outbox before a
      // prior quit/community-switch so a click made <2s before teardown still
      // syncs. Merges all records (order-independent), then publishes. Replay
      // runs BEFORE reclamation so a same-second record the head appears to
      // supersede is consumed into pending here and can never be GC'd out.
      const outboxMeta = readChannelStarsOutboxWithMeta(pubkey, relayUrl);
      if (outboxMeta) {
        // Skip the publish only when the fetched head already subsumes the
        // fold — a lingering never-deleted legacy key or head-subsumed record
        // would otherwise re-drive an identical publish on every boot. A `hold`
        // (no head fetched) cannot prove redundancy, so always publishes.
        // Merge LWW keeps this correctness-safe either way; the gate removes
        // noise only.
        const subsumed =
          result.action === "apply-remote" &&
          isStarsStoreSubsumedBy(
            outboxMeta.store,
            result.data.store,
            outboxMeta.preservedKey ?? undefined,
          );
        if (!subsumed) {
          // Forward the preserved key so the clicked channel's capacity
          // reservation survives remount and restart. The key is selected
          // from all records (own and foreign) by max queuedAt, so it is
          // recovered even when the prior window's record is now foreign
          // after a quit (Kalvin P3).
          managerRef.current?.publishStars(
            outboxMeta.store,
            outboxMeta.preservedKey,
          );
        }
      } else {
        clearChannelStarsOutbox(pubkey, relayUrl);
      }
      if (result.action === "apply-remote") {
        // Head fetch succeeded: reclaim any foreign window's write-once outbox
        // key the head already subsumes (a peer that published then quit).
        // Gated on the fetched head; records are immutable so no recheck is
        // needed and a live peer's unpublished edit (under a different key) is
        // never destroyed. A `hold` (absent/failed head) reclaims nothing.
        reclaimSubsumedStarsOutbox(pubkey, relayUrl, result.data.store);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, relayUrl, applyRemote]);

  // Reconciliation loop: a single scheduler that both retries a failed bootstrap
  // fetch with bounded backoff and periodically re-fetches the head, so a
  // silently-lost publish converges within the steady cadence without waiting
  // for a reconnect a healthy socket never fires. Also refreshes on visibility.
  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    let timer: number | null = null;
    let delayMs = RECONCILE_RETRY_BASE_MS;

    const schedule = (ms: number) => {
      if (cancelled) return;
      if (timer !== null) window.clearTimeout(timer);
      timer = window.setTimeout(tick, ms);
    };

    const tick = () => {
      void managerRef.current?.fetchRemoteStars().then((result) => {
        if (cancelled) return;
        if (result.status === "found") {
          // max-merge folds the head into state without dropping a pending
          // edit (that edit is in prev and owned by the manager's retry lane).
          setStore(applyRemote(result.data));
          delayMs = RECONCILE_STEADY_MS; // relay answered → steady cadence
        } else if (result.status === "absent") {
          delayMs = RECONCILE_STEADY_MS; // answered (no blob) → steady cadence
        } else {
          delayMs = Math.min(delayMs * 2, RECONCILE_RETRY_MAX_MS); // failed → back off
        }
        schedule(delayMs);
      });
    };

    const onVisible = () => {
      if (document.visibilityState === "visible") {
        delayMs = RECONCILE_RETRY_BASE_MS;
        tick();
      }
    };
    document.addEventListener("visibilitychange", onVisible);
    schedule(delayMs);

    return () => {
      cancelled = true;
      if (timer !== null) window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [pubkey, relayUrl, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: relayUrl is intentional — rebinds subscription when the active relay changes even though it is not used inside the effect body directly (the manager via managerRef.current carries it)
  React.useEffect(() => {
    if (!pubkey) return;
    let unsub: (() => Promise<void>) | null = null;
    let cancelled = false;
    void managerRef.current
      ?.subscribeToStars((remote) => {
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
      void managerRef.current?.fetchRemoteStars().then((result) => {
        if (cancelled) return;
        if (result.status === "found") {
          setStore(applyRemote(result.data));
        }
        // Re-drive the existing generation rather than opening a new one so
        // pendingPreservedKey is not reset — a new publish() call would clear
        // the key and let a 501-entry pre-publish merge evict the clicked
        // channel (Kalvin P3).
        managerRef.current?.retryReconnectStarsPublish();
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, relayUrl, applyRemote]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: store.channels is the relevant dep — the outer store identity can change without channels changing (e.g., on reconnect writes)
  const starredChannelIds = React.useMemo(
    () => starredChannelIdsFromStore(store),
    [store.channels],
  );

  const setStarState = React.useCallback(
    (channelId: string, starred: boolean) => {
      if (!pubkey) return;
      const now = Math.floor(Date.now() / 1000);
      setStore((prev) => {
        const manager = managerRef.current;
        const localEntry = prev.channels[channelId];
        // Logical-monotonic mint: never regress below any (updatedAt, rev) this
        // replica has observed for the channel (local entry OR manager
        // high-water), so the click strictly dominates observed state in both
        // merge keys — it can never lose to state it has already seen.
        const updatedAt = Math.max(
          now,
          localEntry?.updatedAt ?? 0,
          manager?.maxUpdatedAtSeen(channelId) ?? 0,
        );
        const rev =
          Math.max(localEntry?.rev ?? 0, manager?.maxRevSeen(channelId) ?? 0) +
          1;
        const entry: ChannelStarEntry = { starred, updatedAt, rev };
        const next = boundStarStore(
          {
            version: 1,
            channels: { ...prev.channels, [channelId]: entry },
          },
          channelId,
        );
        // Read-merge-write: fold this click into any concurrent peer-window
        // click already persisted under the shared key, then thread the merged
        // store into both React state and the publish so neither window's edit
        // is dropped (Carl prong b). Preserve the clicked channel through the
        // re-bound so a same-second mutation is not evicted at capacity.
        const persisted = writeChannelStarsStore(
          pubkey,
          next,
          relayUrl,
          channelId,
        );
        if (!persisted) return prev;
        manager?.publishStars(persisted, channelId);
        return persisted;
      });
    },
    [pubkey, relayUrl],
  );

  const starChannel = React.useCallback(
    (channelId: string) => setStarState(channelId, true),
    [setStarState],
  );
  const unstarChannel = React.useCallback(
    (channelId: string) => setStarState(channelId, false),
    [setStarState],
  );

  return {
    starredChannelIds,
    starChannel,
    unstarChannel,
  };
}
