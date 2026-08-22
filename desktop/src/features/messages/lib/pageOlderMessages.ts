import type { QueryClient } from "@tanstack/react-query";

import {
  appendOlderChannelWindow,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import { projectChannelWindowMessages } from "@/features/messages/lib/projectChannelWindow";
import { parseChannelWindowResponse } from "@/features/messages/lib/channelWindowResponse";
import { channelWindowKey } from "@/features/messages/lib/messageQueryKeys";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";

const CHANNEL_WINDOW_PAGE_SIZE = 50;
export type PageOlderResult = { hasOlderMessages: boolean };
const inFlightPasses = new Map<string, Promise<PageOlderResult>>();

/** Fetch exactly one server-defined older window and append it atomically. */
export function pageOlderMessagesUntilRowFloor(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
): Promise<PageOlderResult> {
  const running = inFlightPasses.get(channelId);
  if (running) return running;
  const pass = runPage(queryClient, channelId, shouldContinue).finally(() => {
    inFlightPasses.delete(channelId);
  });
  inFlightPasses.set(channelId, pass);
  return pass;
}

async function runPage(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
): Promise<PageOlderResult> {
  const store = queryClient.getQueryData<ChannelWindowStore>(
    channelWindowKey(channelId),
  );
  const tail = store?.pages[store.pages.length - 1];
  if (!store || !tail?.hasMore || !tail.nextCursor || !shouldContinue()) {
    return { hasOlderMessages: false };
  }

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
  return { hasOlderMessages: page.hasMore };
}
