import * as React from "react";

import type { deriveShellRoute } from "@/app/AppShell.helpers";
import type { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  consumePendingCommunityRestore,
  loadCommunityDestination,
  saveCommunityDestination,
} from "@/features/communities/communityNavigationStorage";
import type { Channel } from "@/shared/api/types";

type ShellRoute = ReturnType<typeof deriveShellRoute>;
type AppNavigation = ReturnType<typeof useAppNavigation>;

/**
 * Restores the channel a community was last viewed on, once for the first
 * successful channel load after an explicit community transition.
 */
export function useCommunityDestinationRestore({
  activeCommunityId,
  channelsDataUpdatedAt,
  channelsLoaded,
  goChannel,
  goHome,
  selectedView,
  sidebarChannels,
}: {
  activeCommunityId: string | undefined;
  channelsDataUpdatedAt: number;
  channelsLoaded: boolean;
  goChannel: AppNavigation["goChannel"];
  goHome: AppNavigation["goHome"];
  selectedView: ShellRoute["selectedView"];
  sidebarChannels: Channel[];
}) {
  const hasRestoredCommunityDestinationRef = React.useRef(false);
  React.useEffect(() => {
    if (
      hasRestoredCommunityDestinationRef.current ||
      !channelsLoaded ||
      channelsDataUpdatedAt === 0 ||
      !activeCommunityId
    ) {
      return;
    }
    hasRestoredCommunityDestinationRef.current = true;

    // Restoration belongs to an explicit community transition. Cold boot and
    // reconnect remounts must preserve the route the user explicitly opened.
    if (!consumePendingCommunityRestore(activeCommunityId)) {
      return;
    }

    const destination = loadCommunityDestination(activeCommunityId);
    if (!destination || destination.kind === "home") {
      return;
    }

    const channelIsAvailable = sidebarChannels.some(
      (channel) => channel.id === destination.channelId,
    );
    if (!channelIsAvailable) {
      saveCommunityDestination(activeCommunityId, { kind: "home" });
      void goHome({ replace: true });
      return;
    }

    // The normal switch path writes the remembered channel into the hash before
    // the target community mounts, so no intermediate Inbox frame is painted.
    // Older transition callers may still arrive at neutral Home; repair those.
    if (selectedView === "home") {
      void goChannel(destination.channelId, { replace: true });
    }
  }, [
    activeCommunityId,
    channelsDataUpdatedAt,
    channelsLoaded,
    goChannel,
    goHome,
    selectedView,
    sidebarChannels,
  ]);
}
