import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { getRelaySelf } from "@/features/moderation/lib/relaySelf";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";

import {
  COFOUNDER_LEAD_D_TAG,
  KIND_COFOUNDER_LEAD_HEAD,
  parseCofounderLeadStatus,
} from "./leadStatusRecords";
import type { CofounderLeadStatus } from "./leadStatusRecords";

function newestFirst(first: RelayEvent, second: RelayEvent): number {
  return (
    second.created_at - first.created_at || first.id.localeCompare(second.id)
  );
}

async function fetchCofounderLeadStatus(): Promise<CofounderLeadStatus | null> {
  const relaySelf = await getRelaySelf();
  if (!relaySelf) {
    throw new Error("This relay does not advertise a signing identity.");
  }
  const events = await relayClient.fetchEvents({
    kinds: [KIND_COFOUNDER_LEAD_HEAD],
    authors: [relaySelf],
    "#d": [COFOUNDER_LEAD_D_TAG],
    limit: 20,
  });
  return (
    [...events]
      .sort(newestFirst)
      .map((event) => parseCofounderLeadStatus(event, relaySelf))
      .find((status) => status !== null) ?? null
  );
}

export function useCofounderLeadStatusQuery(enabled = true) {
  const { activeCommunity } = useCommunities();
  const communityId = activeCommunity?.id ?? null;
  const relayUrl = activeCommunity?.relayUrl ?? null;
  return useQuery({
    enabled: enabled && Boolean(communityId && relayUrl),
    queryKey: ["cofounder-lead-status", communityId, relayUrl],
    queryFn: fetchCofounderLeadStatus,
    staleTime: 10_000,
    refetchInterval: 30_000,
    refetchOnWindowFocus: true,
  });
}
