import type { QueryClient } from "@tanstack/react-query";

import { channelMessagesKey, sortMessages } from "./messageQueryKeys";
import type { MainTimelineEntry } from "./threadPanel";
import type { TimelineMessage } from "../types";
import { relayClient } from "@/shared/api/relayClient";
import { buildChannelReactionAuxFilter } from "@/shared/api/relayChannelFilters";
import type { RelayEvent } from "@/shared/api/types";

export type RenderScopedReactionDeps = {
  fetchReactionEventsForMessages: (
    channelId: string,
    messageIds: string[],
  ) => Promise<RelayEvent[]>;
};

const defaultDeps: RenderScopedReactionDeps = {
  fetchReactionEventsForMessages: (channelId, messageIds) =>
    relayClient.fetchAuxEventsByReference(
      channelId,
      messageIds,
      buildChannelReactionAuxFilter,
    ),
};

const hydratedMessageIdsByChannel = new Map<string, Set<string>>();

export function resetRenderScopedReactionHydration() {
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

function pushUniqueMessageId(ids: string[], seen: Set<string>, id?: string) {
  if (!id || seen.has(id)) {
    return;
  }
  seen.add(id);
  ids.push(id);
}

export function collectRenderScopedReactionMessageIds(input: {
  mainEntries: readonly MainTimelineEntry[];
  threadHeadMessage?: TimelineMessage | null;
  threadEntries?: readonly MainTimelineEntry[];
}): string[] {
  const ids: string[] = [];
  const seen = new Set<string>();

  for (const entry of input.mainEntries) {
    pushUniqueMessageId(ids, seen, entry.message.id);
  }

  pushUniqueMessageId(ids, seen, input.threadHeadMessage?.id);

  for (const entry of input.threadEntries ?? []) {
    pushUniqueMessageId(ids, seen, entry.message.id);
  }

  return ids;
}

export function claimUnhydratedRenderScopedReactionIds(
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

export function releaseRenderScopedReactionIds(
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

export async function hydrateRenderScopedReactions(input: {
  channelId: string;
  messageIds: readonly string[];
  queryClient: QueryClient;
  deps?: RenderScopedReactionDeps;
}): Promise<void> {
  const messageIds = claimUnhydratedRenderScopedReactionIds(
    input.channelId,
    input.messageIds,
  );
  if (messageIds.length === 0) {
    return;
  }

  try {
    const reactionEvents = await (
      input.deps ?? defaultDeps
    ).fetchReactionEventsForMessages(input.channelId, messageIds);
    if (reactionEvents.length === 0) {
      return;
    }

    input.queryClient.setQueryData<RelayEvent[]>(
      channelMessagesKey(input.channelId),
      (current = []) => sortMessages([...current, ...reactionEvents]),
    );
  } catch (error) {
    releaseRenderScopedReactionIds(input.channelId, messageIds);
    console.error(
      "Failed to hydrate visible reactions for channel",
      input.channelId,
      error,
    );
  }
}
