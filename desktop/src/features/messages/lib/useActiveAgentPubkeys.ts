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
        // A provider's `deployed` receipt outlives a shut-down or reaped body,
        // so remote activity comes from the relay directory entry below.
        ...(managedAgents ?? [])
          .filter((agent) => agent.status === "running")
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
