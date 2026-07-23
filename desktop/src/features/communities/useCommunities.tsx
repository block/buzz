import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";

import type { Community } from "./types";
import {
  clearCommunityStorage,
  loadActiveCommunityId,
  loadCommunities,
  saveActiveCommunityId,
  saveCommunities,
} from "./communityStorage";
import { removeSelfProfileCachesForRelay } from "@/features/profile/lib/selfProfileStorage";
import { removeChannelSnapshotForRelay } from "@/features/channels/channelSnapshot";
import { removeMessageSnapshotsForRelay } from "@/features/messages/lib/messageSnapshot";
import { clearSavedCommunitySnapshot } from "@/features/agents/activeAgentTurnsStore";

export type UpdateCommunityResult =
  | { kind: "updated"; requiresReinit: boolean }
  | { kind: "unchanged" }
  | { kind: "duplicate-relay" }
  | { kind: "not-found" };

/**
 * Pure decision logic for updateCommunity — determines the outcome from a
 * synchronous snapshot of communities without side effects.  Extracted so the
 * 5-case result matrix is unit-testable outside React.
 */
export function resolveCommunityUpdateResult(
  communities: Community[],
  activeId: string | null,
  id: string,
  updates: Partial<
    Pick<Community, "name" | "relayUrl" | "token" | "pubkey" | "reposDir">
  >,
): UpdateCommunityResult {
  const current = communities.find((w) => w.id === id);
  if (!current) return { kind: "not-found" };

  if (
    updates.relayUrl !== undefined &&
    updates.relayUrl !== current.relayUrl &&
    communities.some((w) => w.id !== id && w.relayUrl === updates.relayUrl)
  ) {
    return { kind: "duplicate-relay" };
  }

  const hasChange =
    (updates.name !== undefined && updates.name !== current.name) ||
    (updates.relayUrl !== undefined && updates.relayUrl !== current.relayUrl) ||
    (updates.token !== undefined && updates.token !== current.token) ||
    (updates.pubkey !== undefined && updates.pubkey !== current.pubkey) ||
    (updates.reposDir !== undefined && updates.reposDir !== current.reposDir);

  if (!hasChange) return { kind: "unchanged" };

  const isActive = id === activeId;
  const backendFieldsChanged =
    isActive &&
    ((updates.relayUrl !== undefined &&
      updates.relayUrl !== current.relayUrl) ||
      (updates.token !== undefined && updates.token !== current.token) ||
      (updates.reposDir !== undefined &&
        updates.reposDir !== current.reposDir));

  return { kind: "updated", requiresReinit: backendFieldsChanged };
}

/**
 * Permute `communities` so that its order matches `orderedIds`.
 *
 * - Communities whose id appears in `orderedIds` are placed first, in the
 *   order given by `orderedIds`.
 * - Communities not mentioned in `orderedIds` (e.g. added after the drag
 *   completed) are appended at the end in their original relative order.
 *
 * Pure and side-effect-free — extracted so it can be unit-tested without
 * a DOM or React.
 */
export function applyCommunitiesOrder(
  communities: Community[],
  orderedIds: string[],
): Community[] {
  const byId = new Map(communities.map((c) => [c.id, c]));
  const seen = new Set<string>();
  const reordered: Community[] = [];

  for (const id of orderedIds) {
    const c = byId.get(id);
    if (c && !seen.has(id)) {
      reordered.push(c);
      seen.add(id);
    }
  }

  for (const c of communities) {
    if (!seen.has(c.id)) {
      reordered.push(c);
    }
  }

  return reordered;
}

export type UseCommunitiesReturn = {
  communities: Community[];
  activeCommunity: Community | null;
  /** Counter bumped when the active community's config changes (relayUrl/token). */
  reinitKey: number;
  /** Add a community, deduplicating by relayUrl. Returns the final ID in the list. */
  addCommunity: (community: Community) => string;
  clearCommunities: () => void;
  removeCommunity: (id: string) => void;
  switchCommunity: (id: string) => void;
  /** Force the active community to re-init (e.g. after a deep-link reconnect). */
  reconnectCommunity: () => void;
  updateCommunity: (
    id: string,
    updates: Partial<
      Pick<Community, "name" | "relayUrl" | "token" | "pubkey" | "reposDir">
    >,
  ) => UpdateCommunityResult;
  /** Persist a new display order for the rail. IDs not in orderedIds keep their relative position at the end. */
  reorderCommunities: (orderedIds: string[]) => void;
};

const CommunitiesContext = createContext<UseCommunitiesReturn | null>(null);

export function CommunitiesProvider({ children }: { children: ReactNode }) {
  const value = useCommunitiesInternal();
  return (
    <CommunitiesContext.Provider value={value}>
      {children}
    </CommunitiesContext.Provider>
  );
}

export function useCommunities(): UseCommunitiesReturn {
  const ctx = useContext(CommunitiesContext);
  if (!ctx) {
    throw new Error("useCommunities must be used within a CommunitiesProvider");
  }
  return ctx;
}

function useCommunitiesInternal(): UseCommunitiesReturn {
  const [communities, setCommunitiesState] =
    useState<Community[]>(loadCommunities);
  const [activeId, setActiveId] = useState<string | null>(
    loadActiveCommunityId,
  );
  const [reinitKey, setReinitKey] = useState(0);
  const communitiesRef = useRef(communities);
  communitiesRef.current = communities;

  const activeCommunity = useMemo(
    () => communities.find((w) => w.id === activeId) ?? communities[0] ?? null,
    [communities, activeId],
  );

  const addCommunity = useCallback((community: Community): string => {
    const existing = communitiesRef.current.find(
      (w) => w.relayUrl === community.relayUrl,
    );
    const resolvedId = existing?.id ?? community.id;
    setCommunitiesState((prev) => {
      const dup = prev.find((w) => w.relayUrl === community.relayUrl);
      let next: Community[];
      if (dup) {
        next = prev.map((w) =>
          w.id === dup.id
            ? {
                ...w,
                name: community.name || w.name,
                token: community.token ?? w.token,
                pubkey: community.pubkey ?? w.pubkey,
              }
            : w,
        );
      } else {
        next = [...prev, community];
      }
      saveCommunities(next);
      return next;
    });
    return resolvedId;
  }, []);

  const clearCommunities = useCallback(() => {
    clearCommunityStorage();
    setCommunitiesState([]);
    setActiveId(null);
  }, []);

  const removeCommunity = useCallback(
    (id: string) => {
      // GC self-profile caches for the removed community's relay. Mirror the
      // updater guard (length > 1) so we only GC when removal will actually
      // proceed. Runs outside the updater — updaters can execute twice under
      // React StrictMode.
      if (communities.length > 1) {
        const removed = communities.find((w) => w.id === id);
        if (removed) {
          removeSelfProfileCachesForRelay(removed.relayUrl);
          removeChannelSnapshotForRelay(removed.relayUrl);
          removeMessageSnapshotsForRelay(removed.relayUrl);
          clearSavedCommunitySnapshot(id);
        }
      }

      setCommunitiesState((prev) => {
        // Never allow removing the last community
        if (prev.length <= 1) {
          return prev;
        }
        const next = prev.filter((w) => w.id !== id);
        saveCommunities(next);

        // If removing the active community, switch to first remaining
        if (activeId === id && next.length > 0) {
          saveActiveCommunityId(next[0].id);
          setActiveId(next[0].id);
        }

        return next;
      });
    },
    [activeId, communities],
  );

  const switchCommunity = useCallback(
    (id: string) => {
      if (id === activeId) return;
      saveActiveCommunityId(id);
      setActiveId(id);
    },
    [activeId],
  );

  const reconnectCommunity = useCallback(() => {
    setReinitKey((k) => k + 1);
  }, []);

  const updateCommunity = useCallback(
    (
      id: string,
      updates: Partial<
        Pick<Community, "name" | "relayUrl" | "token" | "pubkey" | "reposDir">
      >,
    ): UpdateCommunityResult => {
      const result = resolveCommunityUpdateResult(
        communitiesRef.current,
        activeId,
        id,
        updates,
      );

      if (result.kind === "updated") {
        setCommunitiesState((prev) => {
          const next = prev.map((w) =>
            w.id === id ? { ...w, ...updates } : w,
          );
          saveCommunities(next);
          return next;
        });

        if (result.requiresReinit) {
          setReinitKey((k) => k + 1);
        }
      }

      return result;
    },
    [activeId],
  );

  const reorderCommunities = useCallback((orderedIds: string[]) => {
    setCommunitiesState((prev) => {
      const next = applyCommunitiesOrder(prev, orderedIds);
      saveCommunities(next);
      return next;
    });
  }, []);

  return {
    communities,
    activeCommunity,
    reinitKey,
    addCommunity,
    clearCommunities,
    removeCommunity,
    switchCommunity,
    reconnectCommunity,
    updateCommunity,
    reorderCommunities,
  };
}
