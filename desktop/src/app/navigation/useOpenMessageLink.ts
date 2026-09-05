import * as React from "react";

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
 * Returns the navigation promise. The deep-link listener acks a pending link by
 * resolving `true`, and that ack drops the link from the durable queue, so it
 * has to await the navigation it claims to have performed — the channel-only
 * listener beside it already awaits `goChannel`. Click handlers that have
 * nothing to wait for can discard it.
 */
export function useOpenMessageLink() {
  const { goChannel, goForumPost } = useAppNavigation();

  return React.useCallback(
    (link: ParsedMessageLink): Promise<void> =>
      resolveMessageLinkDestination(
        link.channelId,
        link.messageId,
        link.threadRootId,
      ).then(async (destination) => {
        if (destination.kind === "forum-post") {
          await goForumPost(destination.channelId, destination.postId, {
            replyId: destination.replyId,
          });
          return;
        }
        await goChannel(destination.channelId, {
          messageId: destination.messageId,
          threadRootId: destination.threadRootId,
        });
      }),
    [goChannel, goForumPost],
  );
}
