import type { Channel, ManagedAgent } from "@/shared/api/types";

/**
 * Defers the trust decision until both ownership and channel membership have
 * initialized. A draft may open only when its owned sender and the owner share
 * the claimed originating channel.
 */
export function classifyAgentManagementOrigin(
  agents: readonly Pick<ManagedAgent, "pubkey">[] | undefined,
  channels:
    | readonly Pick<Channel, "id" | "isMember" | "memberPubkeys">[]
    | undefined,
  agentPubkey: string,
  channelId: string,
): "buffer" | "accept" | "reject" {
  if (agents === undefined || channels === undefined) return "buffer";
  const normalizedAgentPubkey = agentPubkey.toLowerCase();
  const isOwnedAgent = agents.some(
    (agent) => agent.pubkey.toLowerCase() === normalizedAgentPubkey,
  );
  const originChannel = channels.find((channel) => channel.id === channelId);
  return isOwnedAgent &&
    originChannel?.isMember === true &&
    originChannel.memberPubkeys.some(
      (pubkey) => pubkey.toLowerCase() === normalizedAgentPubkey,
    )
    ? "accept"
    : "reject";
}
