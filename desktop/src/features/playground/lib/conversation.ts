import type { AppView } from "@/app/AppShell.helpers";

export type PlaygroundConversation = {
  channelId: string;
  draftKey: string;
};

/**
 * Screenshot is only offered when a channel or thread composer is underneath
 * the overlay. Pinned sites, Agents, Inbox-without-a-channel, and the other
 * primary views have no conversation to stage into.
 */
export function playgroundConversationFromRoute(input: {
  selectedView: AppView;
  selectedChannelId: string | null;
  threadId?: string | null;
}): PlaygroundConversation | null {
  if (input.selectedView !== "channel") {
    return null;
  }
  const channelId = input.selectedChannelId?.trim() ?? "";
  if (!channelId) {
    return null;
  }
  const threadId = input.threadId?.trim() ?? "";
  return {
    channelId,
    draftKey: threadId ? `thread:${threadId}` : channelId,
  };
}

export function playgroundScreenshotAvailable(
  conversation: PlaygroundConversation | null,
): boolean {
  return conversation != null;
}

/** True when the overlay sits on a channel with a split thread pane open. */
export function playgroundConversationHasOpenThread(
  conversation: PlaygroundConversation | null,
): boolean {
  return Boolean(conversation?.draftKey.startsWith("thread:"));
}

/**
 * Scope key for header playground pins. Channel view → that channel;
 * thread view → that thread only (not the parent channel).
 */
export function playgroundPinScopeKey(
  conversation: PlaygroundConversation,
): string {
  return conversation.draftKey.startsWith("thread:")
    ? conversation.draftKey
    : `channel:${conversation.channelId}`;
}
