import type { Channel, ManagedAgent } from "@/shared/api/types";
import type { AgentManagementRequest } from "./agentManagement";

export type QueuedAgentManagementRequest = {
  agentPubkey: string;
  request: AgentManagementRequest;
};

export type AgentManagementRequestQueue = {
  active: QueuedAgentManagementRequest | null;
  queued: QueuedAgentManagementRequest[];
};

export function enqueueAgentManagementRequest(
  state: AgentManagementRequestQueue,
  incoming: QueuedAgentManagementRequest,
): AgentManagementRequestQueue {
  if (state.active === null) {
    return { active: incoming, queued: state.queued };
  }
  return { active: state.active, queued: [...state.queued, incoming] };
}

export function advanceAgentManagementRequest(
  state: AgentManagementRequestQueue,
): AgentManagementRequestQueue {
  const [active, ...queued] = state.queued;
  return { active: active ?? null, queued };
}

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
