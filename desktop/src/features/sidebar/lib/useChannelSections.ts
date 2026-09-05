import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  boundChannelSectionsStore,
  clearChannelSectionsOutbox,
  DEFAULT_STORE,
  markChannelSectionsLegacyConsumed,
  readChannelSectionsOutbox,
  readChannelSectionsStore,
  reclaimSupersededSectionsOutbox,
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

// Reconciliation cadence (fix 1). Steady interval re-fetches the head on a
// healthy socket so divergence self-heals without a reconnect; the retry
// window backs off from base to max while the fetch keeps failing.
const RECONCILE_STEADY_MS = 60_000;
const RECONCILE_RETRY_BASE_MS = 3_000;
const RECONCILE_RETRY_MAX_MS = 60_000;

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
      // A peer window wrote to the shared cache. Honour the same pending-aware
      // guard that relay arrivals use in `applyRemote`: if a local whole-blob
      // edit is in flight the manager owns convergence (its debounced
      // publish-or-adopt is the single arbitration point). Applying a peer cache
      // write unconditionally would clobber the optimistic edit and, on the next
      // keystroke, strand the outbox record for the old intent (Carl P1).
      // When nothing is pending the peer write applies immediately, preserving
      // cross-window state mirroring for the idle case.
      if (managerRef.current?.hasPendingEdit()) return;
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
        // A pending local edit owns convergence: its debounced publish
        // re-checks the head and either wins (publish) or loses (adopt, which
        // routes back through onRemoteAdopted with pending already cleared).
        // Never let a passive remote arrival clobber the optimistic edit or
        // strand its durable outbox — that is the one-convergence-mechanism
        // invariant. The adopt path clears pending before calling us, so this
        // guard is false there and the winning remote still writes through.
        if (managerRef.current?.hasPendingEdit()) return prev;
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
        lastAppliedRemoteTs.current = remote.createdAt;
        lastAppliedEventId.current = remote.eventId;
        if (!writeChannelSectionsStore(pubkey, remote.store, relayUrl))
          return prev;
        return remote.store;
      };
    },
    [pubkey, relayUrl],
  );

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    const manager = managerRef.current;
    if (!manager) return;
    // When a local edit loses whole-blob LWW (pre-publish head is newer), the
    // manager adopts the winning remote store. Write it through to React state
    // + localStorage so the UI and relay never diverge; applyRemote also
    // advances the applied-ts guard.
    manager.setOnRemoteAdopted((remote) => {
      setStore(applyRemote(remote));
    });
  }, [pubkey, relayUrl, applyRemote]);

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
      // blocked (failed fetch / prior watermark). The reconciliation effect
      // below retries a failed fetch; here we only resume any edit that was
      // persisted to the durable outbox before a prior quit/community-switch.
      // Replay runs BEFORE reclamation so a same-second record the head appears
      // to supersede is consumed into pending here and can never be GC'd out.
      const outbox = readChannelSectionsOutbox(pubkey, relayUrl);
      if (outbox) {
        // When bootstrap fetched and applied a remote head, suppress replay
        // only for an outbox edit that is STRICTLY older than that head:
        // `queuedAt < appliedHead.createdAt`. Equality replays — a same-second
        // edit was in-flight with the head and must not be silently swallowed —
        // and this keeps the guard aligned with reclamation's strict `<` so an
        // equal-second edit is neither stranded on disk nor double-suppressed.
        // A `hold` (absent/failed fetch) carries no confirmed applied head so
        // we always replay regardless of queuedAt. Note: `queuedAt` and relay
        // `created_at` are independent device wall clocks; a slow-clocked
        // device can produce a queuedAt that appears to predate a head it never
        // saw — this is an accepted residual of the LWW max-merge strategy the
        // feature already relies on.
        const shouldReplay =
          result.action !== "apply-remote" ||
          outbox.queuedAt >= result.data.createdAt;
        if (shouldReplay) {
          // publishSections synchronously copies the intent into this window's
          // own v2 key and returns whether that transfer is durable. Mark the
          // legacy blob consumed ONLY when it is: a `setItem` failure (quota)
          // returns false, so the marker is not written and the legacy record
          // stays replayable on a later boot rather than being silently
          // suppressed (Thufir pass-3 finding). A crash between a durable
          // transfer and the marker write replays the legacy blob once more, a
          // crash after resumes it from the v2 key. The marker is what stops the
          // never-deleted legacy key republishing above the head every boot
          // (Thufir pass-2 resurrection finding).
          const durable = managerRef.current?.publishSections(
            outbox.store,
            true,
            outbox.queuedAt,
          );
          if (durable && outbox.legacyRawToConsume !== null) {
            markChannelSectionsLegacyConsumed(
              pubkey,
              relayUrl,
              outbox.legacyRawToConsume,
            );
          }
        }
        // Stale outbox (queuedAt < appliedHead.createdAt) is left for
        // reclamation below — reclaimSupersededSectionsOutbox will clean it up.
      } else {
        clearChannelSectionsOutbox(pubkey, relayUrl);
      }
      if (result.action === "apply-remote") {
        // Head fetch succeeded: reclaim any foreign window's write-once outbox
        // key the head STRICTLY supersedes (`queuedAt` < head `created_at`).
        // Gated on the fetched head; records are immutable so no recheck is
        // needed and a live peer's edit queued at/after the head is kept. A
        // `hold` (absent/failed) reclaims none.
        reclaimSupersededSectionsOutbox(
          pubkey,
          relayUrl,
          result.data.createdAt,
        );
      }
    });
    return () => {
      cancelled = true;
    };
  }, [pubkey, relayUrl, applyRemote]);

  // Reconciliation loop (fix 1): a single scheduler that both retries a failed
  // bootstrap with bounded backoff and periodically re-fetches the head, so
  // stale-at-open state converges without waiting for a reconnect event a
  // healthy socket never fires. Also refreshes when the window becomes visible.
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
      void managerRef.current?.fetchRemoteSections().then((result) => {
        if (cancelled) return;
        if (result.status === "found") {
          // applyRemote defers to a pending local edit (whose own debounced
          // publish converges via publish-or-adopt), so a periodic reconcile
          // can never drop it — no re-queue needed.
          setStore(applyRemote(result.data));
          delayMs = RECONCILE_STEADY_MS; // relay answered → steady cadence
        } else if (result.status === "absent") {
          delayMs = RECONCILE_STEADY_MS; // answered (no blob) → steady cadence
        } else {
          delayMs = Math.min(delayMs * 2, RECONCILE_RETRY_MAX_MS); // fetch failed → back off
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
        // Wake the existing pending edit's cycle rather than re-queueing it via
        // publishSections(): a re-queue would bump the generation and reset the
        // frozen publishBaseline to the just-fetched head, so a remote that won
        // LWW while the edit was pending would be published over instead of
        // adopted (Carl P1). retryPendingPublish keeps the baseline, so the
        // pre-publish check still adopts a genuinely-advanced remote.
        managerRef.current?.retryPendingPublish();
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
        const assignments = { ...prev.assignments };
        delete assignments[channelId];
        assignments[channelId] = sectionId;
        const next = boundChannelSectionsStore({
          ...prev,
          assignments,
        });
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
