import { useRouter } from "@tanstack/react-router";
import * as React from "react";

import type { deriveShellRoute } from "@/app/AppShell.helpers";
import type { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  replaceCommunityDestinationRoute,
  runCommunityViewTransition,
} from "@/app/communityViewTransition";
import {
  loadCommunityDestination,
  markPendingCommunityRestore,
  saveCommunityDestination,
} from "@/features/communities/communityNavigationStorage";
import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
import {
  markRelayRemoved,
  refuseRelayAdmission,
  stopManagedAgentPairsOnRelay,
} from "@/features/agents/managedAgentRelayCleanup";
import { markCommunityDiscoveryAfterLeave } from "@/features/communities/communityStorage";
import type { useCommunities } from "@/features/communities/useCommunities";
import { leaveCommunity } from "@/features/communities/leaveCommunity";

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
  const router = useRouter();
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

      await runCommunityViewTransition(async () => {
        saveActiveDestination();
        await goHome({ replace: true });
        markPendingCommunityRestore(id);
        const destination = loadCommunityDestination(id);
        if (destination?.kind === "channel") {
          replaceCommunityDestinationRoute(
            destination.channelId,
            router.history,
          );
        }
        communities.switchCommunity(id);
      });
    },
    [communities, goHome, router.history, saveActiveDestination],
  );

  // Local-only cleanup shared by Leave and "Remove from this device". It never
  // contacts the relay, so it still works when the relay is gone. Once the
  // community is gone, its relay's agent pairs are stopped so none keep
  // reconnecting to it.
  const removeCommunityFromDevice = React.useCallback(
    async (id: string) => {
      const target = communities.communities.find(
        (community) => community.id === id,
      );
      if (!target) return;
      // Another community can point at the same relay; its pairs stay up.
      const relayStillUsed = communities.communities.some(
        (community) =>
          community.id !== id &&
          canonicalRelayUrl(community.relayUrl) ===
            canonicalRelayUrl(target.relayUrl),
      );
      // Fences any reconcile in flight for this relay; see markRelayRemoved.
      const markRemoved = () => {
        if (!relayStillUsed) markRelayRemoved(target.relayUrl);
      };
      const stopRelayPairs = () => {
        if (!relayStillUsed) void stopManagedAgentPairsOnRelay(target.relayUrl);
      };
      // Refuses every local pair start on the relay in Rust, including a
      // launch restore or start already in flight; pairs registered first are
      // caught by the stop. Relay routing is left untouched.
      const refuseRelay = async () => {
        if (!relayStillUsed) await refuseRelayAdmission(target.relayUrl);
      };

      if (id !== communities.activeCommunity?.id) {
        markRemoved();
        await refuseRelay();
        communities.removeCommunity(id);
        stopRelayPairs();
        return;
      }

      const fallback = communities.communities.find(
        (community) => community.id !== id,
      );
      if (!fallback) {
        if (!markCommunityDiscoveryAfterLeave()) {
          throw new Error(
            "Couldn't finish removing the community from this device because community discovery state could not be saved. Restart Buzz and try again.",
          );
        }
        markRemoved();
        await refuseRelay();
        await goHome({ replace: true });
        communities.removeCommunity(id);
        stopRelayPairs();
        return;
      }

      markRemoved();
      await refuseRelay();
      await runCommunityViewTransition(async () => {
        saveActiveDestination();
        await goHome({ replace: true });
        markPendingCommunityRestore(fallback.id);
        const destination = loadCommunityDestination(fallback.id);
        if (destination?.kind === "channel") {
          replaceCommunityDestinationRoute(
            destination.channelId,
            router.history,
          );
        }
        communities.removeCommunity(id);
      });
      stopRelayPairs();
    },
    [communities, goHome, router.history, saveActiveDestination],
  );

  const leaveAndRemoveCommunity = React.useCallback(
    async (id: string) => {
      const target = communities.communities.find(
        (community) => community.id === id,
      );
      if (!target) return;

      // Do not touch local state until this relay has explicitly accepted the
      // signed NIP-43 leave request. Rejections and timeouts bubble back to the
      // menu so the person can retry without losing their community config.
      const leaveResult = await leaveCommunity(
        target.relayUrl,
        communities.activeCommunity?.relayUrl,
      );
      await removeCommunityFromDevice(id);
      return leaveResult;
    },
    [communities, removeCommunityFromDevice],
  );

  return {
    leaveAndRemoveCommunity,
    removeCommunityFromDevice,
    switchCommunity,
  };
}
