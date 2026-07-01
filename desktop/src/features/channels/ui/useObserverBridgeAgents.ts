import * as React from "react";

import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type ObserverBridgeAgent = Pick<ManagedAgent, "pubkey" | "status">;

export function useObserverBridgeAgents({
  currentPubkey,
  managedAgents,
  openAgentSessionPubkey,
  profilePanelPubkey,
  profiles,
  relayAgents,
}: {
  currentPubkey: string | undefined;
  managedAgents: readonly ObserverBridgeAgent[];
  openAgentSessionPubkey: string | null;
  profilePanelPubkey: string | null;
  profiles: UserProfileLookup | undefined;
  relayAgents: readonly Pick<RelayAgent, "pubkey">[];
}): ObserverBridgeAgent[] {
  return React.useMemo(() => {
    const byPubkey = new Map<string, ObserverBridgeAgent>(
      managedAgents.map((agent) => [normalizePubkey(agent.pubkey), agent]),
    );

    for (const agent of relayAgents) {
      const key = normalizePubkey(agent.pubkey);
      if (byPubkey.has(key)) continue;
      if (ownsAuthorAgent(profiles?.[key], currentPubkey)) {
        byPubkey.set(key, { pubkey: agent.pubkey, status: "deployed" });
      }
    }

    if (
      profilePanelPubkey &&
      openAgentSessionPubkey &&
      normalizePubkey(profilePanelPubkey) ===
        normalizePubkey(openAgentSessionPubkey) &&
      !byPubkey.has(normalizePubkey(profilePanelPubkey))
    ) {
      byPubkey.set(normalizePubkey(profilePanelPubkey), {
        pubkey: profilePanelPubkey,
        status: "deployed",
      });
    }

    return [...byPubkey.values()];
  }, [
    currentPubkey,
    managedAgents,
    openAgentSessionPubkey,
    profilePanelPubkey,
    profiles,
    relayAgents,
  ]);
}
