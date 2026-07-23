import type { RelayEvent, ThreadCursor } from "@/shared/api/types";
import { getThreadReplies } from "@/shared/api/tauri";
import {
  collectMessageIdsForAuxBackfill,
  fetchStructuralAuxForMessages,
} from "@/features/messages/lib/auxBackfill";
import {
  channelWindowThreadSummaries,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import { isTimelineContentEvent } from "@/features/messages/lib/formatTimelineMessages";
import {
  dedupeMessagesById,
  sortMessages,
} from "@/features/messages/lib/messageQueryKeys";

const THREAD_PAGE_LIMIT = 200;
const MAX_THREAD_PAGES = 50;

type FlattenTimelineFetchDeps = {
  getThreadReplies: typeof getThreadReplies;
  fetchStructuralAuxForMessages: typeof fetchStructuralAuxForMessages;
};

const defaultFetchDeps: FlattenTimelineFetchDeps = {
  getThreadReplies,
  fetchStructuralAuxForMessages,
};

/**
 * Fetch reply bodies for roots that the channel window only surfaces as
 * kind:39005 summaries. Used so private/DM timelines can render those replies
 * inline without requiring `["broadcast","1"]` on the wire.
 */
export async function fetchFlattenTimelineReplies(
  channelId: string,
  rootIds: readonly string[],
  deps: FlattenTimelineFetchDeps = defaultFetchDeps,
): Promise<RelayEvent[]> {
  const replies: RelayEvent[] = [];
  for (const rootId of rootIds) {
    let cursor: ThreadCursor | null = null;
    for (let page = 0; page < MAX_THREAD_PAGES; page += 1) {
      const response = await deps.getThreadReplies(rootId, channelId, {
        limit: THREAD_PAGE_LIMIT,
        cursor,
      });
      for (const event of response.events) {
        if (isTimelineContentEvent(event)) {
          replies.push(event);
        }
      }
      if (!response.nextCursor) break;
      cursor = response.nextCursor;
    }
  }

  const messageIds = [
    ...new Set([...rootIds, ...collectMessageIdsForAuxBackfill(replies)]),
  ];
  let structuralAux: RelayEvent[] = [];
  try {
    structuralAux = await deps.fetchStructuralAuxForMessages(
      channelId,
      messageIds,
    );
  } catch (error) {
    console.error(
      "Failed to backfill flattened timeline structural aux for channel",
      channelId,
      error,
    );
  }
  return sortMessages([...replies, ...structuralAux]);
}

/** Roots that have a relay thread summary and therefore hidden reply bodies. */
export function flattenTimelineRootIds(store: ChannelWindowStore): string[] {
  return [...channelWindowThreadSummaries(store).keys()];
}

/**
 * Merge hydrated reply events into the reconciled message list. Reply tags are
 * preserved; callers decide whether to render them via flattenReplies.
 */
export function mergeFlattenTimelineReplies(
  messages: RelayEvent[],
  replies: RelayEvent[],
): RelayEvent[] {
  if (replies.length === 0) return messages;
  return sortMessages(dedupeMessagesById([...messages, ...replies]));
}
