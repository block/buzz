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
  artilleryChannel?: string;
  artilleryMatch?: string;
  artilleryRoot?: string;
  item?: string;
  lab?: string;
  profile?: string;
  profileTab?: string;
  profileView?: string;
};

const ArtilleryGameLab = React.lazy(async () => {
  const module = await import("@/features/games/artillery/ArtilleryGameLab");
  return { default: module.ArtilleryGameLab };
});

function validateHomeSearch(search: Record<string, unknown>): HomeRouteSearch {
  return {
    artilleryChannel:
      typeof search.artilleryChannel === "string" &&
      search.artilleryChannel.length > 0
        ? search.artilleryChannel
        : undefined,
    artilleryMatch:
      typeof search.artilleryMatch === "string" &&
      search.artilleryMatch.length > 0
        ? search.artilleryMatch
        : undefined,
    artilleryRoot:
      typeof search.artilleryRoot === "string" &&
      search.artilleryRoot.length > 0
        ? search.artilleryRoot
        : undefined,
    item:
      typeof search.item === "string" && search.item.length > 0
        ? search.item
        : undefined,
    lab:
      typeof search.lab === "string" && search.lab.length > 0
        ? search.lab
        : undefined,
    profile:
      typeof search.profile === "string" && search.profile.length > 0
        ? search.profile
        : undefined,
    profileTab:
      typeof search.profileTab === "string" && search.profileTab.length > 0
        ? search.profileTab
        : undefined,
    profileView:
      typeof search.profileView === "string" && search.profileView.length > 0
        ? search.profileView
        : undefined,
  };
}

export const Route = createFileRoute("/")({
  validateSearch: validateHomeSearch,
  component: HomeRouteComponent,
});

function HomeRouteComponent() {
  const search = Route.useSearch();
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

  return search.lab === "artillery" ? (
    <React.Suspense fallback={null}>
      <ArtilleryGameLab
        durableMatch={
          search.artilleryChannel &&
          search.artilleryMatch &&
          search.artilleryRoot
            ? {
                channelId: search.artilleryChannel,
                matchId: search.artilleryMatch,
                rootEventId: search.artilleryRoot,
              }
            : null
        }
      />
    </React.Suspense>
  ) : (
    <HomeScreen
      availableChannelIds={availableChannelIds}
      currentPubkey={identityQuery.data?.pubkey}
      onOpenContext={(channelId, messageId, threadRootId) => {
        void goChannel(channelId, { messageId, threadRootId });
      }}
    />
  );
}
