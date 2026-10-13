import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { markHiddenDmFeedItems } from "@/features/channels/dmResurface";
import { useHiddenDmIds } from "@/features/channels/useHiddenDmIds";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { useInboxPaging } from "@/features/home/useInboxPaging";
import { HomeView } from "@/features/home/ui/HomeView";
import type { HomeFeedResponse } from "@/shared/api/types";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_MESSAGE,
} from "@/shared/lib/relayError";

type HomeScreenProps = {
  availableChannelIds: ReadonlySet<string>;
  currentPubkey?: string;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
};

export function HomeScreen({
  availableChannelIds,
  currentPubkey,
  onOpenContext,
}: HomeScreenProps) {
  const homeFeedQuery = useHomeFeedQuery();
  const paging = useInboxPaging(homeFeedQuery.data);
  const pagedFeed = paging.feed;
  const { threadActivityFeedItems } = useAppShell();
  const hiddenDmIds = useHiddenDmIds(currentPubkey);

  const augmentedFeed = React.useMemo((): HomeFeedResponse | undefined => {
    if (!pagedFeed) return undefined;
    const withThreadActivity =
      threadActivityFeedItems.length === 0
        ? pagedFeed
        : {
            ...pagedFeed,
            feed: {
              ...pagedFeed.feed,
              activity: [
                ...pagedFeed.feed.activity,
                ...threadActivityFeedItems,
              ],
            },
          };
    return markHiddenDmFeedItems(withThreadActivity, hiddenDmIds);
  }, [hiddenDmIds, pagedFeed, threadActivityFeedItems]);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <HomeView
        availableChannelIds={availableChannelIds}
        currentPubkey={currentPubkey}
        errorMessage={
          homeFeedQuery.error !== null && homeFeedQuery.error !== undefined
            ? isRelayUnreachableError(homeFeedQuery.error)
              ? RELAY_UNREACHABLE_MESSAGE
              : homeFeedQuery.error instanceof Error
                ? homeFeedQuery.error.message
                : undefined
            : undefined
        }
        feed={augmentedFeed}
        inboxPaging={{
          hasMore: paging.hasMore,
          isLoading: paging.isLoadingOlder,
          error: paging.loadOlderError,
          onLoadOlder: () => {
            void paging.loadOlder();
          },
        }}
        isLoading={homeFeedQuery.isLoading}
        onOpenContext={onOpenContext}
        onRefresh={() => {
          void homeFeedQuery.refetch();
        }}
      />
    </div>
  );
}
