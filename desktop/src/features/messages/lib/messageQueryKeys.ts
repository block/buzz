import type { RelayEvent } from "@/shared/api/types";
import { compareChronologicalOrder } from "@/shared/lib/eventOrder";

export function channelMessagesKey(channelId: string) {
  return ["channel-messages", channelId] as const;
}

export function channelWindowKey(channelId: string) {
  return ["channel-window", channelId] as const;
}

export function threadRepliesKey(channelId: string, rootId: string) {
  return ["thread-replies", channelId, rootId] as const;
}

export function dedupeMessagesById(messages: RelayEvent[]) {
  const seenIds = new Set<string>();
  const deduped: RelayEvent[] = [];

  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];

    if (seenIds.has(message.id)) {
      continue;
    }

    seenIds.add(message.id);
    deduped.push(message);
  }

  return deduped.reverse();
}

export function sortMessages(messages: RelayEvent[]) {
  // The channel window projects the same timeline, so same-second events
  // must tiebreak exactly as it does, whichever REQ delivered them first.
  return dedupeMessagesById(messages).sort(compareChronologicalOrder);
}

export function normalizeTimelineMessages(messages: RelayEvent[]) {
  return sortMessages(messages);
}

function isOlderHistoryPage(current: RelayEvent[], history: RelayEvent[]) {
  if (current.length === 0 || history.length === 0) {
    return false;
  }

  const sortedCurrent = sortMessages(current);
  const sortedHistory = sortMessages(history);
  const newestHistory = sortedHistory[sortedHistory.length - 1]?.created_at;
  const oldestCurrent = sortedCurrent[0]?.created_at;

  if (newestHistory === undefined || oldestCurrent === undefined) {
    return false;
  }

  return newestHistory <= oldestCurrent;
}

function normalizeTimelineHistoryMessages(
  current: RelayEvent[],
  history: RelayEvent[],
) {
  return sortMessages([...current, ...history]);
}

export function mergeTimelineHistoryMessages(
  current: RelayEvent[],
  history: RelayEvent[],
) {
  if (isOlderHistoryPage(current, history)) {
    return normalizeTimelineHistoryMessages(current, history);
  }

  return normalizeTimelineMessages([...current, ...history]);
}
