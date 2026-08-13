import type { QueryClient } from "@tanstack/react-query";

import type { RelayEvent } from "@/shared/api/types";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys";
import {
  emptyChannelWindowStore,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { reconcileChannelWindowMessages } from "./channelWindowReconciliation";

/**
 * Keep the rendered timeline cache aligned with its authoritative window.
 *
 * Only aligns an EXISTING timeline cache: a removed `channelMessagesKey` reads
 * back absent, and a bare `(messages = []) => …` updater would resurrect the
 * timeline this cache just bound away. Every pin-covered caller (live append,
 * optimistic send, older-page fetch) still holds the key, so the guard is a
 * no-op there; it fences only the deferred `refreshChannelWindowMessages` path,
 * whose post-`await` projection can otherwise land after a fetch-settle resweep
 * evicted the channel. The channel's own messages query owns key CREATION on
 * (re)open — projection never needs to create it. An in-flight initial fetch
 * has query state (only a never-created/removed key reads `undefined`), so
 * first-load population is unaffected.
 */
export function projectChannelWindowMessages(
  queryClient: QueryClient,
  channelId: string,
) {
  const messagesKey = channelMessagesKey(channelId);
  if (queryClient.getQueryState(messagesKey) === undefined) {
    return;
  }
  const window =
    queryClient.getQueryData<ChannelWindowStore>(channelWindowKey(channelId)) ??
    emptyChannelWindowStore();
  queryClient.setQueryData<RelayEvent[]>(messagesKey, (messages = []) =>
    reconcileChannelWindowMessages(window, messages),
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
