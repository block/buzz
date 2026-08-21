import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type AgentControlIdentity = { name: string; pubkey: string };

export function resolveInterruptibleAgents(
  activeAgentPubkeys: readonly string[],
  managedAgents: readonly Pick<ManagedAgent, "name" | "pubkey" | "status">[],
  relayAgents: readonly Pick<RelayAgent, "name" | "pubkey">[],
): AgentControlIdentity[] {
  const managedByPubkey = new Map(
    managedAgents
      .filter(
        (agent) => agent.status === "running" || agent.status === "deployed",
      )
      .map((agent) => [normalizePubkey(agent.pubkey), agent]),
  );
  const relayByPubkey = new Map(
    relayAgents.map((agent) => [normalizePubkey(agent.pubkey), agent]),
  );

  return activeAgentPubkeys.flatMap((pubkey) => {
    const key = normalizePubkey(pubkey);
    const agent = managedByPubkey.get(key) ?? relayByPubkey.get(key);
    return agent ? [{ name: agent.name, pubkey: agent.pubkey }] : [];
  });
}
