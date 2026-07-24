import type { QueryClient } from "@tanstack/react-query";

import {
  appendOlderChannelWindow,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import { projectChannelWindowMessages } from "@/features/messages/lib/projectChannelWindow";
import { parseChannelWindowResponse } from "@/features/messages/lib/channelWindowResponse";
import {
  channelMessagesKey,
  channelWindowKey,
} from "@/features/messages/lib/messageQueryKeys";
import {
  fetchFlattenTimelineReplies,
  flattenTimelineRootIds,
  mergeFlattenTimelineReplies,
} from "@/features/messages/lib/flattenChannelTimeline";
import { shouldFlattenChannelTimeline } from "@/features/messages/lib/threading";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";
import type { Channel, RelayEvent } from "@/shared/api/types";

const CHANNEL_WINDOW_PAGE_SIZE = 50;
export type PageOlderResult = { hasOlderMessages: boolean };
const inFlightPasses = new Map<string, Promise<PageOlderResult>>();

/** Fetch exactly one server-defined older window and append it atomically. */
export function pageOlderMessagesUntilRowFloor(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
  channel?: Channel | null,
): Promise<PageOlderResult> {
  const running = inFlightPasses.get(channelId);
  if (running) return running;
  const pass = runPage(queryClient, channelId, shouldContinue, channel).finally(
    () => {
      inFlightPasses.delete(channelId);
    },
  );
  inFlightPasses.set(channelId, pass);
  return pass;
}

async function hydrateFlattenedOlderReplies(
  queryClient: QueryClient,
  channel: Channel,
  store: ChannelWindowStore,
  knownRootIds: ReadonlySet<string>,
) {
  const rootIds = flattenTimelineRootIds(store).filter(
    (rootId) => !knownRootIds.has(rootId),
  );
  if (rootIds.length === 0) return;
  try {
    const replies = await fetchFlattenTimelineReplies(channel.id, rootIds);
    if (replies.length === 0) return;
    queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(channel.id),
      (messages = []) => mergeFlattenTimelineReplies(messages, replies),
    );
  } catch (error) {
    console.error(
      "Failed to hydrate flattened older timeline replies for channel",
      channel.id,
      error,
    );
  }
}

async function runPage(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
  channel?: Channel | null,
): Promise<PageOlderResult> {
  const store = queryClient.getQueryData<ChannelWindowStore>(
    channelWindowKey(channelId),
  );
  const tail = store?.pages[store.pages.length - 1];
  if (!store || !tail?.hasMore || !tail.nextCursor || !shouldContinue()) {
    return { hasOlderMessages: false };
  }

  const knownRootIds = new Set(flattenTimelineRootIds(store));
  const requestCursor = tail.nextCursor;
  const events = await getChannelWindowEvents(
    channelId,
    requestCursor,
    CHANNEL_WINDOW_PAGE_SIZE,
  );
  if (!shouldContinue()) return { hasOlderMessages: true };
  const page = parseChannelWindowResponse(events, channelId, requestCursor);
  const retained = queryClient.getQueryData<ChannelWindowStore>(
    channelWindowKey(channelId),
  );
  if (!retained) return { hasOlderMessages: true };
  const next = appendOlderChannelWindow(retained, page);
  queryClient.setQueryData(channelWindowKey(channelId), next);
  projectChannelWindowMessages(queryClient, channelId);
  if (shouldFlattenChannelTimeline(channel ?? null) && channel) {
    await hydrateFlattenedOlderReplies(
      queryClient,
      channel,
      next,
      knownRootIds,
    );
  }
  return { hasOlderMessages: page.hasMore };
}
