import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { HomeView } from "@/features/home/ui/HomeView";
import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";
import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_MESSAGE,
} from "@/shared/lib/relayError";

type HomeScreenProps = {
  availableChannelIds: ReadonlySet<string>;
  currentPubkey?: string;
  onOpenContext: (channelId: string, messageId: string) => void;
};

export function HomeScreen({
  availableChannelIds,
  currentPubkey,
  onOpenContext,
}: HomeScreenProps) {
  const homeFeedQuery = useHomeFeedQuery();
  const { threadActivityItems } = useAppShell();

  const augmentedFeed = React.useMemo((): HomeFeedResponse | undefined => {
    if (!homeFeedQuery.data) return undefined;
    if (!threadActivityItems || threadActivityItems.length === 0) {
      return homeFeedQuery.data;
    }

    const syntheticItems: FeedItem[] = threadActivityItems.map(
      (item: ThreadActivityItem) => ({
        id: item.id,
        kind: item.kind,
        pubkey: item.pubkey,
        content: item.content,
        createdAt: item.createdAt,
        channelId: item.channelId,
        channelName: item.channelName,
        channelType: undefined,
        tags: item.tags,
        category: "activity" as const,
      }),
    );

    return {
      ...homeFeedQuery.data,
      feed: {
        ...homeFeedQuery.data.feed,
        activity: [...homeFeedQuery.data.feed.activity, ...syntheticItems],
      },
    };
  }, [homeFeedQuery.data, threadActivityItems]);

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
        onOpenContext={onOpenContext}
        onRefresh={() => {
          void homeFeedQuery.refetch();
        }}
      />
    </div>
  );
}
