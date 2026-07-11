import { notifyManager, type QueryClient } from "@tanstack/react-query";

import {
  appendOlderChannelWindow,
  flattenChannelWindowEvents,
  type ChannelWindowCursor,
  type ChannelWindowPage,
  type ChannelWindowStore,
} from "@/features/messages/lib/channelWindowStore";
import { parseChannelWindowResponse } from "@/features/messages/lib/channelWindowResponse";
import {
  channelMessagesKey,
  channelWindowKey,
} from "@/features/messages/lib/messageQueryKeys";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";
import type { RelayEvent } from "@/shared/api/types";

const CHANNEL_WINDOW_PAGE_SIZE = 50;
export type PageOlderResult = { hasOlderMessages: boolean };
const inFlightPasses = new Map<string, Promise<PageOlderResult>>();

type FetchOlderPage = (
  channelId: string,
  cursor: ChannelWindowCursor,
) => Promise<ChannelWindowPage>;

const fetchOlderPage: FetchOlderPage = async (channelId, cursor) => {
  const events = await getChannelWindowEvents(
    channelId,
    cursor,
    CHANNEL_WINDOW_PAGE_SIZE,
  );
  return parseChannelWindowResponse(events, channelId, cursor);
};

type StagedPage = {
  cursor: ChannelWindowCursor;
  promise: Promise<ChannelWindowPage | null>;
};
const stagedPages = new Map<string, StagedPage>();

function cursorsEqual(
  left: ChannelWindowCursor | null | undefined,
  right: ChannelWindowCursor | null | undefined,
) {
  return (
    left === right ||
    (left != null &&
      right != null &&
      left.createdAt === right.createdAt &&
      left.eventId === right.eventId)
  );
}

/**
 * Fetch one page without publishing it to React Query or the DOM. The exact
 * cursor is retained with the response so a later admission cannot append it
 * to a replaced or advanced window chain.
 */
export function stageOlderMessages(
  queryClient: QueryClient,
  channelId: string,
  fetchPage: FetchOlderPage = fetchOlderPage,
): Promise<void> {
  const store = queryClient.getQueryData<ChannelWindowStore>(
    channelWindowKey(channelId),
  );
  const tail = store?.pages.at(-1);
  const cursor = tail?.hasMore ? tail.nextCursor : null;
  if (!cursor) {
    stagedPages.delete(channelId);
    return Promise.resolve();
  }

  const retained = stagedPages.get(channelId);
  if (retained && cursorsEqual(retained.cursor, cursor)) {
    return retained.promise.then(() => undefined);
  }

  const entry: StagedPage = {
    cursor,
    promise: fetchPage(channelId, cursor).catch(() => null),
  };
  stagedPages.set(channelId, entry);
  return entry.promise.then(() => undefined);
}

/** Drop speculative state when its channel is no longer the active consumer. */
export function discardStagedOlderMessages(channelId: string) {
  stagedPages.delete(channelId);
}

async function consumeStagedPage(
  channelId: string,
  cursor: ChannelWindowCursor,
) {
  const entry = stagedPages.get(channelId);
  if (!entry || !cursorsEqual(entry.cursor, cursor)) return null;
  const page = await entry.promise;
  if (stagedPages.get(channelId) === entry) stagedPages.delete(channelId);
  return page;
}

/**
 * Publish a page only if its request cursor still exactly matches the retained
 * tail. Both source-of-truth caches notify observers as one batched commit.
 */
export function admitOlderPage(
  queryClient: QueryClient,
  channelId: string,
  requestCursor: ChannelWindowCursor,
  page: ChannelWindowPage,
) {
  const retained = queryClient.getQueryData<ChannelWindowStore>(
    channelWindowKey(channelId),
  );
  const tail = retained?.pages.at(-1);
  if (
    !retained ||
    !tail?.hasMore ||
    !cursorsEqual(tail.nextCursor, requestCursor)
  ) {
    return false;
  }

  const next = appendOlderChannelWindow(retained, page);
  notifyManager.batch(() => {
    queryClient.setQueryData(channelWindowKey(channelId), next);
    queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(channelId),
      flattenChannelWindowEvents(next),
    );
  });
  return true;
}

/** Fetch exactly one server-defined older window and append it atomically. */
export function pageOlderMessagesUntilRowFloor(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
  fetchPage: FetchOlderPage = fetchOlderPage,
): Promise<PageOlderResult> {
  const running = inFlightPasses.get(channelId);
  if (running) return running;
  const pass = runPage(
    queryClient,
    channelId,
    shouldContinue,
    fetchPage,
  ).finally(() => {
    inFlightPasses.delete(channelId);
  });
  inFlightPasses.set(channelId, pass);
  return pass;
}

async function runPage(
  queryClient: QueryClient,
  channelId: string,
  shouldContinue: () => boolean,
  fetchPage: FetchOlderPage,
): Promise<PageOlderResult> {
  const store = queryClient.getQueryData<ChannelWindowStore>(
    channelWindowKey(channelId),
  );
  const tail = store?.pages.at(-1);
  if (!store || !tail?.hasMore || !tail.nextCursor || !shouldContinue()) {
    return { hasOlderMessages: false };
  }

  const requestCursor = tail.nextCursor;
  let page = await consumeStagedPage(channelId, requestCursor);
  if (!page) {
    page = await fetchPage(channelId, requestCursor);
  }
  if (!shouldContinue()) return { hasOlderMessages: true };

  // A head refresh, another pager, or any cursor-chain replacement may have
  // landed while the staged/network request was pending. Never speculate past
  // the exact tail that originated this response.
  if (!admitOlderPage(queryClient, channelId, requestCursor, page)) {
    return { hasOlderMessages: true };
  }
  return { hasOlderMessages: page.hasMore };
}
