import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { SearchResult } from "@/features/search/ui/SearchResultItem";
import type { Channel } from "@/shared/api/types";

export const MIN_SEARCH_QUERY_LENGTH = 2;

export function useSearchResults({
  channels,
  enabled,
  limit = 12,
}: {
  channels: Channel[];
  enabled: boolean;
  limit?: number;
}) {
  const [query, setQuery] = React.useState("");
  const [debouncedQuery, setDebouncedQuery] = React.useState("");
  const [selectedIndex, setSelectedIndex] = React.useState(0);

  const channelLookup = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  const searchQuery = useSearchMessagesQuery(debouncedQuery, {
    enabled,
    limit,
  });

  const messageResults = searchQuery.data?.hits ?? [];
  const channelResults = React.useMemo(() => {
    if (debouncedQuery.length < MIN_SEARCH_QUERY_LENGTH) {
      return [];
    }

    const normalizedQuery = debouncedQuery.toLowerCase();

    return channels
      .filter(
        (channel) =>
          channel.channelType !== "dm" &&
          (channel.archivedAt
            ? channel.isMember
            : channel.visibility === "open" || channel.isMember) &&
          (channel.name.toLowerCase().includes(normalizedQuery) ||
            channel.description.toLowerCase().includes(normalizedQuery)),
      )
      .sort((a, b) => {
        const aNameMatches = a.name.toLowerCase().includes(normalizedQuery);
        const bNameMatches = b.name.toLowerCase().includes(normalizedQuery);

        if (aNameMatches !== bNameMatches) {
          return aNameMatches ? -1 : 1;
        }

        return a.name.localeCompare(b.name);
      })
      .slice(0, 5);
  }, [channels, debouncedQuery]);

  const results = React.useMemo<SearchResult[]>(
    () => [
      ...channelResults.map((channel) => ({
        kind: "channel" as const,
        channel,
      })),
      ...messageResults.map((hit) => ({
        kind: "message" as const,
        hit,
      })),
    ],
    [channelResults, messageResults],
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
    messageResults,
    query,
    resultProfiles: resultProfilesQuery.data?.profiles,
    results,
    searchQuery,
    selectedIndex,
    selectedResult: results[selectedIndex],
    setQuery,
    setSelectedIndex,
  };
}
