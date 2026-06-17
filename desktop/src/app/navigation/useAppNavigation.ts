import * as React from "react";
import {
  useCanGoBack,
  useLocation,
  useNavigate,
  useRouter,
} from "@tanstack/react-router";

import { cacheSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import { resolveSearchHitDestination } from "@/app/navigation/resolveSearchHitDestination";
import type { SearchHit } from "@/shared/api/types";

type NavigationBehavior = {
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
      },
      behavior: NavigationBehavior = {},
    ) => {
      const nextLocation = router.buildLocation(next as never);

      if (location.href === nextLocation.href) {
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
    (projectId: string, behavior?: NavigationBehavior) =>
      commitNavigation(
        {
          to: "/projects/$projectId",
          params: {
            projectId,
          },
        },
        behavior,
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
        },
        behavior,
      ),
    [commitNavigation],
  );

  const goChannel = React.useCallback(
    (
      channelId: string,
      options?: {
        messageId?: string;
        replace?: boolean;
        threadRootId?: string | null;
      },
    ) =>
      commitNavigation(
        {
          to: "/channels/$channelId",
          params: {
            channelId,
          },
          search: options?.messageId
            ? {
                messageId: options.messageId,
                threadRootId: options.threadRootId ?? undefined,
              }
            : {},
        },
        {
          replace: options?.replace,
          resetScroll: options?.messageId ? true : undefined,
        },
      ),
    [commitNavigation],
  );

  const goForumPost = React.useCallback(
    (
      channelId: string,
      postId: string,
      options?: {
        replace?: boolean;
        replyId?: string;
      },
    ) =>
      commitNavigation(
        {
          to: "/channels/$channelId/posts/$postId",
          params: {
            channelId,
            postId,
          },
          search: options?.replyId ? { replyId: options.replyId } : {},
        },
        {
          replace: options?.replace,
          resetScroll: false,
        },
      ),
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
      router.history.back();
      return;
    }

    void goHome({ replace: true });
  }, [canGoBack, goHome, router.history]);

  const closeWorkflowDetail = React.useCallback(() => {
    if (canGoBack) {
      router.history.back();
      return;
    }

    void goWorkflows({ replace: true });
  }, [canGoBack, goWorkflows, router.history]);

  const closeForumPost = React.useCallback(
    (channelId: string) => {
      if (canGoBack) {
        router.history.back();
        return;
      }

      void goChannel(channelId, { replace: true });
    },
    [canGoBack, goChannel, router.history],
  );

  const openSearchHit = React.useCallback(
    async (hit: SearchHit) => {
      cacheSearchHitEvent(hit);

      const destination = await resolveSearchHitDestination(hit);
      if (!destination) {
        return false;
      }

      if (destination.kind === "forum-post") {
        return goForumPost(destination.channelId, destination.postId, {
          replyId: destination.replyId,
        });
      }

      return goChannel(destination.channelId, {
        messageId: destination.messageId,
        threadRootId: destination.threadRootId,
      });
    },
    [goChannel, goForumPost],
  );

  return {
    closeForumPost,
    closeSettings,
    closeWorkflowDetail,
    goAgents,
    goChannel,
    goForumPost,
    goHome,
    goProject,
    goProjects,
    goPulse,
    goSettings,
    goWorkflow,
    goWorkflows,
    openSearchHit,
  };
}
