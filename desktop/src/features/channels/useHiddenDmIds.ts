import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_DM_VISIBILITY } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const dmVisibilityQueryKey = ["dm-visibility"] as const;

/** Exact query key for one relay+identity scope's DM-visibility snapshot. */
export function dmVisibilityQueryKeyFor(
  relayUrl: string | undefined,
  pubkey: string | undefined,
) {
  return [
    ...dmVisibilityQueryKey,
    relayUrl ?? "",
    normalizePubkey(pubkey ?? ""),
  ] as const;
}

export function extractHiddenDmIds(events: readonly RelayEvent[]): Set<string> {
  const latest = events.reduce<RelayEvent | null>(
    (current, event) =>
      current === null || event.created_at > current.created_at
        ? event
        : current,
    null,
  );
  return new Set(
    (latest?.tags ?? [])
      .filter((tag) => tag[0] === "h" && tag[1])
      .map((tag) => tag[1]),
  );
}

export async function fetchHiddenDmIds(pubkey: string): Promise<Set<string>> {
  const normalizedPubkey = normalizePubkey(pubkey);
  if (normalizedPubkey.length === 0) return new Set();
  const events = await relayClient.fetchEvents({
    kinds: [KIND_DM_VISIBILITY],
    "#p": [normalizedPubkey],
    limit: 1,
  });
  return extractHiddenDmIds(events);
}

export function useHiddenDmIds(pubkey: string | undefined) {
  const { activeCommunity } = useCommunities();
  const normalizedPubkey = normalizePubkey(pubkey ?? "");
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const query = useQuery({
    queryKey: dmVisibilityQueryKeyFor(relayUrl, normalizedPubkey),
    queryFn: () => fetchHiddenDmIds(normalizedPubkey),
    enabled: relayUrl.length > 0 && normalizedPubkey.length > 0,
    staleTime: 30_000,
  });

  return React.useMemo(() => query.data ?? new Set<string>(), [query.data]);
}
