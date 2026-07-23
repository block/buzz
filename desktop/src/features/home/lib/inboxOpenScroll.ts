import type { ScrollTargetAlignment } from "@/features/messages/ui/anchoredScrollTarget";

export type InboxOpenScrollIntent = {
  anchorEventId: string;
  conversationId: string;
  excludedRootId: string | null;
  forcedUnreadMessageId: string | null;
  openReadAt: number | null;
  requestId: number;
  wasUnread: boolean;
};

export type InboxOpenScrollTarget = {
  alignment: ScrollTargetAlignment;
  id: string;
};

/**
 * Resolves the Inbox-owned layout target from the read state captured before
 * opening the row advances its marker. The caller latches this once context
 * loading finishes so live arrivals cannot move the open-time boundary.
 */
export function resolveInboxOpenScrollTarget(
  intent: InboxOpenScrollIntent,
  messages: readonly { createdAt: number; id: string }[],
): InboxOpenScrollTarget | null {
  if (messages.length === 0) return null;

  if (intent.wasUnread) {
    const firstUnread = intent.forcedUnreadMessageId
      ? messages.find((message) => message.id === intent.forcedUnreadMessageId)
      : messages.find(
          (message) =>
            message.id !== intent.excludedRootId &&
            (intent.openReadAt === null ||
              message.createdAt > intent.openReadAt),
        );
    if (firstUnread) {
      return { alignment: "top-with-divider", id: firstUnread.id };
    }
  }

  const lastMessage = messages.at(-1);
  return lastMessage ? { alignment: "bottom", id: lastMessage.id } : null;
}
