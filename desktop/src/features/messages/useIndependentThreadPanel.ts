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
  ownerProfiles: UserProfileLookup | undefined;
  members: ChannelMember[] | undefined;
  personaLookup: Map<string, string>;
  respondToLookup: Map<string, RespondToMode>;
  relaySelfPubkey: string | null | undefined;
}) {
  // Depend on the individual fields, NOT the `args` object — callers pass a
  // fresh object literal every render, so `[args]` never memoizes and the
  // full O(replies) formatTimelineMessages + buildThreadPanelData rebuild
  // runs on every ChannelScreen render. In a long thread with agents
  // streaming (an event or typing tick per ~150ms, each re-rendering the
  // screen) that saturates the main thread and starves keystrokes — see
  // typing-latency.perf.ts scenario "thread68+". Mirrors the main timeline's
  // memoization of the same formatter (ChannelScreen `timelineMessages`).
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
        args.relaySelfPubkey,
        args.ownerProfiles,
      ),
    [
      args.channelEvents,
      args.threadReplyEvents,
      args.rootId,
      args.replyTargetId,
      args.expandedReplyIds,
      args.activeChannel,
      args.currentPubkey,
      args.currentAvatarUrl,
      args.profiles,
      args.ownerProfiles,
      args.members,
      args.personaLookup,
      args.respondToLookup,
      args.relaySelfPubkey,
    ],
  );
}
