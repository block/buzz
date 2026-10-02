import { invokeTauri } from "@/shared/api/tauri";
import type { RelayAgent } from "@/shared/api/types";

type RawRelayAgent = {
  pubkey: string;
  owner_pubkey?: string | null;
  name: string;
  agent_type: string;
  channels: string[];
  channel_ids: string[];
  capabilities: string[];
  status: RelayAgent["status"];
  respond_to?: RelayAgent["respondTo"];
  respond_to_allowlist?: string[];
};

export async function revalidateRelayAgents(
  pubkeys: string[],
  channelId?: string,
): Promise<RelayAgent[]> {
  const agents = await invokeTauri<RawRelayAgent[]>("revalidate_relay_agents", {
    pubkeys,
    channelId,
  });
  return agents.map((agent) => ({
    pubkey: agent.pubkey,
    ownerPubkey: agent.owner_pubkey ?? null,
    name: agent.name,
    agentType: agent.agent_type,
    channels: agent.channels,
    channelIds: agent.channel_ids ?? [],
    capabilities: agent.capabilities,
    status: agent.status,
    respondTo: agent.respond_to ?? null,
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  }));
}
