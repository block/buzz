import * as React from "react";

import type { deriveShellRoute } from "@/app/AppShell.helpers";
import type { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  loadCommunityDestination,
  markPendingCommunityRestore,
  saveCommunityDestination,
} from "@/features/communities/communityNavigationStorage";
import type { useCommunities } from "@/features/communities/useCommunities";

type Communities = ReturnType<typeof useCommunities>;
type ShellRoute = ReturnType<typeof deriveShellRoute>;
type GoHome = ReturnType<typeof useAppNavigation>["goHome"];

export function useCommunityNavigationTransitions({
  communities,
  goHome,
  selectedChannelId,
  selectedView,
}: {
  communities: Communities;
  goHome: GoHome;
  selectedChannelId: ShellRoute["selectedChannelId"];
  selectedView: ShellRoute["selectedView"];
}) {
  const saveActiveDestination = React.useCallback(() => {
    const activeCommunityId = communities.activeCommunity?.id;
    if (!activeCommunityId) return;
    saveCommunityDestination(
      activeCommunityId,
      selectedView === "channel" && selectedChannelId
        ? { kind: "channel", channelId: selectedChannelId }
        : { kind: "home" },
    );
  }, [communities.activeCommunity?.id, selectedChannelId, selectedView]);

  // Home is a teardown barrier: the outgoing channel must unmount before the
  // relay changes, or its read effect can advance markers on the wrong relay.
  const switchCommunity = React.useCallback(
    async (id: string) => {
      const activeCommunityId = communities.activeCommunity?.id;
      if (id === activeCommunityId) return;
      if (!activeCommunityId) {
        communities.switchCommunity(id);
        return;
      }

      saveActiveDestination();
      await goHome({ replace: true });
      markPendingCommunityRestore(id);
      const destination = loadCommunityDestination(id);
      if (destination?.kind === "channel") {
        window.history.replaceState(
          window.history.state,
          "",
          `#/channels/${encodeURIComponent(destination.channelId)}`,
        );
      }
      communities.switchCommunity(id);
    },
    [communities, goHome, saveActiveDestination],
  );

  const removeCommunity = React.useCallback(
    async (id: string) => {
      if (id !== communities.activeCommunity?.id) {
        communities.removeCommunity(id);
        return;
      }
      const fallback = communities.communities.find(
        (community) => community.id !== id,
      );
      if (!fallback) return;

      saveActiveDestination();
      await goHome({ replace: true });
      markPendingCommunityRestore(fallback.id);
      const destination = loadCommunityDestination(fallback.id);
      if (destination?.kind === "channel") {
        window.history.replaceState(
          window.history.state,
          "",
          `#/channels/${encodeURIComponent(destination.channelId)}`,
        );
      }
      communities.removeCommunity(id);
    },
    [communities, goHome, saveActiveDestination],
  );

  return { removeCommunity, switchCommunity };
}
