import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { HomeView } from "@/features/home/ui/HomeView";
import {
  type WelcomeAction,
  WelcomeEmptyState,
} from "@/features/home/ui/WelcomeEmptyState";
import { useWelcomeFirstRun } from "@/features/home/useWelcomeFirstRun";
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
  const { threadActivityFeedItems } = useAppShell();
  const { goNewMessage } = useAppNavigation();
  const { showWelcome, dismiss } = useWelcomeFirstRun(currentPubkey);

  const handleWelcomeAction = React.useCallback(
    (action: WelcomeAction) => {
      // Dismissing reveals the normal Home inbox behind the welcome state.
      // "inbox" and "channels" simply dismiss (inbox is Home; channels live in
      // the sidebar); "dm" additionally opens the new-message composer.
      dismiss();
      if (action === "dm") {
        void goNewMessage();
      }
    },
    [dismiss, goNewMessage],
  );

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

  if (showWelcome) {
    return (
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <WelcomeEmptyState onAction={handleWelcomeAction} />
      </div>
    );
  }

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
