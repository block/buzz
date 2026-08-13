import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import {
  messageUnitGeneration,
  updateRetainedMessageUnit,
} from "@/features/messages/lib/messageUnitGuard";
import { mergeMessages } from "@/features/messages/hooks";
import {
  getChannelIdFromTags,
  getThreadReference,
} from "@/features/messages/lib/threading";
import { getEventById } from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";

export function useLoadMissingAncestors(
  activeChannel: Channel | null,
  resolvedMessages: RelayEvent[],
) {
  const queryClient = useQueryClient();
  const requestedAncestorIdsRef = React.useRef<Set<string>>(new Set());
  const previousChannelIdRef = React.useRef<string | null>(null);

  React.useEffect(() => {
    const activeChannelId = activeChannel?.id ?? null;
    if (previousChannelIdRef.current === activeChannelId) {
      return;
    }
    previousChannelIdRef.current = activeChannelId;
    requestedAncestorIdsRef.current.clear();
  }, [activeChannel?.id]);

  React.useEffect(() => {
    if (!activeChannel || activeChannel.channelType === "forum") {
      return;
    }

    const knownEvents = new Map(
      resolvedMessages.map((message) => [message.id, message]),
    );
    const missingAncestorIds = new Set<string>();

    for (const message of resolvedMessages) {
      const thread = getThreadReference(message.tags);

      for (const eventId of [thread.parentId, thread.rootId]) {
        if (
          !eventId ||
          knownEvents.has(eventId) ||
          requestedAncestorIdsRef.current.has(eventId)
        ) {
          continue;
        }

        missingAncestorIds.add(eventId);
      }
    }

    if (missingAncestorIds.size === 0) {
      return;
    }

    for (const eventId of missingAncestorIds) {
      requestedAncestorIdsRef.current.add(eventId);
    }

    const maxRequestedAncestors = 500;
    if (requestedAncestorIdsRef.current.size > maxRequestedAncestors) {
      const excess =
        requestedAncestorIdsRef.current.size - maxRequestedAncestors;
      let removed = 0;
      for (const id of requestedAncestorIdsRef.current) {
        if (removed >= excess) {
          break;
        }
        requestedAncestorIdsRef.current.delete(id);
        removed++;
      }
    }

    let isCancelled = false;

    // Capture the channel unit's generation before the ancestor fetches. If the
    // window is evicted while a `getEventById` is in flight, the generation
    // advances and `updateRetainedMessageUnit` drops the merge below — a
    // deferred ancestor write can neither resurrect the evicted timeline nor
    // stale-merge onto an A→B→A re-fetch. (`requestedAncestorIdsRef` needs no
    // matching clear on eviction: it is a per-hook ref already reset whenever
    // `activeChannel.id` changes, so it never carries ids across channels.)
    const generationAtStart = messageUnitGeneration(activeChannel.id);

    void Promise.all(
      [...missingAncestorIds].map(async (eventId) => {
        try {
          const event = await getEventById(eventId);

          if (
            isCancelled ||
            getChannelIdFromTags(event.tags) !== activeChannel.id
          ) {
            return;
          }

          updateRetainedMessageUnit<RelayEvent[]>(
            queryClient,
            channelMessagesKey(activeChannel.id),
            activeChannel.id,
            generationAtStart,
            (current) => mergeMessages(current, event),
          );
        } catch (error) {
          console.error("Failed to load ancestor event", eventId, error);
        }
      }),
    );

    return () => {
      isCancelled = true;
    };
  }, [activeChannel, queryClient, resolvedMessages]);
}
