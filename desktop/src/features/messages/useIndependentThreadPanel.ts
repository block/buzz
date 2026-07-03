import * as React from "react";

import { buildIndependentThreadPanel } from "@/features/messages/lib/independentThreadPanel";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
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
  const replies = useThreadReplies(args.activeChannel, args.rootId);
  return React.useMemo(
    () =>
      buildIndependentThreadPanel(
        args.channelEvents,
        replies.data ?? [],
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
    [args, replies.data],
  );
}
