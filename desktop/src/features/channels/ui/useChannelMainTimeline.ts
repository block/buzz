import * as React from "react";

import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import { useInlineAgentActivity } from "@/features/channels/ui/InlineAgentActivity";
import { useChannelPaneMessages } from "@/features/channels/ui/useChannelPaneMessages";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";

export function useChannelMainTimeline({
  activeChannel,
  activityAgents,
  isHuddleTranscript,
  messages,
  onOpenAgentSession,
  profiles,
  threadSummaries,
  workingBotPubkeys,
}: {
  activeChannel: Channel | null;
  activityAgents: BotActivityAgent[];
  isHuddleTranscript: boolean;
  messages: TimelineMessage[];
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  profiles?: UserProfileLookup;
  threadSummaries?: ReadonlyMap<string, ChannelWindowThreadSummary>;
  workingBotPubkeys: string[];
}) {
  const { mainTimelineEntries, visibleMessages } = useChannelPaneMessages({
    activeChannel,
    isHuddleTranscript,
    messages,
    profiles,
    threadSummaries,
  });
  const renderedMessageIds = React.useMemo(
    () => new Set(mainTimelineEntries.map((entry) => entry.message.id)),
    [mainTimelineEntries],
  );
  const inlineAgentActivity = useInlineAgentActivity({
    agents: activityAgents,
    channelId: activeChannel?.id ?? null,
    onOpenAgentSession,
    profiles,
    renderedMessageIds,
    workingBotPubkeys,
  });
  const messageLeadingContent = React.useMemo(
    () =>
      inlineAgentActivity?.anchorMessageId
        ? {
            [inlineAgentActivity.anchorMessageId]: inlineAgentActivity.content,
          }
        : undefined,
    [inlineAgentActivity],
  );

  return {
    mainTimelineEntries,
    messageLeadingContent,
    trailingContent:
      inlineAgentActivity && !inlineAgentActivity.anchorMessageId
        ? inlineAgentActivity.content
        : null,
    visibleMessages,
  };
}
