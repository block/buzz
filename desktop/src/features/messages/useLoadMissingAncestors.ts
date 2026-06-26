import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { channelMessagesKey } from "@/features/messages/lib/messageQueryKeys";
import { mergeMessages } from "@/features/messages/hooks";
import { makeDmIngestDecryptor } from "@/features/messages/lib/dmCrypto";
import {
  getChannelIdFromTags,
  getThreadReference,
} from "@/features/messages/lib/threading";
import { getEventById } from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";

/** The scope that the requested-ancestor dedup set is valid for. */
interface AncestorScope {
  channelId: string | null;
  selfPubkey: string | undefined;
}

/**
 * Whether the requested-ancestor dedup tracking must reset.
 *
 * The dedup set keys "ancestor already fetched" by id, but a fetched ancestor
 * lands in `channelMessagesKey(channelId, selfPubkey)` — a bucket scoped by
 * BOTH channel AND identity. So the set is only valid within a single
 * (channel, identity) scope. On a cold start an ancestor fetched while
 * `selfPubkey` is undefined no-op-decrypts into the orphaned `[...,null]`
 * bucket yet gets recorded as done; without resetting on the identity flip the
 * effect would skip re-fetching it into the live `[...,pubkey]` bucket and the
 * ancestor would silently go missing from the thread.
 */
export function shouldResetAncestorTracking(
  previous: AncestorScope,
  next: AncestorScope,
): boolean {
  return (
    previous.channelId !== next.channelId ||
    previous.selfPubkey !== next.selfPubkey
  );
}

export function useLoadMissingAncestors(
  activeChannel: Channel | null,
  resolvedMessages: RelayEvent[],
  selfPubkey?: string,
) {
  const queryClient = useQueryClient();
  const requestedAncestorIdsRef = React.useRef<Set<string>>(new Set());
  const previousScopeRef = React.useRef<AncestorScope>({
    channelId: null,
    selfPubkey: undefined,
  });

  React.useEffect(() => {
    const scope: AncestorScope = {
      channelId: activeChannel?.id ?? null,
      selfPubkey,
    };
    if (!shouldResetAncestorTracking(previousScopeRef.current, scope)) {
      return;
    }
    previousScopeRef.current = scope;
    requestedAncestorIdsRef.current.clear();
  }, [activeChannel?.id, selfPubkey]);

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

    const decryptIngested = makeDmIngestDecryptor(activeChannel, selfPubkey);

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

          // Decrypt before caching: a DM ancestor is a NIP-44 v2 ciphertext
          // body, so it must route through the same decryptor as every other
          // ingest site or it lands raw in the rendered bucket (the decryptor
          // is a no-op outside a 2-party DM, so this is uniform/safe).
          const [decrypted] = await decryptIngested([event]);

          queryClient.setQueryData<RelayEvent[]>(
            channelMessagesKey(activeChannel.id, selfPubkey),
            (current = []) => mergeMessages(current, decrypted),
          );
        } catch (error) {
          console.error("Failed to load ancestor event", eventId, error);
        }
      }),
    );

    return () => {
      isCancelled = true;
    };
  }, [activeChannel, queryClient, resolvedMessages, selfPubkey]);
}
