import * as React from "react";
import { useHuddle } from "@/features/huddle/HuddleContext";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import { mergeMessages } from "@/features/messages/hooks";
import {
  channelWindowThreadSummaries,
  channelWindowHasMore,
  channelWindowHistoryExhausted,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import { reconcileChannelWindowMessages } from "@/features/messages/lib/channelWindowReconciliation";
import { useThreadRepliesForRoots } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";

export function useIsHuddleTranscript(activeChannelId: string | null) {
  const { activeEphemeralChannelId } = useHuddle();
  return (
    huddleWindowChannelId() !== null ||
    (activeChannelId !== null && activeChannelId === activeEphemeralChannelId)
  );
}

type HuddleChannelMessagesOptions = {
  activeChannel: Channel | null;
  isHuddleTranscript: boolean;
  messages: RelayEvent[];
  targetMessageEvents: RelayEvent[];
  windowStore?: ChannelWindowStore;
};

export function useHuddleChannelMessages({
  activeChannel,
  isHuddleTranscript,
  messages,
  targetMessageEvents,
  windowStore,
}: HuddleChannelMessagesOptions) {
  // Query observers may notify separately. Project from the SAME store that
  // supplies the receipt and structural metadata; never pair a new revision
  // with an older channelMessages cache notification.
  const windowMessages = React.useMemo(
    () =>
      windowStore
        ? reconcileChannelWindowMessages(windowStore, messages)
        : messages,
    [messages, windowStore],
  );
  const resolvedChannelMessages = React.useMemo(() => {
    if (!activeChannel || targetMessageEvents.length === 0)
      return windowMessages;
    return targetMessageEvents.reduce(mergeMessages, windowMessages);
  }, [activeChannel, windowMessages, targetMessageEvents]);

  const threadSummaries = React.useMemo(
    () => (windowStore ? channelWindowThreadSummaries(windowStore) : new Map()),
    [windowStore],
  );
  const huddleThreadRootIds = React.useMemo(
    () =>
      isHuddleTranscript
        ? [...threadSummaries.entries()]
            .filter(([, summary]) => summary.descendantCount > 0)
            .map(([rootId]) => rootId)
        : [],
    [isHuddleTranscript, threadSummaries],
  );
  const huddleThreadReplies = useThreadRepliesForRoots(
    activeChannel,
    huddleThreadRootIds,
  );
  const resolvedMessages = React.useMemo(
    () =>
      isHuddleTranscript
        ? huddleThreadReplies.events.reduce(
            mergeMessages,
            resolvedChannelMessages,
          )
        : resolvedChannelMessages,
    [huddleThreadReplies.events, isHuddleTranscript, resolvedChannelMessages],
  );

  return {
    resolvedMessages,
    threadSummaries,
    historyRevision: windowStore?.revision ?? 0,
    hasOlderMessages: windowStore ? channelWindowHasMore(windowStore) : false,
    historyExhausted: windowStore
      ? channelWindowHistoryExhausted(windowStore)
      : false,
    // A summarized reply subtree failing must not leave the transcript reading
    // as complete: surface the aggregate failure so the consumer can show a
    // non-destructive retry alert alongside the rows that did load.
    threadRepliesError: isHuddleTranscript && huddleThreadReplies.isError,
    onRetryThreadReplies: huddleThreadReplies.refetch,
  };
}
