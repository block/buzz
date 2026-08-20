import * as React from "react";

import { channelCatchUpEventKinds } from "@/features/channels/useUnreadChannels";
import { useChannelsQuery } from "@/features/channels/hooks";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import {
  candidateFromEvent,
  collectChannelCandidates,
  sortOldestFirst,
  type TriageCandidate,
} from "@/features/triage/lib/collectCandidates";
import { useTriageAutoScan } from "@/features/triage/useTriageAutoScan";
import { useIngestMessagesMutation } from "@/features/triage/hooks";

const MESSAGE_KIND_SET = new Set<number>(CHANNEL_MESSAGE_EVENT_KINDS);

export function useFibreIngest() {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data ?? [];
  const ingestMutation = useIngestMessagesMutation(currentPubkey);
  const mutateAsync = ingestMutation.mutateAsync;

  const pendingRef = React.useRef<Map<string, TriageCandidate>>(new Map());
  const [pendingCount, setPendingCount] = React.useState(0);
  const backfilledForRef = React.useRef<string | null>(null);

  const context = React.useMemo(() => ({ currentPubkey }), [currentPubkey]);

  const flush = React.useCallback(async () => {
    const batch = [...pendingRef.current.values()];
    pendingRef.current.clear();
    setPendingCount(0);
    if (batch.length === 0) return;
    try {
      await mutateAsync(sortOldestFirst(batch));
    } catch (error) {
      console.warn("[fibre] ingest failed", error);
    }
  }, [mutateAsync]);

  const enqueue = React.useCallback(
    (event: RelayEvent, channel?: Channel) => {
      if (!MESSAGE_KIND_SET.has(event.kind) && channel?.channelType !== "dm") {
        return;
      }
      if (!event.content.trim()) return;
      const candidate = candidateFromEvent(event, channel, context);
      candidate.source = "live";
      pendingRef.current.set(candidate.eventId, candidate);
      setPendingCount(pendingRef.current.size);
    },
    [context],
  );

  const enqueueLive = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      const channel = channels.find((entry) => entry.id === channelId);
      enqueue(event, channel);
    },
    [channels, enqueue],
  );

  const enqueueSelf = React.useCallback(
    (event: RelayEvent) => {
      const channelId = getChannelIdFromTags(event.tags);
      const channel = channelId
        ? channels.find((entry) => entry.id === channelId)
        : undefined;
      enqueue(event, channel);
    },
    [channels, enqueue],
  );

  useTriageAutoScan({
    enabled: Boolean(currentPubkey),
    isScanning: ingestMutation.isPending,
    pendingCount,
    onScan: () => {
      void flush();
    },
  });

  React.useEffect(() => {
    if (!currentPubkey) {
      backfilledForRef.current = null;
      return;
    }
    if (channelsQuery.isLoading) return;
    if (backfilledForRef.current === currentPubkey) return;

    let cancelled = false;
    void (async () => {
      try {
        const collected = await collectChannelCandidates({
          channels,
          context,
          fetchEvents: (filter) => relayClient.fetchEvents(filter),
          kindsForChannel: channelCatchUpEventKinds,
        });
        if (cancelled || collected.length === 0) return;
        await mutateAsync(sortOldestFirst(collected));
        if (!cancelled) backfilledForRef.current = currentPubkey;
      } catch (error) {
        console.warn("[fibre] backfill failed", error);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [channels, channelsQuery.isLoading, context, currentPubkey, mutateAsync]);

  return {
    currentPubkey,
    enqueueLive,
    enqueueSelf,
    isIngesting: ingestMutation.isPending,
  };
}
