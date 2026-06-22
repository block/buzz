import * as React from "react";
import { createFileRoute } from "@tanstack/react-router";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { HomeScreen } from "@/features/home/ui/HomeScreen";
import {
  consumePendingWelcomeChannel,
  WELCOME_CHANNEL_READY_EVENT,
} from "@/features/onboarding/welcome";
import { useIdentityQuery } from "@/shared/api/hooks";

type HomeRouteSearch = {
  item?: string;
};

function validateHomeSearch(search: Record<string, unknown>): HomeRouteSearch {
  return {
    item:
      typeof search.item === "string" && search.item.length > 0
        ? search.item
        : undefined,
  };
}

export const Route = createFileRoute("/")({
  validateSearch: validateHomeSearch,
  component: HomeRouteComponent,
});

function HomeRouteComponent() {
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const channels = channelsQuery.data ?? [];
  const availableChannelIds = React.useMemo(
    () => new Set(channels.map((channel) => channel.id)),
    [channels],
  );
  const availableChannelIdsRef = React.useRef(availableChannelIds);
  const openPendingWelcomeChannel = React.useCallback(
    (ids: ReadonlySet<string>) => {
      const welcomeChannelId = consumePendingWelcomeChannel(ids);
      if (!welcomeChannelId) {
        return;
      }

      void goChannel(welcomeChannelId, { replace: true });
    },
    [goChannel],
  );

  React.useEffect(() => {
    availableChannelIdsRef.current = availableChannelIds;
  }, [availableChannelIds]);

  React.useEffect(() => {
    function handleWelcomeChannelReady() {
      openPendingWelcomeChannel(availableChannelIdsRef.current);
    }

    window.addEventListener(
      WELCOME_CHANNEL_READY_EVENT,
      handleWelcomeChannelReady,
    );
    return () => {
      window.removeEventListener(
        WELCOME_CHANNEL_READY_EVENT,
        handleWelcomeChannelReady,
      );
    };
  }, [openPendingWelcomeChannel]);

  React.useEffect(() => {
    openPendingWelcomeChannel(availableChannelIds);
  }, [availableChannelIds, openPendingWelcomeChannel]);

  return (
    <HomeScreen
      availableChannelIds={availableChannelIds}
      currentPubkey={identityQuery.data?.pubkey}
      onOpenContext={(channelId, messageId, threadRootId) => {
        void goChannel(channelId, { messageId, threadRootId });
      }}
    />
  );
}
