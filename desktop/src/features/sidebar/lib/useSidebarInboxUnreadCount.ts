import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { buildInboxItems } from "@/features/home/lib/inbox";
import {
  deriveSidebarInboxUnreadCount,
  projectInboxEffectiveDoneSet,
} from "@/features/home/lib/inboxUnreadCount";
import { filterInboxItems } from "@/features/home/lib/inboxViewHelpers";
import { useOwnedAgentPubkeys } from "@/features/home/useOwnedAgentPubkeys";
import type { HomeFeedResponse } from "@/shared/api/types";
import { useIdentityQuery } from "@/shared/api/hooks";

/**
 * Inbox left-nav unread: same source as InboxListPane's Mark-all-read numeral
 * (default "all" filter rows not in the effective done set). Does **not** use
 * `homeBadgeCount`, which only counts mentions/needsAction and is zeroed by
 * `homeBadgeEnabled` / seen-feed marking after visiting Home.
 *
 * Folds `threadActivityFeedItems` the same way HomeScreen does so live
 * non-mention thread replies update the badge without waiting for the home
 * feed poll.
 */
export function useSidebarInboxUnreadCount(): number | undefined {
  const identityQuery = useIdentityQuery();
  const homeFeedQuery = useHomeFeedQuery();
  const channelsQuery = useChannelsQuery();
  const {
    feedItemState,
    getChannelReadAt,
    getMessageReadAt,
    getThreadReadAt,
    readStateVersion,
    threadActivityFeedItems,
  } = useAppShell();
  const ownedAgentPubkeys = useOwnedAgentPubkeys(
    true,
    undefined,
    identityQuery.data?.pubkey,
  );

  const feed = React.useMemo((): HomeFeedResponse | undefined => {
    if (homeFeedQuery.data === undefined) return undefined;
    if (threadActivityFeedItems.length === 0) return homeFeedQuery.data;
    return {
      ...homeFeedQuery.data,
      feed: {
        ...homeFeedQuery.data.feed,
        activity: [
          ...homeFeedQuery.data.feed.activity,
          ...threadActivityFeedItems,
        ],
      },
    };
  }, [homeFeedQuery.data, threadActivityFeedItems]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion invalidates read lookups
  return React.useMemo(() => {
    if (feed === undefined) return undefined;

    const items = filterInboxItems(
      buildInboxItems({
        channels: channelsQuery.data,
        currentPubkey: identityQuery.data?.pubkey,
        feed,
        getChannelReadAt,
        getMessageReadAt,
        getThreadReadAt,
      }),
    );
    const doneSet = projectInboxEffectiveDoneSet(items, {
      getChannelReadAt,
      getMessageReadAt,
      getThreadReadAt,
      localDoneSet: feedItemState.doneSet,
      localUnreadSet: feedItemState.unreadSet,
    });
    return deriveSidebarInboxUnreadCount({
      items,
      doneSet,
      ownedAgentPubkeys,
    });
  }, [
    channelsQuery.data,
    feed,
    feedItemState.doneSet,
    feedItemState.unreadSet,
    getChannelReadAt,
    getMessageReadAt,
    getThreadReadAt,
    identityQuery.data?.pubkey,
    ownedAgentPubkeys,
    readStateVersion,
  ]);
}
