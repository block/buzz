import * as React from "react";
import {
  useCanGoBack,
  useLocation,
  useNavigate,
  useRouter,
} from "@tanstack/react-router";

import type { SearchHighlightNavigation } from "@/app/navigation/searchHighlightNavigation";
import { openSearchHitWithNavigation } from "@/app/navigation/searchHitNavigation";
import {
  allowNavigation,
  type GuardedNavigation,
  traverseHistory,
} from "@/app/navigation/navigationGuard";
import type { SearchHit } from "@/shared/api/types";

type NavigationBehavior = {
  force?: boolean;
  replace?: boolean;
  resetScroll?: boolean;
};

export function useAppNavigation() {
  const router = useRouter();
  const navigate = useNavigate();
  const location = useLocation();
  const canGoBack = useCanGoBack();

  const commitNavigation = React.useCallback(
    async (
      next: {
        to: string;
        params?: Record<string, string>;
        search?: Record<string, string | undefined>;
        state?:
          | Record<string, unknown>
          | ((
              previousState: Record<string, unknown>,
            ) => Record<string, unknown>);
      },
      behavior: NavigationBehavior = {},
      guardedTarget?: GuardedNavigation,
    ) => {
      const nextLocation = router.buildLocation(next as never);
      const hasStateUpdate = next.state !== undefined;

      if (
        location.href === nextLocation.href &&
        !behavior.force &&
        !hasStateUpdate
      ) {
        return false;
      }

      if (
        !allowNavigation(
          guardedTarget ?? { kind: "route", href: nextLocation.href },
        )
      ) {
        return false;
      }

      await navigate({
        ...next,
        replace: behavior.replace,
        resetScroll: behavior.resetScroll,
      } as never);
      return true;
    },
    [location.href, navigate, router],
  );

  const goHome = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/",
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goAgents = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/agents",
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goPulse = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/pulse",
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goProfile = React.useCallback(
    (pubkey: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/pulse",
          search: { profile: pubkey },
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goProjects = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/projects",
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goProject = React.useCallback(
    (
      projectId: string,
      behavior?: NavigationBehavior & {
        commitHash?: string;
        filePath?: string;
        pullRequestId?: string;
        issueId?: string;
        repositoryId?: string;
        /** Workspace tab requested by a share link (link vocabulary). */
        tab?: string;
        /** Unique per entity-link activation so repeating the same link can
         * re-apply an unchanged tab selection. */
        entityNavigationId?: string;
      },
    ) =>
      commitNavigation(
        {
          to: "/projects/$projectId",
          params: {
            projectId,
          },
          search: {
            ...(behavior?.commitHash
              ? { commitHash: behavior.commitHash }
              : {}),
            ...(behavior?.filePath ? { filePath: behavior.filePath } : {}),
            ...(behavior?.pullRequestId
              ? { pullRequestId: behavior.pullRequestId }
              : {}),
            ...(behavior?.issueId ? { issueId: behavior.issueId } : {}),
            ...(behavior?.repositoryId
              ? { repositoryId: behavior.repositoryId }
              : {}),
            ...(behavior?.tab ? { tab: behavior.tab } : {}),
          },
          state: behavior?.entityNavigationId
            ? { entityNavigationId: behavior.entityNavigationId }
            : undefined,
        },
        {
          ...behavior,
          force: Boolean(behavior?.entityNavigationId),
        },
      ),
    [commitNavigation],
  );

  const goWorkflows = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/workflows",
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goWorkflow = React.useCallback(
    (workflowId: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/workflows/$workflowId",
          params: {
            workflowId,
          },
          search: { pane: "trigger" },
          state: { workflowEditorHasOrigin: true },
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goNewWorkflow = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/workflows",
          search: { pane: "trigger", view: "create" },
          state: { workflowEditorHasOrigin: true },
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goNewWorkflowForChannel = React.useCallback(
    (channelId: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/workflows",
          search: {
            channel: channelId,
            pane: "trigger",
            view: "create",
          },
          state: { workflowEditorHasOrigin: true },
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goEditWorkflow = React.useCallback(
    (workflowId: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/workflows/$workflowId",
          params: { workflowId },
          search: { pane: "trigger", view: "edit" },
          state: { workflowEditorHasOrigin: true },
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goDuplicateWorkflow = React.useCallback(
    (workflowId: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/workflows/$workflowId",
          params: { workflowId },
          search: { pane: "trigger", view: "duplicate" },
          state: { workflowEditorHasOrigin: true },
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goChannel = React.useCallback(
    (
      channelId: string,
      options?: {
        /** Open the agent activity pane for this agent pubkey on arrival. */
        agentSession?: string;
        /**
         * When set, the main composer auto-submits the draft with this key
         * once on mount. Clears itself (via `?autoSend` search param) after
         * firing. Used by the Drafts panel "Send message" confirm flow.
         */
        autoSend?: string;
        /** Navigate even when the destination matches the current href.
         * Used by desktop-notification activation so a click is never
         * silently swallowed (block/buzz#3509). */
        force?: boolean;
        messageId?: string;
        /** Preserve an active search highlight; ordinary navigation clears it. */
        preserveSearchHighlight?: boolean;
        searchHighlight?: SearchHighlightNavigation;
        replace?: boolean;
        /** Open this thread panel directly without waiting for a timeline row. */
        thread?: string;
        threadRootId?: string | null;
      },
    ) => {
      return commitNavigation(
        {
          to: "/channels/$channelId",
          params: {
            channelId,
          },
          search: {
            ...(options?.messageId
              ? {
                  messageId: options.messageId,
                  threadRootId: options.threadRootId ?? undefined,
                }
              : {}),
            ...(options?.agentSession
              ? { agentSession: options.agentSession }
              : {}),
            ...(options?.thread ? { thread: options.thread } : {}),
            ...(options?.autoSend ? { autoSend: options.autoSend } : {}),
          },
          state: options?.preserveSearchHighlight
            ? undefined
            : (previousState: Record<string, unknown>) => ({
                ...previousState,
                searchHighlight: options?.searchHighlight ?? null,
              }),
        },
        {
          force: options?.force,
          replace: options?.replace,
          resetScroll: options?.messageId ? true : undefined,
        },
        options?.messageId
          ? {
              kind: "channel-message",
              channelId,
              messageId: options.messageId,
              threadRootId: options.threadRootId ?? null,
            }
          : undefined,
      );
    },
    [commitNavigation],
  );

  const goNewMessage = React.useCallback(
    (behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/messages/new",
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goForumPost = React.useCallback(
    (
      channelId: string,
      postId: string,
      options?: {
        /** Navigate even when the destination matches the current href. */
        force?: boolean;
        replace?: boolean;
        replyId?: string;
        /** Preserve an active search highlight; ordinary navigation clears it. */
        preserveSearchHighlight?: boolean;
        searchHighlight?: SearchHighlightNavigation;
      },
    ) => {
      return commitNavigation(
        {
          to: "/channels/$channelId/posts/$postId",
          params: {
            channelId,
            postId,
          },
          search: {
            ...(options?.replyId ? { replyId: options.replyId } : {}),
          },
          state: options?.preserveSearchHighlight
            ? undefined
            : (previousState: Record<string, unknown>) => ({
                ...previousState,
                searchHighlight: options?.searchHighlight ?? null,
              }),
        },
        {
          force: options?.force,
          replace: options?.replace,
          resetScroll: false,
        },
        {
          kind: "forum-post",
          channelId,
          postId,
          replyId: options?.replyId ?? null,
        },
      );
    },
    [commitNavigation],
  );

  const goSettings = React.useCallback(
    (section?: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/settings",
          search: section ? { section } : {},
        },
        behavior,
      ),
    [commitNavigation],
  );

  const closeSettings = React.useCallback(() => {
    if (canGoBack) {
      traverseHistory(router.history, "back");
      return;
    }

    void goHome({ replace: true });
  }, [canGoBack, goHome, router.history]);

  const closeWorkflowDetail = React.useCallback(() => {
    if (canGoBack) {
      traverseHistory(router.history, "back");
      return;
    }

    void goWorkflows({ replace: true });
  }, [canGoBack, goWorkflows, router.history]);

  const closeForumPost = React.useCallback(
    (channelId: string) => {
      if (canGoBack) {
        traverseHistory(router.history, "back");
        return;
      }

      void goChannel(channelId, { replace: true });
    },
    [canGoBack, goChannel, router.history],
  );

  const openSearchHit = React.useCallback(
    async (
      hit: SearchHit,
      behavior?: {
        /** Navigate even when the destination matches the current href.
         * Used by desktop-notification activation so a click is never
         * silently swallowed (block/buzz#3509). */
        force?: boolean;
        /** Search text to highlight after opening this result. */
        query?: string;
        /** Stop notification-driven routing when its owning lifecycle ends. */
        signal?: AbortSignal;
      },
    ) =>
      openSearchHitWithNavigation(hit, {
        force: behavior?.force,
        goChannel,
        goForumPost,
        query: behavior?.query,
        signal: behavior?.signal,
      }),
    [goChannel, goForumPost],
  );

  return {
    closeForumPost,
    closeSettings,
    closeWorkflowDetail,
    goAgents,
    goChannel,
    goDuplicateWorkflow,
    goEditWorkflow,
    goForumPost,
    goHome,
    goNewMessage,
    goNewWorkflow,
    goNewWorkflowForChannel,
    goProject,
    goProjects,
    goPulse,
    goProfile,
    goSettings,
    goWorkflow,
    goWorkflows,
    openSearchHit,
  };
}
