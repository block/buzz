import { buildMessageLink } from "@/features/messages/lib/messageLink";
import type { TimelineMessage } from "@/features/messages/types";

export function buildBestieMessageContext(
  channelId: string | null | undefined,
  message: TimelineMessage | undefined,
): string | null {
  if (!channelId || !message) return null;

  const threadRootId = message.rootId ?? message.id;
  const threadLink = buildMessageLink({
    channelId,
    messageId: threadRootId,
    threadRootId: message.rootId ? threadRootId : null,
  });

  return `Help me with this thread from ${message.author}:\n\n${threadLink}`;
}
