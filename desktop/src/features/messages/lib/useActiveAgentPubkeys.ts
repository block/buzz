import * as React from "react";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function useActiveAgentPubkeys(
  managedAgents?: readonly ManagedAgent[],
  relayAgents?: readonly RelayAgent[],
): ReadonlySet<string> {
  return React.useMemo(
    () =>
      new Set([
        ...(managedAgents ?? [])
          .filter(
            (agent) =>
              agent.status === "running" || agent.status === "deployed",
          )
          .map((agent) => normalizePubkey(agent.pubkey)),
        ...(relayAgents ?? [])
          .filter(
            (agent) => agent.status === "online" || agent.status === "away",
          )
          .map((agent) => normalizePubkey(agent.pubkey)),
      ]),
    [managedAgents, relayAgents],
  );
}
