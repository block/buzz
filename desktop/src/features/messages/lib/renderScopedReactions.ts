import type { QueryClient } from "@tanstack/react-query";

import { channelMessagesKey, sortMessages } from "./messageQueryKeys";
import {
  messageUnitGeneration,
  updateRetainedMessageUnit,
} from "./messageUnitGuard";
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

/**
 * Drop the hydration claim set for one channel. Called when the channel's
 * message window is evicted so a revisit re-hydrates its visible reactions from
 * scratch — without this, the claim set would report every id as already
 * hydrated and the re-fetched window would render with no reactions until a
 * live reaction arrived.
 */
export function clearChannelRenderScopedReactionHydration(channelId: string) {
  hydratedMessageIdsByChannel.delete(channelId);
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

  // Capture the channel unit's generation before the fetch. If the window is
  // evicted while this reaction fetch is in flight, the generation advances and
  // `updateRetainedMessageUnit` drops the write below — so a deferred reaction
  // merge can neither resurrect the evicted timeline nor stale-merge onto an
  // A→B→A re-fetch.
  const generationAtStart = messageUnitGeneration(input.channelId);

  try {
    const reactionEvents = await (
      input.deps ?? defaultDeps
    ).fetchReactionEventsForMessages(input.channelId, messageIds);
    if (reactionEvents.length === 0) {
      return;
    }

    updateRetainedMessageUnit<RelayEvent[]>(
      input.queryClient,
      channelMessagesKey(input.channelId),
      input.channelId,
      generationAtStart,
      (current) => sortMessages([...current, ...reactionEvents]),
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
