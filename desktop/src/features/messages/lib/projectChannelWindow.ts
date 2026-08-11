import type { QueryClient } from "@tanstack/react-query";

import type { RelayEvent } from "@/shared/api/types";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys";
import {
  emptyChannelWindowStore,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { reconcileChannelWindowMessages } from "./channelWindowReconciliation";

export const CHANNEL_WINDOW_FRESH_MS = 5 * 60_000;

/**
 * Subscription setup closes the gap between the initial page and live events,
 * but revisiting a channel with a fresh page has no gap to close. Reconnects
 * still refresh unconditionally at their call site.
 */
export function shouldRefreshChannelWindowAfterSubscribe(
  queryClient: QueryClient,
  channelId: string,
  now = Date.now(),
): boolean {
  const messagesState = queryClient.getQueryState(
    channelMessagesKey(channelId),
  );
  if (!messagesState) return true;
  if (messagesState.fetchStatus === "fetching") return false;
  const windowState = queryClient.getQueryState(channelWindowKey(channelId));
  if (
    messagesState.status !== "success" ||
    windowState?.status !== "success" ||
    windowState.dataUpdatedAt === 0
  ) {
    return true;
  }
  return now - windowState.dataUpdatedAt >= CHANNEL_WINDOW_FRESH_MS;
}

/** Keep the rendered timeline cache aligned with its authoritative window. */
export function projectChannelWindowMessages(
  queryClient: QueryClient,
  channelId: string,
) {
  const window =
    queryClient.getQueryData<ChannelWindowStore>(channelWindowKey(channelId)) ??
    emptyChannelWindowStore();
  queryClient.setQueryData<RelayEvent[]>(
    channelMessagesKey(channelId),
    (messages = []) => reconcileChannelWindowMessages(window, messages),
  );
}

export async function refreshChannelWindowMessages(
  queryClient: QueryClient,
  channelId: string,
) {
  await queryClient.invalidateQueries({
    queryKey: channelMessagesKey(channelId),
    exact: true,
    refetchType: "active",
  });
  projectChannelWindowMessages(queryClient, channelId);
}
