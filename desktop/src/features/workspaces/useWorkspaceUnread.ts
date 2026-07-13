import * as React from "react";

import { getIdentity } from "@/shared/api/tauriIdentity";
import { markWorkspaceRead } from "@/features/workspaces/workspaceMarkRead";
import { pollWorkspaceUnread } from "@/features/workspaces/workspaceUnreadObserver";

import type { Workspace } from "./types";

const WORKSPACE_UNREAD_POLL_MS = 30_000;

/**
 * Per-workspace unread summary for the workspace rail.
 *
 * `state` distinguishes "observed, no unread" from "not observed yet" so the
 * rail never renders a false "no unread" for a relay it could not reach:
 * - `unknown`  — not yet observed (render dim, no unread affordance)
 * - `loading`  — observation in flight (render dim/skeleton)
 * - `ready`    — observed; trust `hasUnread` / `count`
 * - `error`    — observation failed (render neutral, never "no unread")
 *
 * `count` carries the MENTION count (not total unread) — the rail shows a dot
 * for any unread and a numeric badge only when mentions are present.
 */
export type WorkspaceUnreadState = {
  hasUnread: boolean;
  count?: number;
  state: "unknown" | "loading" | "ready" | "error";
};

const unknownUnreadState: WorkspaceUnreadState = {
  hasUnread: false,
  state: "unknown",
};

function seedWorkspaceStates(
  workspaces: Workspace[],
  previous: Record<string, WorkspaceUnreadState>,
): Record<string, WorkspaceUnreadState> {
  const next: Record<string, WorkspaceUnreadState> = {};
  for (const workspace of workspaces) {
    next[workspace.id] = previous[workspace.id] ?? unknownUnreadState;
  }
  return next;
}

/**
 * Observe unread activity for INACTIVE workspaces without touching the active
 * relay singleton.
 */
export function useWorkspaceUnread(
  workspaces: Workspace[],
  activeWorkspaceId: string | null,
): {
  unreadByWorkspace: Record<string, WorkspaceUnreadState>;
  markWorkspaceRead: (workspaceId: string) => Promise<void>;
} {
  const [unreadByWorkspace, setUnreadByWorkspace] = React.useState<
    Record<string, WorkspaceUnreadState>
  >(() => seedWorkspaceStates(workspaces, {}));

  React.useEffect(() => {
    let cancelled = false;
    let pollTimer: number | null = null;
    const inactiveWorkspaces = workspaces.filter(
      (workspace) => workspace.id !== activeWorkspaceId,
    );

    setUnreadByWorkspace((previous) =>
      seedWorkspaceStates(workspaces, previous),
    );

    const markLoading = (workspaceId: string) => {
      setUnreadByWorkspace((previous) => {
        const current = previous[workspaceId] ?? unknownUnreadState;
        if (current.state === "ready") {
          return previous;
        }
        return {
          ...previous,
          [workspaceId]: { hasUnread: false, state: "loading" },
        };
      });
    };

    const markReady = (
      workspaceId: string,
      result: { hasUnread: boolean; mentionCount: number },
    ) => {
      setUnreadByWorkspace((previous) => ({
        ...previous,
        [workspaceId]: {
          hasUnread: result.hasUnread,
          count: result.mentionCount > 0 ? result.mentionCount : undefined,
          state: "ready",
        },
      }));
    };

    const markError = (workspaceId: string) => {
      setUnreadByWorkspace((previous) => ({
        ...previous,
        [workspaceId]: { hasUnread: false, state: "error" },
      }));
    };

    const scheduleNextPoll = () => {
      if (cancelled) return;
      pollTimer = window.setTimeout(() => {
        void pollInactiveWorkspaces();
      }, WORKSPACE_UNREAD_POLL_MS);
    };

    const pollInactiveWorkspaces = async () => {
      if (inactiveWorkspaces.length === 0) {
        return;
      }

      let pubkey: string;
      try {
        pubkey = (await getIdentity()).pubkey;
      } catch {
        for (const workspace of inactiveWorkspaces) {
          if (cancelled) return;
          markError(workspace.id);
        }
        scheduleNextPoll();
        return;
      }

      for (const workspace of inactiveWorkspaces) {
        if (cancelled) return;
        markLoading(workspace.id);
        try {
          const result = await pollWorkspaceUnread(workspace, pubkey);
          if (cancelled) return;
          markReady(workspace.id, result);
        } catch (error) {
          console.debug(
            `[WorkspaceUnread] poll failed workspace=${workspace.id}:`,
            error,
          );
          if (cancelled) return;
          markError(workspace.id);
        }
      }

      scheduleNextPoll();
    };

    void pollInactiveWorkspaces();

    return () => {
      cancelled = true;
      if (pollTimer !== null) {
        window.clearTimeout(pollTimer);
      }
    };
  }, [activeWorkspaceId, workspaces]);

  const workspacesRef = React.useRef(workspaces);
  workspacesRef.current = workspaces;

  const markRead = React.useCallback(
    async (workspaceId: string) => {
      const workspace = workspacesRef.current.find(
        (candidate) => candidate.id === workspaceId,
      );
      if (!workspace || workspaceId === activeWorkspaceId) return;

      const { pubkey } = await getIdentity();
      await markWorkspaceRead(workspace, pubkey);
      // Optimistic clear — the next poll re-verifies against the relay.
      setUnreadByWorkspace((previous) => ({
        ...previous,
        [workspaceId]: { hasUnread: false, state: "ready" },
      }));
    },
    [activeWorkspaceId],
  );

  return { unreadByWorkspace, markWorkspaceRead: markRead };
}
