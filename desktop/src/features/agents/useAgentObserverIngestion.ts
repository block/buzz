import * as React from "react";

import { useActiveAgentTurnsBridge } from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { useCommunityBotsQuery } from "@/features/community-bots/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type IngestionAgent = Pick<ManagedAgent, "pubkey" | "status">;

/**
 * Combine locally managed agents, relay agents the current identity
 * declared-owns (NIP-OA `ownerPubkey == me`), and catalog community bots into
 * one ingestion list.
 *
 * Managed agents keep their real status; owned relay agents and community bots
 * that are not managed locally are treated as `deployed` so the observer
 * subscription starts and their frames decrypt into `knownAgentPubkeys`.
 *
 * Community bots (OpenClaw / buzz-acp@korg etc.) publish kind 24200 frames
 * `#p`-addressed to `BUZZ_ACP_AGENT_OWNER`. Those frames arrive on the
 * owner-global subscription, but without registering the bot pubkey they are
 * dropped by the trusted-agent gate — which is why the ACP activity pane stays
 * empty after a successful mention. Catalog pubkeys close that gap.
 */
export function combineObserverIngestionAgents(
  managedAgents: readonly IngestionAgent[],
  relayAgentPubkeys: readonly string[],
  ownerByPubkey: ReadonlyMap<string, string>,
  currentPubkey: string | null | undefined,
  communityBotPubkeys: readonly string[] = [],
): IngestionAgent[] {
  const managed = managedAgents.map((agent) => ({
    pubkey: agent.pubkey,
    status: agent.status,
  }));
  if (!currentPubkey) {
    return managed;
  }

  const seen = new Set(managed.map((agent) => normalizePubkey(agent.pubkey)));
  const me = normalizePubkey(currentPubkey);
  const owned: IngestionAgent[] = [];
  for (const pubkey of relayAgentPubkeys) {
    const key = normalizePubkey(pubkey);
    if (seen.has(key)) {
      continue;
    }
    const owner = ownerByPubkey.get(key);
    if (owner && normalizePubkey(owner) === me) {
      owned.push({ pubkey, status: "deployed" as const });
      seen.add(key);
    }
  }

  const community: IngestionAgent[] = [];
  for (const pubkey of communityBotPubkeys) {
    const key = normalizePubkey(pubkey);
    if (!key || seen.has(key)) {
      continue;
    }
    community.push({ pubkey, status: "deployed" as const });
    seen.add(key);
  }

  return [...managed, ...owned, ...community];
}

/**
 * App-level owner-global observer ingestion.
 *
 * Mounted once in AppShell so observer frames (kind 24200) are received,
 * decrypted, and folded into the derived active-turns store regardless of
 * which screen or panel happens to be open. Individual surfaces read from the
 * stores; none of them need to mount their own bridge for ingestion to work.
 *
 * This is the product invariant: if the current identity owns an agent (local
 * managed agent or declared-owned relay agent) — or has installed a community
 * bot that telemeters to this owner — its turn activity is ingested app-wide,
 * not only while a panel that happens to mount a bridge is open.
 *
 * Mounts before identity resolves by design: while `currentPubkey` is still
 * `undefined`, `combineObserverIngestionAgents` returns managed agents only,
 * and relay-owned / community agents are folded in on the render after identity
 * arrives. Do not gate this hook on identity/startup readiness — that would
 * drop managed-agent observer coverage during startup.
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

  const communityBotsQuery = useCommunityBotsQuery(Boolean(currentPubkey));
  const communityBotPubkeys = React.useMemo(
    () => (communityBotsQuery.data ?? []).map((bot) => bot.pubkey),
    [communityBotsQuery.data],
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
      communityBotPubkeys,
    );
  }, [
    communityBotPubkeys,
    currentPubkey,
    managedAgents,
    profiles,
    relayAgentPubkeys,
  ]);

  useManagedAgentObserverBridge(ingestionAgents);
  useActiveAgentTurnsBridge(ingestionAgents);
}
