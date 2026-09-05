import { invokeTauri } from "@/shared/api/tauri";
import type { RelayAgent } from "@/shared/api/types";

/** Wire shape of a relay agent directory entry. */
export type RawRelayAgent = {
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
  device_id?: string | null;
  device_label?: string | null;
};

/** Normalize a wire relay agent, defaulting fields absent on older payloads. */
export function fromRawRelayAgent(agent: RawRelayAgent): RelayAgent {
  return {
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
    deviceId: agent.device_id ?? null,
    deviceLabel: agent.device_label ?? null,
  };
}

/** List the agents visible in the viewer's relay agent directory. */
export async function listRelayAgents(): Promise<RelayAgent[]> {
  return (await invokeTauri<RawRelayAgent[]>("list_relay_agents")).map(
    fromRawRelayAgent,
  );
}

export async function revalidateRelayAgents(
  pubkeys: string[],
  channelId?: string,
): Promise<RelayAgent[]> {
  const agents = await invokeTauri<RawRelayAgent[]>("revalidate_relay_agents", {
    pubkeys,
    channelId,
  });
  return agents.map(fromRawRelayAgent);
}
