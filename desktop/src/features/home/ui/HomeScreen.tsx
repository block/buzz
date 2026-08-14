import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useHomeFeedPagination, useHomeFeedQuery } from "@/features/home/hooks";
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
  const channelsQuery = useChannelsQuery();
  const { threadActivityFeedItems } = useAppShell();
  // Older inbox pages, cursor-fetched on scroll-end and merged under the
  // live (30s-polled) first page.
  const pagination = useHomeFeedPagination(
    homeFeedQuery.data,
    channelsQuery.data,
  );

  const augmentedFeed = React.useMemo((): HomeFeedResponse | undefined => {
    if (!pagination.feed) return undefined;
    if (threadActivityFeedItems.length === 0) {
      return pagination.feed;
    }

    return {
      ...pagination.feed,
      feed: {
        ...pagination.feed.feed,
        activity: [
          ...pagination.feed.feed.activity,
          ...threadActivityFeedItems,
        ],
      },
    };
  }, [pagination.feed, threadActivityFeedItems]);

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
        isLoading={homeFeedQuery.isLoading}
        isLoadingOlder={pagination.isFetchingOlder}
        onLoadOlder={pagination.hasOlder ? pagination.fetchOlder : undefined}
        onOpenContext={onOpenContext}
        onRefresh={() => {
          void homeFeedQuery.refetch();
        }}
      />
    </div>
  );
}
