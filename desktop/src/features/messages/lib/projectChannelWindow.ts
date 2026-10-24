import type { QueryClient } from "@tanstack/react-query";

import type { RelayEvent } from "@/shared/api/types";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys";
import {
  emptyChannelWindowStore,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { reconcileChannelWindowMessages } from "./channelWindowReconciliation";
import { channelHeadHydration } from "./channelHeadCache";

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
  const queryKey = channelMessagesKey(channelId);
  // Sequence behind persisted-head hydration. While the channel query is parked
  // on that gate it has no data, so TanStack would dedupe this invalidation
  // onto it — and that fetch returns the seeded snapshot, never asking the
  // relay. A seeded query is recognisable by data at `dataUpdatedAt` 0; let its
  // snapshot fetch settle (consuming the mount gate) before invalidating, so
  // the refetch is a distinct authoritative window fetch. Concurrent callers
  // (subscribe settlement + reconnect) wake on the same promise, so the seeded
  // branch must join an authoritative fetch already in flight rather than
  // cancel and replace it. Cold and warm channels carry no such marker and
  // dedupe/cancel exactly as before.
  await channelHeadHydration(queryClient);
  const query = queryClient.getQueryCache().find({ queryKey, exact: true });
  const seeded =
    query?.state.data !== undefined && query.state.dataUpdatedAt === 0;
  if (seeded) {
    await query.promise?.catch(() => {});
  }
  await queryClient.invalidateQueries(
    { queryKey, exact: true, refetchType: "active" },
    { cancelRefetch: !seeded, throwOnError: true },
  );
  projectChannelWindowMessages(queryClient, channelId);
}
