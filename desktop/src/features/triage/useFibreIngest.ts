import * as React from "react";

import { channelCatchUpEventKinds } from "@/features/channels/useUnreadChannels";
import { useChannelsQuery } from "@/features/channels/hooks";
import { usefulStoredPersonLabel } from "@/features/home/ui/fibre/fibreFormat";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import {
  candidateFromEvent,
  collectChannelCandidates,
  sortOldestFirst,
  type TriageCandidate,
} from "@/features/triage/lib/collectCandidates";
import {
  useFibresQuery,
  useIngestMessagesMutation,
} from "@/features/triage/hooks";
import { useTriageAutoScan } from "@/features/triage/useTriageAutoScan";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";

const MESSAGE_KIND_SET = new Set<number>(CHANNEL_MESSAGE_EVENT_KINDS);

async function withResolvedAuthorLabels(
  candidates: TriageCandidate[],
  currentPubkey?: string,
): Promise<TriageCandidate[]> {
  const pubkeys = [
    ...new Set(candidates.map((candidate) => candidate.authorPubkey)),
  ].filter((pubkey) => pubkey.length > 0);
  if (pubkeys.length === 0) return candidates;
  try {
    const { profiles } = await getUsersBatch(pubkeys);
    return candidates.map((candidate) => ({
      ...candidate,
      authorLabel: resolveUserLabel({
        pubkey: candidate.authorPubkey,
        currentPubkey,
        profiles,
        preferResolvedSelfLabel: true,
        fallbackName: usefulStoredPersonLabel(
          candidate.authorLabel,
          candidate.authorPubkey,
        ),
      }),
    }));
  } catch (error) {
    console.warn("[fibre] profile lookup failed", error);
    return candidates;
  }
}

export function useFibreIngest() {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data ?? [];
  const ingestMutation = useIngestMessagesMutation(currentPubkey);
  const fibresQuery = useFibresQuery(currentPubkey);
  const mutateAsync = ingestMutation.mutateAsync;

  const pendingRef = React.useRef<Map<string, TriageCandidate>>(new Map());
  const [pendingCount, setPendingCount] = React.useState(0);
  const backfilledForRef = React.useRef<string | null>(null);
  const backfillInFlightRef = React.useRef(false);
  const hadFibresRef = React.useRef(false);

  const context = React.useMemo(() => ({ currentPubkey }), [currentPubkey]);

  const flush = React.useCallback(async () => {
    const batch = [...pendingRef.current.values()];
    pendingRef.current.clear();
    setPendingCount(0);
    if (batch.length === 0) return;
    try {
      await mutateAsync(
        sortOldestFirst(await withResolvedAuthorLabels(batch, currentPubkey)),
      );
    } catch (error) {
      console.warn("[fibre] ingest failed", error);
    }
  }, [currentPubkey, mutateAsync]);

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

  const runBackfill = React.useCallback(async () => {
    if (!currentPubkey || backfillInFlightRef.current) return;
    backfillInFlightRef.current = true;
    try {
      const collected = await collectChannelCandidates({
        channels,
        context,
        fetchEvents: (filter) => relayClient.fetchEvents(filter),
        kindsForChannel: channelCatchUpEventKinds,
      });
      if (collected.length === 0) return;
      await mutateAsync(
        sortOldestFirst(
          await withResolvedAuthorLabels(collected, currentPubkey),
        ),
      );
      backfilledForRef.current = currentPubkey;
    } catch (error) {
      console.warn("[fibre] backfill failed", error);
    } finally {
      backfillInFlightRef.current = false;
    }
  }, [channels, context, currentPubkey, mutateAsync]);

  useTriageAutoScan({
    enabled: Boolean(currentPubkey),
    isScanning: ingestMutation.isPending,
    pendingCount,
    onScan: () => {
      void flush();
    },
  });

  // After the engine store is purged, open+cleared both drop to zero. Re-run
  // catch-up once so fibres regenerate with the current classifier.
  React.useEffect(() => {
    const data = fibresQuery.data;
    if (!data) return;
    const empty = data.fibres.length === 0 && data.clearedCount === 0;
    if (!empty) {
      hadFibresRef.current = true;
      return;
    }
    if (hadFibresRef.current && backfilledForRef.current) {
      hadFibresRef.current = false;
      backfilledForRef.current = null;
      void runBackfill();
    }
  }, [fibresQuery.data, runBackfill]);

  React.useEffect(() => {
    if (!currentPubkey) {
      backfilledForRef.current = null;
      return;
    }
    if (channelsQuery.isLoading) return;
    if (backfilledForRef.current === currentPubkey) return;
    void runBackfill();
  }, [channelsQuery.isLoading, currentPubkey, runBackfill]);

  return {
    currentPubkey,
    enqueueLive,
    enqueueSelf,
    isIngesting: ingestMutation.isPending,
  };
}
