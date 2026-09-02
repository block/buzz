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
 * Combine locally managed agents with every relay agent discoverable through
 * the current identity's channel memberships.
 *
 * Managed agents keep their real status. Relay-only agents are treated as
 * deployed so the app-level observer subscription starts and shared frames
 * addressed to this identity are accepted. The relay still enforces the
 * p-tag and current channel membership before delivery.
 */
export function combineObserverIngestionAgents(
  managedAgents: readonly IngestionAgent[],
  relayAgentPubkeys: readonly string[],
): IngestionAgent[] {
  const combined = managedAgents.map((agent) => ({
    pubkey: agent.pubkey,
    status: agent.status,
  }));
  const known = new Set(combined.map((agent) => normalizePubkey(agent.pubkey)));
  for (const pubkey of relayAgentPubkeys) {
    const key = normalizePubkey(pubkey);
    if (known.has(key)) {
      continue;
    }
    known.add(key);
    combined.push({ pubkey, status: "deployed" });
  }
  return combined;
}

/**
 * App-level channel-scoped observer ingestion.
 *
 * Mounted once in AppShell so observer frames (kind 24200) are received,
 * decrypted, and folded into the derived active-turns store regardless of
 * which screen or panel happens to be open. Relay agent discovery is already
 * scoped to channels visible to the current identity; registering all of those
 * agents lets employees ingest shared frames without granting control rights.
 */
export function useAgentObserverIngestion() {
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data;

  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgentPubkeys = React.useMemo(
    () => (relayAgentsQuery.data ?? []).map((agent) => agent.pubkey),
    [relayAgentsQuery.data],
  );

  const ingestionAgents = React.useMemo(
    () =>
      combineObserverIngestionAgents(managedAgents ?? [], relayAgentPubkeys),
    [managedAgents, relayAgentPubkeys],
  );

  useManagedAgentObserverBridge(ingestionAgents);
  useActiveAgentTurnsBridge(ingestionAgents);
}
