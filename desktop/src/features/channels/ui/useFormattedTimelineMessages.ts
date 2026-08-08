import * as React from "react";

import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  Channel,
  ChannelMember,
  RelayEvent,
  RespondToMode,
} from "@/shared/api/types";

/**
 * Formats channel events into timeline rows. Kept as a dedicated hook so
 * ChannelScreen stays under the file-size ratchet while the format deps stay
 * explicit (not a fresh `args` object each render).
 */
export function useFormattedTimelineMessages(args: {
  events: RelayEvent[];
  channel: Channel | null;
  currentPubkey: string | undefined;
  currentAvatarUrl: string | null;
  profiles: UserProfileLookup | undefined;
  members: ChannelMember[] | undefined;
  personaLookup: Map<string, string>;
  respondToLookup: Map<string, RespondToMode>;
  relaySelfPubkey: string | null | undefined;
  ownerProfiles: UserProfileLookup | undefined;
}): TimelineMessage[] {
  return React.useMemo(
    () =>
      formatTimelineMessages(
        args.events,
        args.channel,
        args.currentPubkey,
        args.currentAvatarUrl,
        args.profiles,
        args.members,
        args.personaLookup,
        args.respondToLookup,
        args.relaySelfPubkey,
        args.ownerProfiles,
      ),
    [
      args.events,
      args.channel,
      args.currentPubkey,
      args.currentAvatarUrl,
      args.profiles,
      args.members,
      args.personaLookup,
      args.respondToLookup,
      args.relaySelfPubkey,
      args.ownerProfiles,
    ],
  );
}
