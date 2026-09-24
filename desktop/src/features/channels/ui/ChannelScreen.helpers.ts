import { getThreadReference } from "@/features/messages/lib/threading";
import type { RelayEvent } from "@/shared/api/types";

export function getLatestActiveChannelMessage(
  messages: RelayEvent[] | undefined,
  threadRepliesInChannel: boolean,
) {
  if (!messages) return null;
  if (threadRepliesInChannel) return messages[messages.length - 1] ?? null;
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (getThreadReference(messages[index].tags ?? []).parentId === null) {
      return messages[index];
    }
  }
  return null;
}
