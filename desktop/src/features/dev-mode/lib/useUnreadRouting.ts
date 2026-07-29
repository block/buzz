import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import type { SubChannelIndex } from "@/features/dev-mode/lib/subChannels";
import {
  byCreatedAscending,
  DEV_MESSAGE_KINDS,
  selectInlineVisibleCount,
} from "@/features/dev-mode/lib/transcriptRoots";
import { findUnreadFamilyTarget } from "@/features/dev-mode/lib/unreadThreads";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * Opening a channel from its navigator row routes to what needs attention:
 * the family's unread tab. The unread thread's side chat opens only when
 * its unread replies are collapsed — hidden behind the "… N more replies"
 * affordance because a human already responded in the thread. Unread
 * replies that render inline in the main chat view (the leading agent run)
 * are marked read just by looking at the channel, so no side chat is
 * forced open for them.
 *
 * The decision needs the thread's subtree and the channel's member roles,
 * so it is deferred until both load; any navigation elsewhere makes the
 * deferred decision stale.
 *
 * Returns the routed open. Explicit destinations (tab clicks, palette,
 * ⌘[/⌘], ⌥↑↓) keep using the plain open and land exactly where asked.
 */
export function useUnreadRouting({
  subIndex,
  unreadChannelIds,
  topLevelUnreadChannelIds,
  activeChannel,
  roots,
  openChannel,
  openThread,
}: {
  subIndex: SubChannelIndex;
  unreadChannelIds: ReadonlySet<string>;
  topLevelUnreadChannelIds: ReadonlySet<string>;
  /** The channel whose transcript (roots) is currently loaded. */
  activeChannel: Channel | null;
  roots: readonly RelayEvent[];
  openChannel: (channelId: string) => void;
  /** Select the root's card and open its side chat. */
  openThread: (rootId: string) => void;
}): (channelId: string) => void {
  const { getThreadReadAt, threadActivityItems } = useAppShell();
  const activeChannelId = activeChannel?.id ?? null;

  const [pending, setPending] = React.useState<{
    channelId: string;
    rootId: string;
  } | null>(null);

  React.useEffect(() => {
    if (pending && pending.channelId !== activeChannelId) {
      setPending(null);
    }
  }, [activeChannelId, pending]);

  // Shares the transcript's thread-subtree and member query caches, so the
  // routed channel is fetching these anyway.
  const pendingChannel =
    pending && activeChannelId === pending.channelId ? activeChannel : null;
  const repliesQuery = useThreadReplies(
    pendingChannel,
    pending?.rootId ?? null,
  );
  const membersQuery = useChannelMembersQuery(pendingChannel?.id ?? null);

  // Complete the routing once the target root, its subtree, and the member
  // roles are loaded. Declared after the shell's selection-reset effect
  // runs for the channel switch, so the reset cannot clobber the opened
  // side chat.
  React.useEffect(() => {
    if (!pending || pending.channelId !== activeChannelId) return;
    if (!roots.some((root) => root.id === pending.rootId)) return;
    const subtree = repliesQuery.data;
    const members = membersQuery.data;
    if (subtree === undefined || members === undefined) return;

    const replies = subtree
      .filter((event) => DEV_MESSAGE_KINDS.has(event.kind))
      .sort(byCreatedAscending);
    const isAgent = (pubkey: string) =>
      members.some(
        (member) =>
          member.pubkey === pubkey && (member.isAgent || member.role === "bot"),
      );
    const visibleCount = selectInlineVisibleCount(replies, isAgent);
    const readAt = getThreadReadAt(pending.rootId, pending.channelId);
    const hasCollapsedUnread = replies
      .slice(visibleCount)
      .some((reply) => readAt === null || reply.created_at > readAt);

    setPending(null);
    if (hasCollapsedUnread) {
      openThread(pending.rootId);
    }
  }, [
    activeChannelId,
    getThreadReadAt,
    membersQuery.data,
    openThread,
    pending,
    repliesQuery.data,
    roots,
  ]);

  return React.useCallback(
    (channelId: string) => {
      const mainId = subIndex.parentIdByChildId.get(channelId) ?? channelId;
      const target = findUnreadFamilyTarget({
        familyChannelIds: [
          mainId,
          ...(subIndex.subsByParentId.get(mainId) ?? []).map((sub) => sub.id),
        ],
        unreadChannelIds,
        topLevelUnreadChannelIds,
        threadActivity: threadActivityItems,
        getThreadReadAt,
      });
      if (!target) {
        openChannel(channelId);
        return;
      }
      if (target.channelId !== activeChannelId) {
        openChannel(target.channelId);
      }
      setPending(
        target.rootId
          ? { channelId: target.channelId, rootId: target.rootId }
          : null,
      );
    },
    [
      activeChannelId,
      getThreadReadAt,
      openChannel,
      subIndex,
      threadActivityItems,
      topLevelUnreadChannelIds,
      unreadChannelIds,
    ],
  );
}
