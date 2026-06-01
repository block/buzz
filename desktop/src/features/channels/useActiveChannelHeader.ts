import * as React from "react";

import { useEphemeralChannelDisplay } from "@/features/channels/useEphemeralChannelDisplay";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import type { Channel, PresenceStatus } from "@/shared/api/types";

export function useActiveChannelHeader(
  activeChannel: Channel | null,
  currentPubkey?: string,
) {
  const activeDmParticipantPubkeys = React.useMemo(() => {
    if (activeChannel?.channelType !== "dm") {
      return [];
    }

    return activeChannel.participantPubkeys.filter(
      (pubkey) => pubkey.toLowerCase() !== currentPubkey?.toLowerCase(),
    );
  }, [activeChannel, currentPubkey]);
  const activeDmPresenceQuery = usePresenceQuery(activeDmParticipantPubkeys, {
    enabled: activeDmParticipantPubkeys.length > 0,
  });
  const activeDmProfilesQuery = useUsersBatchQuery(activeDmParticipantPubkeys, {
    enabled: activeDmParticipantPubkeys.length > 0,
  });
  const activeChannelEphemeralDisplay =
    useEphemeralChannelDisplay(activeChannel);
  const activeDmPresenceStatus: PresenceStatus | null =
    activeDmParticipantPubkeys.length > 0
      ? (activeDmPresenceQuery.data?.[
          activeDmParticipantPubkeys[0]?.toLowerCase()
        ] ?? null)
      : null;

  return {
    activeChannelTitle: activeChannel
      ? resolveChannelDisplayLabel(
          activeChannel,
          currentPubkey,
          activeDmProfilesQuery.data?.profiles,
        )
      : "Channels",
    activeDmPresenceStatus,
    activeChannelEphemeralDisplay,
  };
}
