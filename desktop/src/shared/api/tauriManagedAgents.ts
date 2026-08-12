import {
  fromRawManagedAgent,
  invokeTauri,
  type RawManagedAgent,
} from "@/shared/api/tauri";
import type {
  ExternalAgentRuntime,
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";

export async function startManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
  });
  return fromRawManagedAgent(response);
}

export async function stopManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("stop_managed_agent", {
    pubkey,
  });
  return fromRawManagedAgent(response);
}

export async function setManagedAgentStartOnAppLaunch(
  pubkey: string,
  startOnAppLaunch: boolean,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_start_on_app_launch",
    {
      pubkey,
      startOnAppLaunch,
    },
  );
  return fromRawManagedAgent(response);
}

export async function setManagedAgentAutoRestart(
  pubkey: string,
  autoRestartOnConfigChange: boolean,
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_auto_restart",
    {
      pubkey,
      autoRestartOnConfigChange,
    },
  );
  return fromRawManagedAgent(response);
}

export async function listManagedAgentRuntimes(): Promise<
  ManagedAgentRuntimeStatus[]
> {
  return invokeTauri<ManagedAgentRuntimeStatus[]>(
    "list_managed_agent_runtimes",
  );
}

export async function startManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("start_managed_agent_runtime", { pubkey, relayUrl });
}

export async function stopManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("stop_managed_agent_runtime", { pubkey, relayUrl });
}

export async function restartManagedAgentRuntime(
  pubkey: string,
  relayUrl: string,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("restart_managed_agent_runtime", { pubkey, relayUrl });
}

export async function putManagedAgentRuntimeLifecycle(
  outerPubkey: string,
  payload: unknown,
): Promise<ManagedAgentRuntimeStatus> {
  return invokeTauri("put_managed_agent_runtime_lifecycle", {
    outerPubkey,
    payload,
  });
}

export async function reconcileManagedAgentRuntimes(
  communities: readonly { relayUrl: string }[],
): Promise<ManagedAgentRuntimeStatus[]> {
  return invokeTauri("reconcile_managed_agent_runtimes", { communities });
}

export type RegisterExternalAgentRuntimeInput = {
  agentPubkey: string;
  ownerAuthTag: string;
  name: string;
  purpose: string;
  deploymentScope: string;
  runnerOwner: string;
  healthSource: string;
  shutdownPath: string;
  allowedChannels: string[];
  mentionOnly?: boolean;
  mentionFilter?: boolean;
  rateLimitPerMinute: number;
  retirementDate: string;
};

export async function listExternalAgentRuntimes(): Promise<
  ExternalAgentRuntime[]
> {
  return invokeTauri<ExternalAgentRuntime[]>("list_external_agent_runtimes");
}

export async function registerExternalAgentRuntime(
  input: RegisterExternalAgentRuntimeInput,
): Promise<ExternalAgentRuntime> {
  return invokeTauri<ExternalAgentRuntime>("register_external_agent_runtime", {
    input,
  });
}

export async function archiveExternalAgentRuntime(
  agentPubkey: string,
): Promise<ExternalAgentRuntime> {
  return invokeTauri<ExternalAgentRuntime>("archive_external_agent_runtime", {
    agentPubkey,
  });
}
