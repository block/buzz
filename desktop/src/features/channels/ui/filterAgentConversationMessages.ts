import * as React from "react";
import {
  buildAgentConversationMarkers,
  getHiddenAgentConversationMessageIds,
  type AgentConversationMarker,
} from "@/features/agents/agentConversations";
import type { TimelineMessage } from "@/features/messages/types";
import type { RelayEvent } from "@/shared/api/types";

function filterHiddenAgentConversationMessages(
  messages: TimelineMessage[],
  markers: readonly AgentConversationMarker[] | undefined,
): TimelineMessage[] {
  const hiddenMessageIds = getHiddenAgentConversationMessageIds(
    messages,
    markers,
  );
  if (hiddenMessageIds.size === 0) {
    return messages;
  }

  return messages.filter((message) => !hiddenMessageIds.has(message.id));
}

export function useUnreadTimelineMessages(
  messages: TimelineMessage[],
  markers: readonly AgentConversationMarker[] | undefined,
): TimelineMessage[] {
  return React.useMemo(
    () => filterHiddenAgentConversationMessages(messages, markers),
    [markers, messages],
  );
}

export function useAgentConversationMarkers(
  messages: RelayEvent[],
  enabled = true,
): AgentConversationMarker[] {
  return React.useMemo(
    () => (enabled ? buildAgentConversationMarkers(messages) : []),
    [enabled, messages],
  );
}

export function useAgentConversationTimelineState(
  events: RelayEvent[],
  messages: TimelineMessage[],
  enabled = true,
) {
  const agentConversationMarkers = useAgentConversationMarkers(events, enabled);
  const unreadTimelineMessages = useUnreadTimelineMessages(
    messages,
    agentConversationMarkers,
  );
  return { agentConversationMarkers, unreadTimelineMessages };
}
