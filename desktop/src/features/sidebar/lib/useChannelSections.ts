import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  DEFAULT_STORE,
  readChannelSectionsStore,
  storageKey,
  writeChannelSectionsStore,
} from "./channelSectionsStorage";
import { ChannelSectionSyncManager } from "./channelSectionsSync";
import type { RemoteSections } from "./channelSectionsSync";
import { swapSectionOrder } from "./channelSectionsHelpers";

export type { ChannelSection } from "./channelSectionsStorage";

import type {
  ChannelSection,
  ChannelSectionStore,
} from "./channelSectionsStorage";

export function useChannelSections(
  pubkey: string | undefined,
  relayUrl?: string,
): {
  sections: ChannelSection[];
  assignments: Record<string, string>;
  createSection: (name: string, icon?: string) => ChannelSection | null;
  renameSection: (sectionId: string, newName: string, icon?: string) => void;
  deleteSection: (sectionId: string) => void;
  moveSectionUp: (sectionId: string) => void;
  moveSectionDown: (sectionId: string) => void;
  reorderSections: (orderedIds: string[]) => void;
  assignChannel: (channelId: string, sectionId: string) => void;
  unassignChannel: (channelId: string) => void;
} {
  const [store, setStore] = React.useState<ChannelSectionStore>(() => {
    if (!pubkey) {
      return DEFAULT_STORE;
    }
    return readChannelSectionsStore(pubkey, relayUrl);
  });

  const managerRef = React.useRef<ChannelSectionSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");

  React.useEffect(() => {
    if (!pubkey) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readChannelSectionsStore(pubkey, relayUrl));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ChannelSectionSyncManager(pubkey);
    return () => {
      managerRef.current?.destroy();
      managerRef.current = null;
    };
  }, [pubkey, relayUrl]);

  React.useEffect(() => {
    if (!pubkey) {
      return;
    }
    const key = storageKey(pubkey, relayUrl);
    const handler = (e: StorageEvent) => {
      if (e.key !== key) {
        return;
      }
      setStore(readChannelSectionsStore(pubkey, relayUrl));
    };
    window.addEventListener("storage", handler);
    return () => {
      window.removeEventListener("storage", handler);
    };
  }, [pubkey, relayUrl]);

  const applyRemote = React.useCallback(
    (
      remote: RemoteSections,
    ): ((prev: ChannelSectionStore) => ChannelSectionStore) => {
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
        if (!writeChannelSectionsStore(pubkey, remote.store, relayUrl))
          return prev;
        return remote.store;
      };
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey) return;
    let cancelled = false;
    void managerRef.current?.fetchRemoteSections().then((remote) => {
      if (cancelled) return;
      if (remote) {
        setStore(applyRemote(remote));
      } else {
        const local = readChannelSectionsStore(pubkey, relayUrl);
        if (local.sections.length > 0) {
          managerRef.current?.publishSections(local);
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
      ?.subscribeToSections((remote) => {
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
      void managerRef.current?.fetchRemoteSections().then((remote) => {
        if (cancelled) return;
        if (remote) {
          setStore(applyRemote(remote));
        }
        const pending = managerRef.current?.getPendingStore();
        if (pending) {
          managerRef.current?.publishSections(pending);
        }
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, applyRemote]);

  const sections = React.useMemo<ChannelSection[]>(
    () => store.sections.slice().sort((a, b) => a.order - b.order),
    [store.sections],
  );

  const createSection = React.useCallback(
    (name: string, icon?: string): ChannelSection | null => {
      if (!pubkey) return null;
      const prev = readChannelSectionsStore(pubkey, relayUrl);
      const maxOrder =
        prev.sections.length > 0
          ? Math.max(...prev.sections.map((s) => s.order))
          : -1;
      const section: ChannelSection = {
        id: crypto.randomUUID(),
        name,
        ...(icon ? { icon } : {}),
        order: maxOrder + 1,
      };
      setStore((current) => {
        const next: ChannelSectionStore = {
          ...current,
          sections: [...current.sections, section],
        };
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) return current;
        managerRef.current?.publishSections(next);
        return next;
      });
      return section;
    },
    [pubkey, relayUrl],
  );

  const renameSection = React.useCallback(
    (sectionId: string, newName: string, icon?: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const next: ChannelSectionStore = {
          ...prev,
          sections: prev.sections.map((s) =>
            s.id === sectionId
              ? {
                  id: s.id,
                  name: newName,
                  ...(icon ? { icon } : {}),
                  order: s.order,
                }
              : s,
          ),
        };
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) {
          return prev;
        }
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const deleteSection = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const assignments = { ...prev.assignments };
        for (const channelId of Object.keys(assignments)) {
          if (assignments[channelId] === sectionId) {
            delete assignments[channelId];
          }
        }
        const next: ChannelSectionStore = {
          ...prev,
          sections: prev.sections.filter((s) => s.id !== sectionId),
          assignments,
        };
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) {
          return prev;
        }
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const moveSectionUp = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) return;
      setStore((prev) => {
        const next = swapSectionOrder(prev, sectionId, "up");
        if (!next || !writeChannelSectionsStore(pubkey, next, relayUrl))
          return prev;
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const moveSectionDown = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) return;
      setStore((prev) => {
        const next = swapSectionOrder(prev, sectionId, "down");
        if (!next || !writeChannelSectionsStore(pubkey, next, relayUrl))
          return prev;
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const reorderSections = React.useCallback(
    (orderedIds: string[]) => {
      if (!pubkey) return;
      setStore((prev) => {
        const sections = prev.sections.map((s) => {
          const newOrder = orderedIds.indexOf(s.id);
          return newOrder === -1 ? s : { ...s, order: newOrder };
        });
        const next: ChannelSectionStore = { ...prev, sections };
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) return prev;
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const assignChannel = React.useCallback(
    (channelId: string, sectionId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const next: ChannelSectionStore = {
          ...prev,
          assignments: { ...prev.assignments, [channelId]: sectionId },
        };
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) {
          return prev;
        }
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const unassignChannel = React.useCallback(
    (channelId: string) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const assignments = { ...prev.assignments };
        delete assignments[channelId];
        const next: ChannelSectionStore = { ...prev, assignments };
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) {
          return prev;
        }
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  return {
    sections,
    assignments: store.assignments,
    createSection,
    renameSection,
    deleteSection,
    moveSectionUp,
    moveSectionDown,
    reorderSections,
    assignChannel,
    unassignChannel,
  };
}
