import { invokeTauri } from "@/shared/api/tauri";
import { fromRawManagedAgent, type RawManagedAgent } from "@/shared/api/tauri";

type RawCreateManagedAgentResponse = {
  agent: RawManagedAgent;
  private_key_nsec: string;
  profile_sync_error: string | null;
  spawn_error: string | null;
};

export async function spawnTempManagedAgent(input: {
  name: string;
  systemPrompt: string;
  channelId: string;
  parentAgentPubkey: string;
  ttlSeconds?: number;
}) {
  const response = await invokeTauri<RawCreateManagedAgentResponse>(
    "spawn_temp_managed_agent",
    {
      input: {
        name: input.name,
        systemPrompt: input.systemPrompt,
        channelId: input.channelId,
        parentAgentPubkey: input.parentAgentPubkey,
        ttlSeconds: input.ttlSeconds ?? null,
      },
    },
  );
  return {
    agent: fromRawManagedAgent(response.agent),
    privateKeyNsec: response.private_key_nsec,
    profileSyncError: response.profile_sync_error,
    spawnError: response.spawn_error,
  };
}

export async function destroyTempManagedAgent(input: {
  channelId: string;
  parentAgentPubkey: string;
  agentName: string;
}): Promise<void> {
  await invokeTauri("destroy_temp_managed_agent", {
    input: {
      channelId: input.channelId,
      parentAgentPubkey: input.parentAgentPubkey,
      agentName: input.agentName,
    },
  });
}

export async function killAllTempAgents(): Promise<number> {
  return invokeTauri<number>("kill_all_temp_agents");
}

export async function sweepExpiredTempAgents(): Promise<number> {
  return invokeTauri<number>("sweep_expired_temp_agents");
}
