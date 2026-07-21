import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { HomeView } from "@/features/home/ui/HomeView";
import type { HomeFeedResponse } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
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
  const activityEnabled = useFeatureEnabled("activity");
  const { threadActivityFeedItems } = useAppShell();

  const augmentedFeed = React.useMemo((): HomeFeedResponse | undefined => {
    if (!homeFeedQuery.data) return undefined;
    if (threadActivityFeedItems.length === 0) {
      return homeFeedQuery.data;
    }

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

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <HomeView
        activityEnabled={activityEnabled}
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
        onOpenContext={onOpenContext}
        onRefresh={() => {
          void homeFeedQuery.refetch();
        }}
      />
    </div>
  );
}
