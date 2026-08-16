import type { QueryClient } from "@tanstack/react-query";

import { channelMessagesKey, sortMessages } from "./messageQueryKeys";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelDispositionAuxFilter } from "@/shared/api/relayChannelFilters";
import type { RelayEvent } from "@/shared/api/types";

export type RenderScopedDispositionDeps = {
  fetchDispositionEventsForMessages: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
};

const defaultDeps: RenderScopedDispositionDeps = {
  fetchDispositionEventsForMessages: (channelId, messageIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelDispositionAuxFilter,
    ),
};

const hydratedMessageIdsByChannel = new Map<string, Set<string>>();

export function resetRenderScopedDispositionHydration() {
  hydratedMessageIdsByChannel.clear();
}

function hydratedSetForChannel(channelId: string): Set<string> {
  let hydrated = hydratedMessageIdsByChannel.get(channelId);
  if (!hydrated) {
    hydrated = new Set();
    hydratedMessageIdsByChannel.set(channelId, hydrated);
  }
  return hydrated;
}

export function claimUnhydratedRenderScopedDispositionIds(
  channelId: string,
  messageIds: readonly string[],
): string[] {
  const hydrated = hydratedSetForChannel(channelId);
  const claimed: string[] = [];

  for (const id of messageIds) {
    if (hydrated.has(id)) {
      continue;
    }
    hydrated.add(id);
    claimed.push(id);
  }

  return claimed;
}

export function releaseRenderScopedDispositionIds(
  channelId: string,
  messageIds: readonly string[],
) {
  const hydrated = hydratedMessageIdsByChannel.get(channelId);
  if (!hydrated) {
    return;
  }

  for (const id of messageIds) {
    hydrated.delete(id);
  }

  if (hydrated.size === 0) {
    hydratedMessageIdsByChannel.delete(channelId);
  }
}

export async function hydrateRenderScopedDispositions(input: {
  channelId: string;
  messageIds: readonly string[];
  queryClient: QueryClient;
  deps?: RenderScopedDispositionDeps;
}): Promise<void> {
  const messageIds = claimUnhydratedRenderScopedDispositionIds(
    input.channelId,
    input.messageIds,
  );
  if (messageIds.length === 0) {
    return;
  }

  try {
    const dispositionEvents = await (
      input.deps ?? defaultDeps
    ).fetchDispositionEventsForMessages(input.channelId, messageIds);
    if (dispositionEvents.length === 0) {
      return;
    }

    input.queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(input.channelId),
      (current = []) => sortMessages([...current, ...dispositionEvents]),
    );
  } catch (error) {
    releaseRenderScopedDispositionIds(input.channelId, messageIds);
    console.error(
      "Failed to hydrate visible dispositions for channel",
      input.channelId,
      error,
    );
  }
}
