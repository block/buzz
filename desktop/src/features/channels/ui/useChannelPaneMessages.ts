import * as React from "react";
import {
  isChannelCreatedSystemMessage,
  isWelcomeSetupSystemMessage,
} from "@/features/channels/ui/ChannelPane.helpers";
import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";
import {
  buildMainTimelineEntries,
  type MainTimelineEntry,
} from "@/features/messages/lib/threadPanel";
import { isWelcomeExperienceChannel } from "@/features/onboarding/welcome";

type ChannelPaneMessagesOptions = Pick<
  ChannelPaneProps,
  "activeChannel" | "messages" | "profiles" | "threadSummaries"
> & {
  inlineThreadHeadId?: string | null;
  inlineThreadMessages?: MainTimelineEntry[];
  inlineThreadMessagesPending?: boolean;
  isHuddleTranscript: boolean;
};

export function useChannelPaneMessages({
  activeChannel,
  inlineThreadHeadId = null,
  inlineThreadMessages = [],
  inlineThreadMessagesPending = false,
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

  const mainTimelineEntries = React.useMemo(() => {
    const entries = isHuddleTranscript
      ? visibleMessages.map((message) => ({ message, summary: null }))
      : buildMainTimelineEntries(
          visibleMessages,
          new Set(),
          threadSummaries,
          profiles,
        );

    if (isHuddleTranscript || !inlineThreadHeadId) {
      return entries;
    }

    return entries.map((entry) =>
      entry.message.id === inlineThreadHeadId
        ? {
            ...entry,
            inlineThread: {
              isPending: inlineThreadMessagesPending,
              replies: inlineThreadMessages,
            },
          }
        : entry,
    );
  }, [
    inlineThreadHeadId,
    inlineThreadMessages,
    inlineThreadMessagesPending,
    isHuddleTranscript,
    profiles,
    threadSummaries,
    visibleMessages,
  ]);

  return { mainTimelineEntries, visibleMessages };
}
