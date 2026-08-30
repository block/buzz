import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  boundChannelSectionsStore,
  DEFAULT_STORE,
  readChannelSectionsStore,
  storageKey,
  writeChannelSectionsStore,
} from "./channelSectionsStorage";
import { ChannelSectionSyncManager } from "./channelSectionsSync";
import type { RemoteSections } from "./channelSectionsSync";
import { swapSectionOrder } from "./channelSectionsHelpers";
import {
  applyAutomaticSmartSectionAssignments,
  assignChannelsToSection,
  manuallyUnassignChannel,
  type SmartSectionChannel,
} from "./smartChannelSections";

const EMPTY_SMART_SECTION_CHANNELS: readonly SmartSectionChannel[] = [];

export type {
  ChannelSection,
  SmartSectionRule,
} from "./channelSectionsStorage";

import type {
  ChannelSection,
  ChannelSectionStore,
  SmartSectionRule,
} from "./channelSectionsStorage";

export function useChannelSections(
  pubkey: string | undefined,
  relayUrl?: string,
  channels: readonly SmartSectionChannel[] = EMPTY_SMART_SECTION_CHANNELS,
): {
  sections: ChannelSection[];
  assignments: Record<string, string>;
  createSection: (
    name: string,
    icon?: string,
    smartRule?: SmartSectionRule,
  ) => ChannelSection | null;
  renameSection: (
    sectionId: string,
    newName: string,
    icon?: string,
    smartRule?: SmartSectionRule,
  ) => void;
  deleteSection: (sectionId: string) => void;
  moveSectionUp: (sectionId: string) => void;
  moveSectionDown: (sectionId: string) => void;
  reorderSections: (orderedIds: string[]) => void;
  assignChannel: (channelId: string, sectionId: string) => void;
  assignChannels: (channelIds: string[], sectionId: string) => void;
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
    if (!pubkey || !relayUrl) {
      setStore(DEFAULT_STORE);
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      return;
    }
    setStore(readChannelSectionsStore(pubkey, relayUrl));
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
    managerRef.current = new ChannelSectionSyncManager(pubkey, relayUrl);
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
    if (!pubkey || !relayUrl) return;
    let cancelled = false;
    const local = readChannelSectionsStore(pubkey, relayUrl);
    void managerRef.current?.bootstrap(local).then((result) => {
      if (cancelled) return;
      if (result.action === "apply-remote") {
        setStore(applyRemote(result.data));
      }
      // "hold": seed already performed by bootstrap (if first-sync), or
      // blocked (failed fetch / prior watermark). Hook does nothing.
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
      void managerRef.current?.fetchRemoteSections().then((result) => {
        if (cancelled) return;
        if (result.status === "found") {
          setStore(applyRemote(result.data));
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
    (
      name: string,
      icon?: string,
      smartRule?: SmartSectionRule,
    ): ChannelSection | null => {
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
        ...(smartRule ? { smartRule } : {}),
      };
      setStore((current) => {
        const next = boundChannelSectionsStore({
          ...current,
          sections: [...current.sections, section],
        });
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) return current;
        managerRef.current?.publishSections(next);
        return next;
      });
      return section;
    },
    [pubkey, relayUrl],
  );

  const renameSection = React.useCallback(
    (
      sectionId: string,
      newName: string,
      icon?: string,
      smartRule?: SmartSectionRule,
    ) => {
      if (!pubkey) {
        return;
      }
      setStore((prev) => {
        const next: ChannelSectionStore = {
          ...prev,
          sections: prev.sections.map((s) =>
            s.id === sectionId
              ? {
                  ...s,
                  name: newName,
                  ...(icon ? { icon } : {}),
                  ...(icon ? {} : { icon: undefined }),
                  ...(smartRule ? { smartRule } : {}),
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
        const next = boundChannelSectionsStore(
          assignChannelsToSection(prev, [channelId], sectionId),
        );
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) {
          return prev;
        }
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const assignChannels = React.useCallback(
    (channelIds: string[], sectionId: string) => {
      if (!pubkey || channelIds.length === 0) return;
      setStore((prev) => {
        const next = boundChannelSectionsStore(
          assignChannelsToSection(prev, channelIds, sectionId),
        );
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) return prev;
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
        const next = boundChannelSectionsStore(
          manuallyUnassignChannel(prev, channelId),
        );
        if (!writeChannelSectionsStore(pubkey, next, relayUrl)) {
          return prev;
        }
        managerRef.current?.publishSections(next);
        return next;
      });
    },
    [pubkey, relayUrl],
  );

  const smartRulesKey = JSON.stringify(
    store.sections.flatMap((section) =>
      section.smartRule
        ? [[section.id, section.order, section.smartRule] as const]
        : [],
    ),
  );

  React.useEffect(() => {
    if (!pubkey || channels.length === 0 || smartRulesKey === "[]") return;
    setStore((prev) => {
      const matched = applyAutomaticSmartSectionAssignments(prev, channels);
      if (matched === prev) return prev;
      const next = boundChannelSectionsStore(matched);
      if (!writeChannelSectionsStore(pubkey, next, relayUrl)) return prev;
      managerRef.current?.publishSections(next);
      return next;
    });
  }, [pubkey, relayUrl, channels, smartRulesKey]);

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
    assignChannels,
    unassignChannel,
  };
}
