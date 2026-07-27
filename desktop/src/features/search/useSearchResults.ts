import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import {
  useUserSearchQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { rankUserCandidatesBySearch } from "@/features/profile/lib/userCandidateSearch";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import {
  isChannelUuid,
  isHexPubkey,
  normalizeFromHandle,
  normalizeInChannel,
  parseSearchOperators,
  type OperatorResolveResult,
} from "@/features/search/lib/parseSearchOperators";
import type { SearchResult } from "@/features/search/ui/SearchResultItem";
import type { Channel, SearchHit, UserSearchResult } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const MIN_SEARCH_QUERY_LENGTH = 2;

function formatUserResultName(user: UserSearchResult) {
  return user.displayName?.trim() || user.nip05Handle?.trim() || user.pubkey;
}

function dedupeSearchHits(hits: SearchHit[]) {
  const seenEventIds = new Set<string>();

  return hits.filter((hit) => {
    if (seenEventIds.has(hit.eventId)) {
      return false;
    }

    seenEventIds.add(hit.eventId);
    return true;
  });
}

function resolveChannelIdFromOperator(
  raw: string | null,
  channels: Channel[],
  channelLabels?: Record<string, string>,
): OperatorResolveResult<string> {
  if (!raw) {
    return { status: "none" };
  }
  const value = normalizeInChannel(raw);
  if (!value) {
    return { status: "none" };
  }
  if (isChannelUuid(value)) {
    return { status: "resolved", value };
  }
  const needle = value.toLowerCase();
  const match = channels.find((channel) => {
    const label = channelLabels?.[channel.id]?.trim() || channel.name;
    return (
      channel.name.toLowerCase() === needle || label.toLowerCase() === needle
    );
  });
  return match
    ? { status: "resolved", value: match.id }
    : { status: "unresolved" };
}

function resolveAuthorFromOperator(
  raw: string | null,
  candidates: Array<{ pubkey: string; displayName?: string | null }>,
): OperatorResolveResult<string> {
  if (!raw) {
    return { status: "none" };
  }
  if (isHexPubkey(raw)) {
    return { status: "resolved", value: normalizePubkey(raw) };
  }
  const handle = normalizeFromHandle(raw).toLowerCase();
  if (!handle) {
    return { status: "unresolved" };
  }
  const match = candidates.find((candidate) => {
    const name = candidate.displayName?.trim().toLowerCase();
    return name === handle || normalizePubkey(candidate.pubkey) === handle;
  });
  return match
    ? { status: "resolved", value: normalizePubkey(match.pubkey) }
    : { status: "unresolved" };
}

export function useSearchResults({
  channelLabels,
  channels,
  enabled,
  limit = 12,
}: {
  channelLabels?: Record<string, string>;
  channels: Channel[];
  enabled: boolean;
  limit?: number;
}) {
  const [query, setQuery] = React.useState("");
  const [debouncedQuery, setDebouncedQuery] = React.useState("");
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const isArchivedDiscovery = useIsArchivedPredicate();

  const channelLookup = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  const parsedQuery = React.useMemo(
    () => parseSearchOperators(debouncedQuery),
    [debouncedQuery],
  );

  const channelResolution = React.useMemo(
    () => resolveChannelIdFromOperator(parsedQuery.in, channels, channelLabels),
    [parsedQuery.in, channels, channelLabels],
  );

  const ftsQuery = parsedQuery.text;

  const hasSearchQuery =
    debouncedQuery.trim().length >= MIN_SEARCH_QUERY_LENGTH ||
    parsedQuery.since !== null ||
    parsedQuery.until !== null ||
    parsedQuery.from !== null ||
    parsedQuery.in !== null;

  const searchBackedQueriesEnabled = enabled && hasSearchQuery;

  const fromHandleForLookup =
    parsedQuery.from && !isHexPubkey(parsedQuery.from)
      ? normalizeFromHandle(parsedQuery.from)
      : "";

  const managedAgentsQuery = useManagedAgentsQuery({
    enabled: searchBackedQueriesEnabled,
  });
  const relayAgentsQuery = useRelayAgentsQuery({
    enabled: searchBackedQueriesEnabled,
  });
  // Resolve `from:@name` against people, not only agents.
  const fromUserSearchQuery = useUserSearchQuery(fromHandleForLookup, {
    enabled: searchBackedQueriesEnabled && fromHandleForLookup.length >= 1,
    limit,
  });
  const userSearchQuery = useUserSearchQuery(ftsQuery, {
    enabled: searchBackedQueriesEnabled,
    limit,
  });

  const authorCandidateSeed = React.useMemo(() => {
    const candidates: Array<{ pubkey: string; displayName?: string | null }> =
      [];
    const seen = new Set<string>();
    const push = (pubkey: string, displayName?: string | null) => {
      const key = normalizePubkey(pubkey);
      if (seen.has(key)) {
        return;
      }
      seen.add(key);
      candidates.push({ pubkey: key, displayName });
    };
    for (const agent of managedAgentsQuery.data ?? []) {
      push(agent.pubkey, agent.name);
    }
    for (const agent of relayAgentsQuery.data ?? []) {
      push(agent.pubkey, agent.name);
    }
    for (const user of fromUserSearchQuery.data ?? []) {
      push(user.pubkey, user.displayName);
    }
    for (const user of userSearchQuery.data ?? []) {
      push(user.pubkey, user.displayName);
    }
    return candidates;
  }, [
    managedAgentsQuery.data,
    relayAgentsQuery.data,
    fromUserSearchQuery.data,
    userSearchQuery.data,
  ]);

  const authorResolution = React.useMemo(
    () => resolveAuthorFromOperator(parsedQuery.from, authorCandidateSeed),
    [parsedQuery.from, authorCandidateSeed],
  );

  const hasUnresolvedOperator =
    authorResolution.status === "unresolved" ||
    channelResolution.status === "unresolved";

  // While `from:@name` user search is still loading, hold off so we do not
  // flash an unresolved empty state before candidates arrive.
  const waitingOnFromResolution =
    Boolean(fromHandleForLookup) &&
    fromUserSearchQuery.isLoading &&
    authorResolution.status === "unresolved";

  const searchQuery = useSearchMessagesQuery(ftsQuery, {
    enabled:
      enabled &&
      !hasUnresolvedOperator &&
      !waitingOnFromResolution &&
      ftsQuery.length >= MIN_SEARCH_QUERY_LENGTH,
    limit,
    channelId:
      channelResolution.status === "resolved"
        ? channelResolution.value
        : undefined,
    authors:
      authorResolution.status === "resolved"
        ? [authorResolution.value]
        : undefined,
    since: parsedQuery.since,
    until: parsedQuery.until,
    unresolvedOperator: hasUnresolvedOperator,
  });

  const messageResults = React.useMemo(() => {
    if (hasUnresolvedOperator) {
      return [];
    }
    return dedupeSearchHits(searchQuery.data?.hits ?? []);
  }, [hasUnresolvedOperator, searchQuery.data?.hits]);
  const channelResults = React.useMemo(() => {
    if (ftsQuery.length < MIN_SEARCH_QUERY_LENGTH) {
      return [];
    }

    const normalizedQuery = ftsQuery.toLowerCase();

    return channels
      .filter(
        (channel) =>
          (channel.archivedAt
            ? channel.isMember
            : channel.visibility === "open" || channel.isMember) &&
          [
            channel.name,
            channel.description,
            channelLabels?.[channel.id] ?? "",
          ].some((value) => value.toLowerCase().includes(normalizedQuery)),
      )
      .sort((a, b) => {
        const aDisplayName = channelLabels?.[a.id]?.trim() || a.name;
        const bDisplayName = channelLabels?.[b.id]?.trim() || b.name;
        const aNameMatches = aDisplayName
          .toLowerCase()
          .includes(normalizedQuery);
        const bNameMatches = bDisplayName
          .toLowerCase()
          .includes(normalizedQuery);

        if (aNameMatches !== bNameMatches) {
          return aNameMatches ? -1 : 1;
        }

        return aDisplayName.localeCompare(bDisplayName);
      })
      .slice(0, 5);
  }, [channelLabels, channels, ftsQuery]);
  const managedAgentPubkeys = React.useMemo(
    () =>
      new Set(
        (managedAgentsQuery.data ?? []).map((agent) =>
          normalizePubkey(agent.pubkey),
        ),
      ),
    [managedAgentsQuery.data],
  );
  const relayAgentPubkeys = React.useMemo(
    () =>
      new Set(
        (relayAgentsQuery.data ?? []).map((agent) =>
          normalizePubkey(agent.pubkey),
        ),
      ),
    [relayAgentsQuery.data],
  );
  const eligibleAgentPubkeys = React.useMemo(() => {
    const pubkeys = new Set(managedAgentPubkeys);

    for (const agent of relayAgentsQuery.data ?? []) {
      if (agent.respondTo === "anyone") {
        pubkeys.add(normalizePubkey(agent.pubkey));
      }
    }

    return pubkeys;
  }, [managedAgentPubkeys, relayAgentsQuery.data]);
  const userResults = React.useMemo<UserSearchResult[]>(() => {
    if (ftsQuery.length < MIN_SEARCH_QUERY_LENGTH) {
      return [];
    }

    const normalizedQuery = ftsQuery.toLowerCase();
    const candidatesByPubkey = new Map<string, UserSearchResult>();

    const matchesQuery = (candidate: UserSearchResult) =>
      [
        candidate.displayName ?? "",
        candidate.nip05Handle ?? "",
        candidate.isAgent ? "agent" : "",
        normalizePubkey(candidate.pubkey),
      ].some((value) => value.toLowerCase().includes(normalizedQuery));

    const addCandidate = (candidate: UserSearchResult) => {
      const pubkey = normalizePubkey(candidate.pubkey);

      if (isArchivedDiscovery(pubkey)) {
        return;
      }

      const isKnownAgent =
        candidate.isAgent ||
        managedAgentPubkeys.has(pubkey) ||
        relayAgentPubkeys.has(pubkey);

      if (isKnownAgent && !eligibleAgentPubkeys.has(pubkey)) {
        return;
      }

      const existing = candidatesByPubkey.get(pubkey);
      if (!existing) {
        candidatesByPubkey.set(pubkey, {
          ...candidate,
          pubkey,
          isAgent: isKnownAgent,
        });
        return;
      }

      candidatesByPubkey.set(pubkey, {
        pubkey,
        avatarUrl: existing.avatarUrl ?? candidate.avatarUrl ?? null,
        displayName:
          candidate.isAgent && candidate.displayName?.trim()
            ? candidate.displayName
            : (existing.displayName ?? candidate.displayName),
        nip05Handle: existing.nip05Handle ?? candidate.nip05Handle ?? null,
        ownerPubkey: existing.ownerPubkey ?? candidate.ownerPubkey ?? null,
        isAgent: existing.isAgent || isKnownAgent,
      });
    };

    for (const user of userSearchQuery.data ?? []) {
      addCandidate(user);
    }

    for (const agent of relayAgentsQuery.data ?? []) {
      if (agent.respondTo !== "anyone") {
        continue;
      }

      const candidate = {
        pubkey: agent.pubkey,
        displayName: agent.name,
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
        isAgent: true,
      };

      if (matchesQuery(candidate)) {
        addCandidate(candidate);
      }
    }

    for (const agent of managedAgentsQuery.data ?? []) {
      const candidate = {
        pubkey: agent.pubkey,
        displayName: agent.name,
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
        isAgent: true,
      };

      if (matchesQuery(candidate)) {
        addCandidate(candidate);
      }
    }

    return rankUserCandidatesBySearch({
      candidates: [...candidatesByPubkey.values()],
      getLabel: formatUserResultName,
      limit,
      query: ftsQuery,
    });
  }, [
    eligibleAgentPubkeys,
    ftsQuery,
    isArchivedDiscovery,
    limit,
    managedAgentPubkeys,
    managedAgentsQuery.data,
    relayAgentPubkeys,
    relayAgentsQuery.data,
    userSearchQuery.data,
  ]);

  const results = React.useMemo<SearchResult[]>(
    () => [
      ...channelResults.map((channel) => ({
        kind: "channel" as const,
        channel,
      })),
      ...userResults.map((user) => ({
        kind: "user" as const,
        user,
      })),
      ...messageResults.map((hit) => ({
        kind: "message" as const,
        hit,
      })),
    ],
    [channelResults, messageResults, userResults],
  );

  const resultProfilesQuery = useUsersBatchQuery(
    messageResults.map((hit) => hit.pubkey),
    {
      enabled: enabled && messageResults.length > 0,
    },
  );

  React.useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < MIN_SEARCH_QUERY_LENGTH) {
      setDebouncedQuery("");
      return;
    }

    const timeout = window.setTimeout(() => {
      setDebouncedQuery(trimmed);
    }, 300);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [query]);

  React.useEffect(() => {
    if (!enabled) {
      setQuery("");
      setDebouncedQuery("");
      setSelectedIndex(0);
    }
  }, [enabled]);

  React.useEffect(() => {
    setSelectedIndex((current) => {
      if (results.length === 0) {
        return 0;
      }

      return Math.min(current, results.length - 1);
    });
  }, [results]);

  return {
    channelLookup,
    channelResults,
    debouncedQuery,
    isWaitingOnFromResolution: waitingOnFromResolution,
    messageResults,
    query,
    resultProfiles: resultProfilesQuery.data?.profiles,
    results,
    searchQuery,
    selectedIndex,
    selectedResult: results[selectedIndex],
    setQuery,
    setSelectedIndex,
    userResults,
    userSearchQuery,
  };
}
