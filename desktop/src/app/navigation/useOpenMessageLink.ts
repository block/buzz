import * as React from "react";

import { beginMessageLinkRequest } from "@/app/navigation/messageLinkRequests";
import { resolveMessageLinkDestination } from "@/app/navigation/resolveMessageLinkDestination";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";

/**
 * Opens a `buzz://message` link at its real destination.
 *
 * Resolves the target event's kind before routing: `/channels/$channelId`
 * hardcodes `selectedPostId={null}`, so a forum post or comment sent through
 * `goChannel` lands on the post list with nothing selected. Stream targets keep
 * the previous behaviour — `goChannel` with `messageId`, resolved by
 * `useAnchoredScroll` + `getEventById` backfill.
 *
 * Shared by the in-app markdown handler and the deep-link listener so both
 * route identically.
 *
 * Returns whether navigation was accepted, so refused links remain queued.
 * Pending lookups cannot navigate after this hook unmounts or the caller's
 * lifecycle signal aborts. Newer activations supersede older pending lookups,
 * including those started by another mounted message renderer.
 */
export function useOpenMessageLink() {
  const { goChannel, goForumPost } = useAppNavigation();

  const lifecycle = React.useRef<AbortController | null>(null);
  React.useEffect(() => {
    const controller = new AbortController();
    lifecycle.current = controller;
    return () => controller.abort();
  }, []);

  return React.useCallback(
    async (
      link: ParsedMessageLink,
      options?: {
        signal?: AbortSignal;
        /** Disable best-effort fallback when success acknowledges a durable link. */
        allowChannelFallback?: boolean;
      },
    ): Promise<boolean> => {
      const signal = options?.signal;
      const ownerSignal = lifecycle.current?.signal;
      const cancelled = () => ownerSignal?.aborted || signal?.aborted;
      if (cancelled()) return false;
      const isCurrent = beginMessageLinkRequest();
      const destination = await resolveMessageLinkDestination(
        link.channelId,
        link.messageId,
        link.threadRootId,
      );
      if (cancelled() || !isCurrent()) return false;
      if (!destination) {
        if (options?.allowChannelFallback === false) return false;
        return goChannel(link.channelId, {
          messageId: link.messageId,
          threadRootId: link.threadRootId,
        });
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
}
