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
import {
  isChannelSectionsAllowlistReady,
  scopeChannelSectionsToKnownChannels,
  swapSectionOrder,
} from "./channelSectionsHelpers";

export type { ChannelSection } from "./channelSectionsStorage";

import type {
  ChannelSection,
  ChannelSectionStore,
} from "./channelSectionsStorage";

function readScopedStore(
  pubkey: string | undefined,
  relayUrl: string | undefined,
): ChannelSectionStore {
  if (!pubkey) {
    return DEFAULT_STORE;
  }
  return readChannelSectionsStore(pubkey, relayUrl);
}

export function useChannelSections(
  pubkey: string | undefined,
  relayUrl?: string,
  knownChannelIds?: ReadonlySet<string> | null,
  /**
   * When true, an empty allowlist means "community has no channels" and
   * foreign assignments are stripped. When false, the allowlist is still
   * loading and scoping is deferred. When omitted, a non-empty allowlist is
   * treated as ready (unit-test / legacy callers).
   */
  channelsReady?: boolean,
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
  const scopeKey = `${pubkey ?? ""}::${relayUrl ?? ""}`;
  const [store, setStore] = React.useState<ChannelSectionStore>(() =>
    readScopedStore(pubkey, relayUrl),
  );

  // Scope fence (#7207): bump generation synchronously so in-flight writers from
  // the previous community cannot persist/publish under the new relayUrl. Do
  // NOT call setState during render — that desyncs later hooks in AppSidebar
  // (invalid hook call / "Should have a queue"). Hydrate from the new scope in
  // layout; until then derive a display store from localStorage for this scope.
  const scopeRef = React.useRef({ pubkey, relayUrl });
  const scopeGenerationRef = React.useRef(0);
  const storeScopeKeyRef = React.useRef(scopeKey);
  const managerRef = React.useRef<ChannelSectionSyncManager | null>(null);
  const lastAppliedRemoteTs = React.useRef(0);
  const lastAppliedEventId = React.useRef("");
  const knownChannelIdsRef = React.useRef(knownChannelIds);
  knownChannelIdsRef.current = knownChannelIds;
  const channelsReadyRef = React.useRef(channelsReady);
  channelsReadyRef.current = channelsReady;
  const allowlistReady = isChannelSectionsAllowlistReady(
    knownChannelIds,
    channelsReady,
  );

  if (
    scopeRef.current.pubkey !== pubkey ||
    scopeRef.current.relayUrl !== relayUrl
  ) {
    scopeRef.current = { pubkey, relayUrl };
    scopeGenerationRef.current += 1;
  }

  React.useLayoutEffect(() => {
    setStore(readScopedStore(pubkey, relayUrl));
    storeScopeKeyRef.current = scopeKey;
    lastAppliedRemoteTs.current = 0;
    lastAppliedEventId.current = "";
  }, [scopeKey, pubkey, relayUrl]);

  const effectiveStore =
    storeScopeKeyRef.current === scopeKey
      ? store
      : readScopedStore(pubkey, relayUrl);

  const persistAndPublish = React.useCallback(
    (
      next: ChannelSectionStore,
      generation: number,
    ): ChannelSectionStore | null => {
      if (!pubkey) {
        return null;
      }
      if (scopeGenerationRef.current !== generation) {
        return null;
      }
      if (
        scopeRef.current.pubkey !== pubkey ||
        scopeRef.current.relayUrl !== relayUrl
      ) {
        return null;
      }
      const scoped = scopeChannelSectionsToKnownChannels(
        boundChannelSectionsStore(next),
        knownChannelIdsRef.current,
        channelsReadyRef.current,
      );
      if (!writeChannelSectionsStore(pubkey, scoped, relayUrl)) {
        return null;
      }
      storeScopeKeyRef.current = `${pubkey}::${relayUrl ?? ""}`;
      managerRef.current?.publishSections(scoped);
      return scoped;
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey || !relayUrl) {
      lastAppliedRemoteTs.current = 0;
      lastAppliedEventId.current = "";
      managerRef.current?.destroy();
      managerRef.current = null;
      return;
    }
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
      const generation = scopeGenerationRef.current;
      return (prev) => {
        if (!pubkey) return prev;
        if (scopeGenerationRef.current !== generation) return prev;
        if (remote.createdAt < lastAppliedRemoteTs.current) return prev;
        if (
          remote.createdAt === lastAppliedRemoteTs.current &&
          remote.eventId <= lastAppliedEventId.current
        )
          return prev;
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        managerRef.current?.cancelPendingPublish();
        const scoped = scopeChannelSectionsToKnownChannels(
          remote.store,
          knownChannelIdsRef.current,
          channelsReadyRef.current,
        );
        if (!writeChannelSectionsStore(pubkey, scoped, relayUrl)) return prev;
        storeScopeKeyRef.current = `${pubkey}::${relayUrl ?? ""}`;
        return scoped;
      };
    },
    [pubkey, relayUrl],
  );

  // Defer bootstrap (including first-sync seed-publish) until the channel
  // allowlist is ready so a polluted local blob cannot seed-publish foreign
  // channel ids while channels are still loading (#7207).
  React.useEffect(() => {
    if (!pubkey || !relayUrl || !allowlistReady) return;
    let cancelled = false;
    const local = scopeChannelSectionsToKnownChannels(
      readChannelSectionsStore(pubkey, relayUrl),
      knownChannelIdsRef.current,
      true,
    );
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
  }, [pubkey, relayUrl, applyRemote, allowlistReady]);

  // When the allowlist becomes ready (or the known channel set changes), heal
  // any foreign assignments / orphan section objects already sitting in memory
  // or localStorage, and publish the cleaned blob so a previously polluted A
  // does not keep serving B's layout.
  React.useEffect(() => {
    if (!pubkey || !allowlistReady) return;
    const generation = scopeGenerationRef.current;
    setStore((prev) => {
      const scoped = scopeChannelSectionsToKnownChannels(
        prev,
        knownChannelIds,
        true,
      );
      if (scoped === prev) {
        return prev;
      }
      return persistAndPublish(scoped, generation) ?? prev;
    });
  }, [pubkey, allowlistReady, knownChannelIds, persistAndPublish]);

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
          const scoped = scopeChannelSectionsToKnownChannels(
            pending,
            knownChannelIdsRef.current,
            channelsReadyRef.current,
          );
          managerRef.current?.publishSections(scoped);
        }
      });
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [pubkey, applyRemote]);

  const displayStore = React.useMemo(
    () =>
      scopeChannelSectionsToKnownChannels(
        effectiveStore,
        knownChannelIds,
        channelsReady,
      ),
    [effectiveStore, knownChannelIds, channelsReady],
  );

  const sections = React.useMemo<ChannelSection[]>(
    () => displayStore.sections.slice().sort((a, b) => a.order - b.order),
    [displayStore.sections],
  );

  const assignments = displayStore.assignments;

  const createSection = React.useCallback(
    (name: string, icon?: string): ChannelSection | null => {
      if (!pubkey) return null;
      const generation = scopeGenerationRef.current;
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
      let created: ChannelSection | null = section;
      setStore((current) => {
        const next = {
          ...current,
          sections: [...current.sections, section],
        };
        const persisted = persistAndPublish(next, generation);
        if (!persisted) {
          created = null;
          return current;
        }
        return persisted;
      });
      return created;
    },
    [pubkey, relayUrl, persistAndPublish],
  );

  const renameSection = React.useCallback(
    (sectionId: string, newName: string, icon?: string) => {
      if (!pubkey) {
        return;
      }
      const generation = scopeGenerationRef.current;
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
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  const deleteSection = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) {
        return;
      }
      const generation = scopeGenerationRef.current;
      setStore((prev) => {
        const nextAssignments = { ...prev.assignments };
        for (const channelId of Object.keys(nextAssignments)) {
          if (nextAssignments[channelId] === sectionId) {
            delete nextAssignments[channelId];
          }
        }
        const next: ChannelSectionStore = {
          ...prev,
          sections: prev.sections.filter((s) => s.id !== sectionId),
          assignments: nextAssignments,
        };
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  const moveSectionUp = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) return;
      const generation = scopeGenerationRef.current;
      setStore((prev) => {
        const next = swapSectionOrder(prev, sectionId, "up");
        if (!next) return prev;
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  const moveSectionDown = React.useCallback(
    (sectionId: string) => {
      if (!pubkey) return;
      const generation = scopeGenerationRef.current;
      setStore((prev) => {
        const next = swapSectionOrder(prev, sectionId, "down");
        if (!next) return prev;
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  const reorderSections = React.useCallback(
    (orderedIds: string[]) => {
      if (!pubkey) return;
      const generation = scopeGenerationRef.current;
      setStore((prev) => {
        const nextSections = prev.sections.map((s) => {
          const newOrder = orderedIds.indexOf(s.id);
          return newOrder === -1 ? s : { ...s, order: newOrder };
        });
        const next: ChannelSectionStore = { ...prev, sections: nextSections };
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  const assignChannel = React.useCallback(
    (channelId: string, sectionId: string) => {
      if (!pubkey) {
        return;
      }
      const generation = scopeGenerationRef.current;
      setStore((prev) => {
        const nextAssignments = { ...prev.assignments };
        delete nextAssignments[channelId];
        nextAssignments[channelId] = sectionId;
        const next = {
          ...prev,
          assignments: nextAssignments,
        };
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  const unassignChannel = React.useCallback(
    (channelId: string) => {
      if (!pubkey) {
        return;
      }
      const generation = scopeGenerationRef.current;
      setStore((prev) => {
        const nextAssignments = { ...prev.assignments };
        delete nextAssignments[channelId];
        const next: ChannelSectionStore = {
          ...prev,
          assignments: nextAssignments,
        };
        return persistAndPublish(next, generation) ?? prev;
      });
    },
    [pubkey, persistAndPublish],
  );

  return {
    sections,
    assignments,
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
