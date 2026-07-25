import { useQuery } from "@tanstack/react-query";

import { searchMessages } from "@/shared/api/tauri";

export function useSearchMessagesQuery(
  query: string,
  options?: {
    channelId?: string;
    authors?: string[];
    since?: number | null;
    until?: number | null;
    enabled?: boolean;
    limit?: number;
  },
) {
  const trimmedQuery = query.trim();
  const enabled = options?.enabled ?? true;
  const limit = options?.limit ?? 12;
  const channelId = options?.channelId;
  const authors = options?.authors;
  const since = options?.since ?? null;
  const until = options?.until ?? null;

  return useQuery({
    queryKey: [
      "search-messages",
      trimmedQuery,
      limit,
      channelId ?? null,
      authors ?? null,
      since,
      until,
    ],
    queryFn: () =>
      searchMessages({
        q: trimmedQuery,
        limit,
        channelId,
        authors,
        since: since ?? undefined,
        until: until ?? undefined,
      }),
    enabled:
      enabled &&
      (trimmedQuery.length >= 2 ||
        Boolean(authors?.length) ||
        since != null ||
        until != null ||
        Boolean(channelId)),
    staleTime: 30_000,
    gcTime: 5 * 60 * 1_000,
  });
}
