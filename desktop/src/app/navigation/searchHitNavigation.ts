import { resolveSearchHitDestination } from "@/app/navigation/resolveSearchHitDestination";
import { createSearchHighlightNavigation } from "@/app/navigation/searchHighlightNavigation";
import { cacheSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import type { SearchHit } from "@/shared/api/types";

type SearchHitNavigationActions = {
  force?: boolean;
  query?: string;
  goChannel: (
    channelId: string,
    options?: {
      force?: boolean;
      messageId?: string;
      searchHighlight?: ReturnType<typeof createSearchHighlightNavigation>;
      threadRootId?: string | null;
    },
  ) => Promise<unknown>;
  goForumPost: (
    channelId: string,
    postId: string,
    options?: {
      force?: boolean;
      replyId?: string;
      searchHighlight?: ReturnType<typeof createSearchHighlightNavigation>;
    },
  ) => Promise<unknown>;
  signal?: AbortSignal;
};

export async function openSearchHitWithNavigation(
  hit: SearchHit,
  actions: SearchHitNavigationActions,
  resolveDestination = resolveSearchHitDestination,
): Promise<unknown> {
  if (actions.signal?.aborted) {
    return false;
  }

  const isLifecycleBound = Boolean(actions.signal);
  const searchHighlight = createSearchHighlightNavigation(
    hit.eventId,
    actions.query,
  );
  if (!isLifecycleBound) {
    cacheSearchHitEvent(hit);
  }

  const destination = await resolveDestination(hit);
  if (!destination || actions.signal?.aborted) {
    return false;
  }

  if (isLifecycleBound) {
    // Delay community-scoped writes for notification routing until async
    // destination resolution completes and its owner is still current.
    cacheSearchHitEvent(hit);
  }

  if (destination.kind === "forum-post") {
    return actions.goForumPost(destination.channelId, destination.postId, {
      force: actions.force || Boolean(searchHighlight),
      replyId: destination.replyId,
      searchHighlight,
    });
  }

  return actions.goChannel(destination.channelId, {
    force: actions.force || Boolean(searchHighlight),
    messageId: destination.messageId,
    searchHighlight,
    threadRootId: destination.threadRootId,
  });
}
