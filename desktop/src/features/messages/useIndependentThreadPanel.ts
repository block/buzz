import * as React from "react";

import { buildIndependentThreadPanel } from "@/features/messages/lib/independentThreadPanel";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  Channel,
  ChannelMember,
  RelayEvent,
  RespondToMode,
} from "@/shared/api/types";

export function useIndependentThreadPanel(args: {
  activeChannel: Channel | null;
  channelEvents: RelayEvent[];
  threadReplyEvents: RelayEvent[];
  rootId: string | null;
  replyTargetId: string | null;
  expandedReplyIds: ReadonlySet<string>;
  currentPubkey: string | undefined;
  currentAvatarUrl: string | null;
  profiles: UserProfileLookup | undefined;
  members: ChannelMember[] | undefined;
  personaLookup: Map<string, string>;
  respondToLookup: Map<string, RespondToMode>;
}) {
  return React.useMemo(
    () =>
      buildIndependentThreadPanel(
        args.channelEvents,
        args.threadReplyEvents,
        args.rootId,
        args.replyTargetId,
        args.expandedReplyIds,
        args.activeChannel,
        args.currentPubkey,
        args.currentAvatarUrl,
        args.profiles,
        args.members,
        args.personaLookup,
        args.respondToLookup,
      ),
    [args],
  );
}
