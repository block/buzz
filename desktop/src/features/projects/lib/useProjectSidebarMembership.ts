import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE,
  mergeProjectSidebarMembershipStores,
  PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
  projectSidebarMembershipStoresEqual,
  projectSidebarMembershipStorageKey,
  readProjectSidebarMembershipStore,
  selectedProjectAddressesFromStore,
  writeProjectSidebarMembershipStore,
  type ProjectSidebarMembershipStore,
} from "./projectSidebarMembership";
import {
  ProjectSidebarMembershipSyncManager,
  type RemoteProjectSidebarMembership,
} from "./projectSidebarMembershipSync";

export function useProjectSidebarMembership(
  relayUrl: string | null,
  pubkey: string | undefined,
): string[] {
  const [store, setStore] = React.useState<ProjectSidebarMembershipStore>(
    EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE,
  );
  const managerRef = React.useRef<ProjectSidebarMembershipSyncManager | null>(
    null,
  );
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!relayUrl || !pubkey) {
      setStore(EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readProjectSidebarMembershipStore(relayUrl, pubkey));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ProjectSidebarMembershipSyncManager(
      pubkey,
      relayUrl,
    );
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!relayUrl || !pubkey) return;
    const refreshAndPublish = () => {
      const next = readProjectSidebarMembershipStore(relayUrl, pubkey);
      setStore(next);
      managerRef.current?.publishMembership(next);
    };
    globalThis.addEventListener(
      PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
      refreshAndPublish,
    );
    return () => {
      globalThis.removeEventListener(
        PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
        refreshAndPublish,
      );
    };
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!relayUrl || !pubkey) return;
    const key = projectSidebarMembershipStorageKey(relayUrl, pubkey);
    const handleStorage = (event: StorageEvent) => {
      if (event.key === key) {
        setStore(readProjectSidebarMembershipStore(relayUrl, pubkey));
      }
    };
    globalThis.addEventListener("storage", handleStorage);
    return () => globalThis.removeEventListener("storage", handleStorage);
  }, [pubkey, relayUrl]);

  const applyRemote = React.useCallback(
    (
      remote: RemoteProjectSidebarMembership,
    ): ((
      current: ProjectSidebarMembershipStore,
    ) => ProjectSidebarMembershipStore) =>
      (current) => {
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
        const merged = mergeProjectSidebarMembershipStores(
          current,
          remote.store,
        );
        // The write records merged as the authoritative in-session state even
        // when the localStorage mirror fails, so the merge always applies.
        writeProjectSidebarMembershipStore(relayUrl, pubkey, merged, false);
        if (!projectSidebarMembershipStoresEqual(merged, remote.store)) {
          managerRef.current?.publishMembership(merged);
        }
        return merged;
      },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!relayUrl || !pubkey) return;
    let cancelled = false;
    const local = readProjectSidebarMembershipStore(relayUrl, pubkey);
    void managerRef.current?.bootstrap(local).then((result) => {
      if (cancelled) return;
      if (result.action === "apply-remote") {
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
      void managerRef.current?.fetchRemoteMembership().then((result) => {
        if (cancelled) return;
        if (result.status === "found") {
          setStore(applyRemote(result.data));
        }
        const pending = managerRef.current?.getPendingStore();
        if (pending) managerRef.current?.publishMembership(pending);
      });
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [applyRemote, pubkey, relayUrl]);

  return React.useMemo(() => selectedProjectAddressesFromStore(store), [store]);
}
