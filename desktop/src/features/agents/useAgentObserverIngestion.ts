import * as React from "react";

import { useActiveAgentTurnsBridge } from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type IngestionAgent = Pick<ManagedAgent, "pubkey" | "status"> & {
  canProcessOwnerFrames: boolean;
};

/**
 * Combine locally managed agents with every directory-listed relay agent into
 * one ingestion list.
 *
 * Managed agents keep their real status and owner-frame privileges. Relay
 * agents are treated as `deployed` so requester-addressed turn frames can be
 * decrypted even when the current identity is not their owner. Declared
 * ownership is retained separately to gate control/config/lifecycle side
 * effects in the observer store.
 */
export function combineObserverIngestionAgents(
  managedAgents: readonly Pick<ManagedAgent, "pubkey" | "status">[],
  relayAgentPubkeys: readonly string[],
  ownerByPubkey: ReadonlyMap<string, string>,
  currentPubkey: string | null | undefined,
): IngestionAgent[] {
  const managed = managedAgents.map((agent) => ({
    pubkey: agent.pubkey,
    status: agent.status,
    canProcessOwnerFrames: true,
  }));

  const managedSet = new Set(
    managed.map((agent) => normalizePubkey(agent.pubkey)),
  );
  const me = currentPubkey ? normalizePubkey(currentPubkey) : null;
  const relay: IngestionAgent[] = [];
  for (const pubkey of relayAgentPubkeys) {
    const key = normalizePubkey(pubkey);
    if (managedSet.has(key)) {
      continue;
    }
    const owner = ownerByPubkey.get(key);
    relay.push({
      pubkey,
      status: "deployed" as const,
      canProcessOwnerFrames: Boolean(
        me && owner && normalizePubkey(owner) === me,
      ),
    });
  }
  return [...managed, ...relay];
}

/**
 * App-level owner-global observer ingestion.
 *
 * Mounted once in AppShell so observer frames (kind 24200) are received,
 * decrypted, and folded into the derived active-turns store regardless of
 * which screen or panel happens to be open. Individual surfaces read from the
 * stores; none of them need to mount their own bridge for ingestion to work.
 *
 * This is the product invariant: owner and requester-addressed turn activity
 * is ingested app-wide, not only while a panel that happens to mount a bridge
 * is open. Owner-only side effects remain separately gated.
 *
 * Mounts before identity resolves by design: while `currentPubkey` is still
 * `undefined`, relay agents are already registered for requester telemetry;
 * ownership privileges are folded in after identity and profiles resolve.
 * Do not gate this hook on identity/startup readiness — that would drop
 * managed-agent observer coverage during startup.
 */
export function useAgentObserverIngestion() {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;

  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data;

  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgentPubkeys = React.useMemo(
    () => (relayAgentsQuery.data ?? []).map((agent) => agent.pubkey),
    [relayAgentsQuery.data],
  );

  const profilesQuery = useUsersBatchQuery(relayAgentPubkeys, {
    enabled: Boolean(currentPubkey) && relayAgentPubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;

  const ingestionAgents = React.useMemo(() => {
    const ownerByPubkey = new Map<string, string>();
    for (const [pubkey, summary] of Object.entries(profiles ?? {})) {
      if (summary.ownerPubkey) {
        // Store both key and value normalized so lookups and ownership
        // comparisons never depend on the casing the relay happened to send.
        ownerByPubkey.set(
          normalizePubkey(pubkey),
          normalizePubkey(summary.ownerPubkey),
        );
      }
    }
    return combineObserverIngestionAgents(
      managedAgents ?? [],
      relayAgentPubkeys,
      ownerByPubkey,
      currentPubkey,
    );
  }, [currentPubkey, managedAgents, profiles, relayAgentPubkeys]);

  const ownerFrameAgentPubkeys = React.useMemo(
    () =>
      ingestionAgents
        .filter((agent) => agent.canProcessOwnerFrames)
        .map((agent) => agent.pubkey),
    [ingestionAgents],
  );

  useManagedAgentObserverBridge(ingestionAgents, ownerFrameAgentPubkeys);
  useActiveAgentTurnsBridge(ingestionAgents);
}
