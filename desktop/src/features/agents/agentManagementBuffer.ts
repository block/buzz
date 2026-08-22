import type { Channel, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

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
  const normalizedAgentPubkey = normalizePubkey(agentPubkey);
  const isOwnedAgent = agents.some(
    (agent) => normalizePubkey(agent.pubkey) === normalizedAgentPubkey,
  );
  const originChannel = channels.find((channel) => channel.id === channelId);
  return isOwnedAgent &&
    originChannel?.isMember === true &&
    originChannel.memberPubkeys.some(
      (pubkey) => normalizePubkey(pubkey) === normalizedAgentPubkey,
    )
    ? "accept"
    : "reject";
}
