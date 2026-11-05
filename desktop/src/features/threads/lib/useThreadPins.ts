import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  EMPTY_THREAD_PIN_STORE,
  mergeThreadPinStores,
  pinnedRoots,
  readThreadPinStore,
  threadPinStoresEqual,
  togglePin,
  writeThreadPinStore,
  type ThreadPinStore,
} from "./threadPins";
import { ThreadPinsSyncManager, type RemoteThreadPins } from "./threadPinsSync";

/**
 * The current identity's pinned thread roots, synced across devices, plus a
 * toggle. Toggling is unavailable (undefined) until the relay and identity
 * resolve.
 */
export function useThreadPins(
  relayUrl: string | null,
  pubkey: string | undefined,
): {
  pinned: ReadonlySet<string>;
  toggle: ((rootId: string) => void) | undefined;
} {
  const [store, setStore] = React.useState<ThreadPinStore>(
    EMPTY_THREAD_PIN_STORE,
  );
  const storeRef = React.useRef(store);
  storeRef.current = store;
  const managerRef = React.useRef<ThreadPinsSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    if (!relayUrl || !pubkey) {
      setStore(EMPTY_THREAD_PIN_STORE);
      return;
    }
    setStore(readThreadPinStore(relayUrl, pubkey));
    managerRef.current = new ThreadPinsSyncManager(pubkey, relayUrl);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

  const applyRemote = React.useCallback(
    (remote: RemoteThreadPins) =>
      (current: ThreadPinStore): ThreadPinStore => {
        if (!relayUrl || !pubkey) return current;
        if (remote.createdAt < lastAppliedRemoteTs.current) return current;
        if (
          remote.createdAt === lastAppliedRemoteTs.current &&
          remote.eventId <= lastAppliedEventId.current
        ) {
          return current;
        }
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        managerRef.current?.cancelPendingPublish();
        const merged = mergeThreadPinStores(current, remote.store);
        writeThreadPinStore(relayUrl, pubkey, merged);
        if (!threadPinStoresEqual(merged, remote.store)) {
          managerRef.current?.publishPins(merged);
        }
        return merged;
      },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!relayUrl || !pubkey) return;
    let cancelled = false;
    void managerRef.current
      ?.bootstrap(readThreadPinStore(relayUrl, pubkey))
      .then((result) => {
        if (!cancelled && result.action === "apply-remote") {
          setStore(applyRemote(result.data));
        }
      });
    return () => {
      cancelled = true;
    };
  }, [applyRemote, pubkey, relayUrl]);

  React.useEffect(() => {
    if (!relayUrl || !pubkey) return;
    let unsubscribe: (() => Promise<void>) | null = null;
    let cancelled = false;
    void managerRef.current
      ?.subscribe((remote) => {
        if (!cancelled) setStore(applyRemote(remote));
      })
      .then((dispose) => {
        if (cancelled) {
          void dispose();
        } else {
          unsubscribe = dispose;
        }
      });
    return () => {
      cancelled = true;
      if (unsubscribe) void unsubscribe();
    };
  }, [applyRemote, pubkey, relayUrl]);

  React.useEffect(() => {
    if (!relayUrl || !pubkey) return;
    let cancelled = false;
    const unsubscribe = relayClient.subscribeToReconnects(() => {
      void managerRef.current?.fetchRemotePins().then((result) => {
        if (cancelled) return;
        if (result.status === "found") setStore(applyRemote(result.data));
        const pending = managerRef.current?.getPendingStore();
        if (pending) managerRef.current?.publishPins(pending);
      });
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [applyRemote, pubkey, relayUrl]);

  const toggle = React.useMemo(() => {
    if (!relayUrl || !pubkey) return undefined;
    return (rootId: string) => {
      // Computed from the ref, not inside a state updater, so the write and
      // publish run exactly once even under StrictMode's double-invoke.
      const next = togglePin(storeRef.current, rootId, Date.now());
      storeRef.current = next;
      setStore(next);
      writeThreadPinStore(relayUrl, pubkey, next);
      managerRef.current?.publishPins(next);
    };
  }, [pubkey, relayUrl]);

  const pinned = React.useMemo(() => pinnedRoots(store), [store]);
  return { pinned, toggle };
}
