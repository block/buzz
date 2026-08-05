import { getSentMessageLink } from "@/features/agents/ui/AgentSessionToolItem/messageLinks";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

export type InlineAgentActivityPlacement = {
  anchorMessageId: string | null;
  items: TranscriptItem[];
};

function findLastMatching<T>(
  items: readonly T[],
  predicate: (item: T) => boolean,
): T | undefined {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item !== undefined && predicate(item)) {
      return item;
    }
  }
  return undefined;
}

function isSameTurn(
  item: TranscriptItem,
  turnId: string | null,
  sessionId: string | null,
) {
  if (turnId) {
    return item.turnId === turnId;
  }
  return Boolean(sessionId) && item.sessionId === sessionId;
}

function isInlineActivityItem(item: TranscriptItem) {
  return (
    item.type !== "message" &&
    item.renderClass !== "raw-rail" &&
    item.renderClass !== "suppressed"
  );
}

/**
 * Select the newest channel turn and anchor it before the message published by
 * send_message. Before that message reaches the timeline, the caller renders
 * the same trace at the live tail instead.
 */
export function buildInlineAgentActivityPlacement({
  channelId,
  isWorking,
  renderedMessageIds,
  transcript,
}: {
  channelId: string;
  isWorking: boolean;
  renderedMessageIds: ReadonlySet<string>;
  transcript: readonly TranscriptItem[];
}): InlineAgentActivityPlacement | null {
  const latestTurnItem = findLastMatching(
    transcript,
    (item) =>
      item.channelId === channelId && Boolean(item.turnId ?? item.sessionId),
  );
  if (!latestTurnItem) {
    return null;
  }

  const items = transcript.filter(
    (item) =>
      item.channelId === channelId &&
      isSameTurn(
        item,
        latestTurnItem.turnId ?? null,
        latestTurnItem.sessionId ?? null,
      ) &&
      isInlineActivityItem(item),
  );
  if (items.length === 0) {
    return null;
  }

  const sentMessage = findLastMatching(
    items,
    (item) => item.type === "tool" && getSentMessageLink(item) !== null,
  );
  const sentMessageLink =
    sentMessage?.type === "tool" ? getSentMessageLink(sentMessage) : null;
  const anchorMessageId =
    sentMessageLink?.channelId === channelId &&
    renderedMessageIds.has(sentMessageLink.messageId)
      ? sentMessageLink.messageId
      : null;

  if (!isWorking && !anchorMessageId) {
    return null;
  }

  return { anchorMessageId, items };
}
