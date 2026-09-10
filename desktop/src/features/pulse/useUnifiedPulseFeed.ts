import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useContactListQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { fetchPulseFeed } from "./lib/fetchPulseFeed";
import { buildPulseConversations, PULSE_SOURCE_LIMIT } from "./lib/unifiedFeed";
import { usePulseRefresh } from "./usePulseRefresh";

export function useUnifiedPulseFeed(currentPubkey?: string) {
  const queryClient = useQueryClient();
  const channelsQuery = useChannelsQuery();
  const contactsQuery = useContactListQuery(currentPubkey);
  const agentPubkeys = useKnownAgentPubkeys();
  const relaySelf = useRelaySelfQuery();
  const connection = useRelayConnection();
  const [excludedIds, setExcludedIds] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [includeNotes, setIncludeNotes] = React.useState(true);
  const channels = React.useMemo(
    () =>
      (channelsQuery.data ?? [])
        .filter((c) => c.isMember && !c.archivedAt)
        .sort((a, b) =>
          (b.lastMessageAt ?? "").localeCompare(a.lastMessageAt ?? ""),
        ),
    [channelsQuery.data],
  );
  const selectedChannels = React.useMemo(
    () =>
      channels
        .filter((c) => !excludedIds.has(c.id))
        .slice(0, PULSE_SOURCE_LIMIT),
    [channels, excludedIds],
  );
  const channelIds = selectedChannels.map((c) => c.id).sort();
  const noteAuthors = includeNotes
    ? [
        ...new Set(
          [
            currentPubkey,
            ...(contactsQuery.data?.contacts ?? []).map((c) => c.pubkey),
          ].filter((key): key is string => Boolean(key)),
        ),
      ]
        .sort()
        .slice(0, 100)
    : [];
  const scope = `${channelIds.join(",")}|${noteAuthors.join(",")}`;
  const refetchInterval = useFocusedRefetchInterval(
    connection === "connected" ? 30_000 : false,
  );
  const query = useQuery({
    queryKey: ["pulse-unified", currentPubkey, scope],
    queryFn: () =>
      fetchPulseFeed(
        channelIds,
        noteAuthors,
        undefined,
        selectedChannels.filter((c) => c.channelType === "dm").map((c) => c.id),
      ),
    enabled: Boolean(currentPubkey) && channelsQuery.isSuccess,
    refetchInterval,
    staleTime: 20_000,
    gcTime: 60_000,
    retry: 1,
    refetchOnWindowFocus: false,
  });
  usePulseRefresh({
    enabled:
      Boolean(currentPubkey) &&
      channelsQuery.isSuccess &&
      connection === "connected",
    scope: `${currentPubkey}:${scope}`,
    refresh: () => query.refetch({ cancelRefetch: false }),
  });
  const pubkeys = React.useMemo(
    () => [...new Set((query.data ?? []).map((event) => event.pubkey))],
    [query.data],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles ?? {};
  const conversations = React.useMemo(
    () =>
      buildPulseConversations(
        query.data ?? [],
        selectedChannels,
        currentPubkey,
        profiles,
        agentPubkeys,
        relaySelf.data,
      ),
    [
      query.data,
      selectedChannels,
      currentPubkey,
      profiles,
      agentPubkeys,
      relaySelf.data,
    ],
  );
  const toggleSource = (id: string) =>
    setExcludedIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  return {
    query,
    channels,
    selectedChannels,
    conversations,
    profiles,
    scope,
    excludedIds,
    toggleSource,
    includeNotes,
    setIncludeNotes,
    refresh: () =>
      queryClient.invalidateQueries(
        {
          predicate: (entry) =>
            [
              "pulse-unified",
              "channels",
              "channel-messages",
              "thread-replies",
            ].includes(String(entry.queryKey[0])),
          refetchType: "active",
        },
        { cancelRefetch: false },
      ),
    error:
      channelsQuery.error ??
      (includeNotes ? contactsQuery.error : null) ??
      query.error,
    isLoading:
      channelsQuery.isPending ||
      (includeNotes && contactsQuery.isPending) ||
      query.isLoading,
    retry: () => {
      void channelsQuery.refetch();
      void contactsQuery.refetch();
      void query.refetch();
    },
  };
}
