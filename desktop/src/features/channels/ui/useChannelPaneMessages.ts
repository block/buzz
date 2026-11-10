import * as React from "react";
import {
  isChannelCreatedSystemMessage,
  isWelcomeSetupSystemMessage,
} from "@/features/channels/ui/ChannelPane.helpers";
import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import { getRecentMentionPubkeys } from "@/features/messages/lib/recentMentionPubkeys";
import { isWelcomeExperienceChannel } from "@/features/onboarding/welcome";

type ChannelPaneMessagesOptions = Pick<
  ChannelPaneProps,
  "activeChannel" | "messages" | "profiles" | "threadSummaries"
> & {
  isHuddleTranscript: boolean;
};

export function useChannelPaneMessages({
  activeChannel,
  isHuddleTranscript,
  messages,
  profiles,
  threadSummaries,
}: ChannelPaneMessagesOptions) {
  const visibleMessages = React.useMemo(() => {
    const withoutWelcomeSetup = isWelcomeExperienceChannel(activeChannel)
      ? messages.filter((message) => !isWelcomeSetupSystemMessage(message))
      : messages;

    return isHuddleTranscript
      ? withoutWelcomeSetup.filter(
          (message) => !isChannelCreatedSystemMessage(message),
        )
      : withoutWelcomeSetup;
  }, [activeChannel, isHuddleTranscript, messages]);

  const mainTimelineEntries = React.useMemo(
    () =>
      isHuddleTranscript
        ? visibleMessages.map((message) => ({ message, summary: null }))
        : buildMainTimelineEntries(
            visibleMessages,
            new Set(),
            threadSummaries,
            profiles,
          ),
    [isHuddleTranscript, profiles, threadSummaries, visibleMessages],
  );

  const recentMentionPubkeys = React.useMemo(
    () => getRecentMentionPubkeys(messages, activeChannel?.channelType),
    [activeChannel?.channelType, messages],
  );

  return {
    mainTimelineEntries,
    recentMentions: recentMentionPubkeys,
    visibleMessages,
  };
}
