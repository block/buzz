import * as React from "react";

import { useActiveAgentTurnsBridge } from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type IngestionAgent = Pick<ManagedAgent, "pubkey" | "status">;

/**
 * Combine locally managed agents with relay-visible agents into one ingestion list.
 *
 * Managed agents keep their real status; relay agents that are not managed
 * locally are treated as `deployed`. The harness sends each accepted turn's
 * encrypted observer frames to both the owner and the request authors, so a
 * non-owner must trust the relay-visible agent pubkey before decrypting those
 * author-addressed frames.
 */
export function combineObserverIngestionAgents(
  managedAgents: readonly IngestionAgent[],
  relayAgentPubkeys: readonly string[],
): IngestionAgent[] {
  const managed = managedAgents.map((agent) => ({
    pubkey: agent.pubkey,
    status: agent.status,
  }));
  const managedSet = new Set(
    managed.map((agent) => normalizePubkey(agent.pubkey)),
  );
  const relayAgents: IngestionAgent[] = [];
  for (const pubkey of relayAgentPubkeys) {
    const key = normalizePubkey(pubkey);
    if (managedSet.has(key)) {
      continue;
    }
    relayAgents.push({ pubkey, status: "deployed" as const });
  }
  return [...managed, ...relayAgents];
}

/**
 * App-level observer ingestion for owned and shared agent turns.
 *
 * Mounted once in AppShell so observer frames (kind 24200) are received,
 * decrypted, and folded into the derived active-turns store regardless of
 * which screen or panel happens to be open. Individual surfaces read from the
 * stores; none of them need to mount their own bridge for ingestion to work.
 *
 * This is the product invariant: owned agent activity and encrypted turn
 * activity explicitly shared with the current request author are ingested
 * app-wide, not only while a panel that mounts a bridge is open.
 *
 * Do not gate this hook on screen or panel readiness; doing so would drop
 * observer coverage during startup and navigation.
 */
export function useAgentObserverIngestion() {
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data;

  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgentPubkeys = React.useMemo(
    () => (relayAgentsQuery.data ?? []).map((agent) => agent.pubkey),
    [relayAgentsQuery.data],
  );

  const ingestionAgents = React.useMemo(() => {
    return combineObserverIngestionAgents(
      managedAgents ?? [],
      relayAgentPubkeys,
    );
  }, [managedAgents, relayAgentPubkeys]);

  useManagedAgentObserverBridge(ingestionAgents);
  useActiveAgentTurnsBridge(ingestionAgents);
}
