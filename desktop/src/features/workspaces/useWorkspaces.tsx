import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react";
import type { ReactNode } from "react";

import type { Workspace } from "./types";
import {
  clearWorkspaceStorage,
  loadActiveWorkspaceId,
  loadWorkspaces,
  saveActiveWorkspaceId,
  saveWorkspaces,
} from "./workspaceStorage";
import { removeSelfProfileCachesForRelay } from "@/features/profile/lib/selfProfileStorage";

export type UseWorkspacesReturn = {
  workspaces: Workspace[];
  activeWorkspace: Workspace | null;
  /** Counter bumped when the active workspace's config changes (relayUrl/token). */
  reinitKey: number;
  /** Add a workspace, deduplicating by relayUrl. Returns the final ID in the list. */
  addWorkspace: (workspace: Workspace) => string;
  clearWorkspaces: () => void;
  removeWorkspace: (id: string) => void;
  switchWorkspace: (id: string) => void;
  /** Force the active workspace to re-init (e.g. after a deep-link reconnect). */
  reconnectWorkspace: () => void;
  updateWorkspace: (
    id: string,
    updates: Partial<
      Pick<Workspace, "name" | "relayUrl" | "token" | "pubkey" | "reposDir">
    >,
  ) => void;
};

const WorkspacesContext = createContext<UseWorkspacesReturn | null>(null);

export function WorkspacesProvider({ children }: { children: ReactNode }) {
  const value = useWorkspacesInternal();
  return (
    <WorkspacesContext.Provider value={value}>
      {children}
    </WorkspacesContext.Provider>
  );
}

export function useWorkspaces(): UseWorkspacesReturn {
  const ctx = useContext(WorkspacesContext);
  if (!ctx) {
    throw new Error("useWorkspaces must be used within a WorkspacesProvider");
  }
  return ctx;
}

function useWorkspacesInternal(): UseWorkspacesReturn {
  const [workspaces, setWorkspacesState] =
    useState<Workspace[]>(loadWorkspaces);
  const [activeId, setActiveId] = useState<string | null>(
    loadActiveWorkspaceId,
  );
  const [reinitKey, setReinitKey] = useState(0);
  const workspacesRef = useRef(workspaces);
  workspacesRef.current = workspaces;

  const activeWorkspace = useMemo(
    () => workspaces.find((w) => w.id === activeId) ?? workspaces[0] ?? null,
    [workspaces, activeId],
  );

  const addWorkspace = useCallback((workspace: Workspace): string => {
    const existing = workspacesRef.current.find(
      (w) => w.relayUrl === workspace.relayUrl,
    );
    const resolvedId = existing?.id ?? workspace.id;
    setWorkspacesState((prev) => {
      const dup = prev.find((w) => w.relayUrl === workspace.relayUrl);
      let next: Workspace[];
      if (dup) {
        next = prev.map((w) =>
          w.id === dup.id
            ? {
                ...w,
                name: workspace.name || w.name,
                token: workspace.token ?? w.token,
                pubkey: workspace.pubkey ?? w.pubkey,
              }
            : w,
        );
      } else {
        next = [...prev, workspace];
      }
      saveWorkspaces(next);
      return next;
    });
    return resolvedId;
  }, []);

  const clearWorkspaces = useCallback(() => {
    clearWorkspaceStorage();
    setWorkspacesState([]);
    setActiveId(null);
  }, []);

  const removeWorkspace = useCallback(
    (id: string) => {
      // GC self-profile caches for the removed workspace's relay. Mirror the
      // updater guard (length > 1) so we only GC when removal will actually
      // proceed. Runs outside the updater — updaters can execute twice under
      // React StrictMode.
      if (workspaces.length > 1) {
        const removed = workspaces.find((w) => w.id === id);
        if (removed) {
          removeSelfProfileCachesForRelay(removed.relayUrl);
        }
      }

      setWorkspacesState((prev) => {
        // Never allow removing the last workspace
        if (prev.length <= 1) {
          return prev;
        }
        const next = prev.filter((w) => w.id !== id);
        saveWorkspaces(next);

        // If removing the active workspace, switch to first remaining
        if (activeId === id && next.length > 0) {
          saveActiveWorkspaceId(next[0].id);
          setActiveId(next[0].id);
        }

        return next;
      });
    },
    [activeId, workspaces],
  );

  const switchWorkspace = useCallback(
    (id: string) => {
      if (id === activeId) return;
      saveActiveWorkspaceId(id);
      setActiveId(id);
    },
    [activeId],
  );

  const reconnectWorkspace = useCallback(() => {
    setReinitKey((k) => k + 1);
  }, []);

  const updateWorkspace = useCallback(
    (
      id: string,
      updates: Partial<
        Pick<Workspace, "name" | "relayUrl" | "token" | "pubkey" | "reposDir">
      >,
    ) => {
      setWorkspacesState((prev) => {
        // Prevent duplicate relay URLs across workspaces
        if (
          updates.relayUrl &&
          prev.some((w) => w.id !== id && w.relayUrl === updates.relayUrl)
        ) {
          return prev;
        }
        const next = prev.map((w) => (w.id === id ? { ...w, ...updates } : w));
        saveWorkspaces(next);
        return next;
      });
      // If the active workspace's relay URL or token changed, bump reinitKey
      // so the React tree remounts with the new config.
      if (
        id === activeId &&
        (updates.relayUrl || updates.token !== undefined)
      ) {
        setReinitKey((k) => k + 1);
      }
    },
    [activeId],
  );

  return {
    workspaces,
    activeWorkspace,
    reinitKey,
    addWorkspace,
    clearWorkspaces,
    removeWorkspace,
    switchWorkspace,
    reconnectWorkspace,
    updateWorkspace,
  };
}
