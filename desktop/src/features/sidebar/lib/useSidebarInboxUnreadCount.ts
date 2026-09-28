import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { markHiddenDmFeedItems } from "@/features/channels/dmResurface";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useHiddenDmIds } from "@/features/channels/useHiddenDmIds";
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
 * Folds `threadActivityFeedItems` and `markHiddenDmFeedItems` the same way
 * HomeScreen does so live non-mention thread replies and hidden DMs update
 * the badge to the same unread set Inbox shows.
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
  const hiddenDmIds = useHiddenDmIds(identityQuery.data?.pubkey);

  // Same augmentation as HomeScreen: live thread activity + hidden-DM typing
  // so the badge matches InboxListPane unread under the default All filter.
  const feed = React.useMemo((): HomeFeedResponse | undefined => {
    if (homeFeedQuery.data === undefined) return undefined;
    const withThreadActivity =
      threadActivityFeedItems.length === 0
        ? homeFeedQuery.data
        : {
            ...homeFeedQuery.data,
            feed: {
              ...homeFeedQuery.data.feed,
              activity: [
                ...homeFeedQuery.data.feed.activity,
                ...threadActivityFeedItems,
              ],
            },
          };
    return markHiddenDmFeedItems(withThreadActivity, hiddenDmIds);
  }, [hiddenDmIds, homeFeedQuery.data, threadActivityFeedItems]);

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
