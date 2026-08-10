import type { RespondToMode } from "./types";

export type RelayAgent = {
  pubkey: string;
  name: string;
  /** Optional model label carried by newer kind:10100 directory records. */
  model?: string | null;
  agentType: string;
  channels: string[];
  channelIds: string[];
  capabilities: string[];
  status: "online" | "away" | "offline";
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
};

export type RawRelayAgent = {
  pubkey: string;
  name: string;
  model?: string | null;
  model_label?: string | null;
  agent_type: string;
  channels: string[];
  channel_ids: string[];
  capabilities: string[];
  status: RelayAgent["status"];
  respond_to?: RelayAgent["respondTo"];
  respond_to_allowlist?: string[];
};

export function fromRawRelayAgent(agent: RawRelayAgent): RelayAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    model: agent.model ?? agent.model_label ?? null,
    agentType: agent.agent_type,
    channels: agent.channels,
    channelIds: agent.channel_ids ?? [],
    capabilities: agent.capabilities,
    status: agent.status,
    respondTo: agent.respond_to ?? null,
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  };
}
