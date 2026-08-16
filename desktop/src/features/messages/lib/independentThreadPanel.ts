import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { buildThreadPanelData } from "@/features/messages/lib/threadPanel";
import type { RelayEvent } from "@/shared/api/types";

export function buildIndependentThreadPanel(
  channelEvents: RelayEvent[],
  replyEvents: RelayEvent[],
  rootId: string | null,
  replyTargetId: string | null,
  expandedReplyIds: ReadonlySet<string>,
  ...formatArgs: Tail<Parameters<typeof formatTimelineMessages>>
) {
  if (!rootId) {
    return {
      ...buildThreadPanelData([], null, replyTargetId, expandedReplyIds),
      messages: [],
    };
  }
  const head = channelEvents.find((event) => event.id === rootId);
  const events = head ? [head, ...replyEvents] : replyEvents;
  const messages = formatTimelineMessages(events, ...formatArgs);
  return {
    ...buildThreadPanelData(messages, rootId, replyTargetId, expandedReplyIds),
    messages,
  };
}

type Tail<T extends readonly unknown[]> = T extends readonly [
  unknown,
  ...infer R,
]
  ? R
  : never;
