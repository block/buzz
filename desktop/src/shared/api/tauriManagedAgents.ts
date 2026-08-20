import { invokeTauri } from "@/shared/api/tauri";
import type {
  CreateManagedAgentInput,
  ManagedAgent,
  ManagedAgentBackend,
  ManagedAgentRuntimeStatus,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import type { RestartDiffEntry as RawRestartDiffEntry } from "./restartDiff";

export type RawManagedAgent = {
  pubkey: string;
  name: string;
  persona_id: string | null;
  // Optional: pre-feature fixtures may omit it. The record's harness/runtime id.
  runtime?: string | null;
  team_id?: string | null;
  relay_url: string;
  acp_command: string;
  agent_command: string;
  agent_command_override?: string | null;
  agent_args: string[];
  mcp_command: string;
  turn_timeout_seconds: number;
  idle_timeout_seconds: number | null;
  max_turn_duration_seconds: number | null;
  parallelism: number;
  system_prompt: string | null;
  avatar_url?: string | null;
  model: string | null;
  model_source?: ManagedAgent["modelSource"];
  provider: string | null;
  persona_out_of_date: boolean;
  persona_orphaned: boolean;
  needs_restart: boolean;
  restart_diff?: RawRestartDiffEntry[];
  env_vars?: Record<string, string>;
  status: ManagedAgent["status"];
  pid: number | null;
  created_at: string;
  updated_at: string;
  last_started_at: string | null;
  last_stopped_at: string | null;
  last_exit_code: number | null;
  last_error: string | null;
  last_error_code: number | null;
  log_path: string;
  start_on_app_launch: boolean;
  auto_restart_on_config_change?: boolean;
  backend: ManagedAgentBackend;
  backend_agent_id: string | null;
  // Pre-feature fixtures may omit these; mapped to "owner-only"/[] in fromRawManagedAgent.
  respond_to?: ManagedAgent["respondTo"];
  respond_to_allowlist?: string[];
};

type RawCreateManagedAgentResponse = {
  agent: RawManagedAgent;
  private_key_nsec: string;
  profile_sync_error: string | null;
  spawn_error: string | null;
};

type RawManagedAgentLog = {
  content: string;
  log_path: string;
};

export function fromRawManagedAgent(agent: RawManagedAgent): ManagedAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    personaId: agent.persona_id,
    runtime: agent.runtime ?? null,
    teamId: agent.team_id ?? null,
    relayUrl: agent.relay_url,
    acpCommand: agent.acp_command,
    agentCommand: agent.agent_command,
    agentCommandOverride: agent.agent_command_override ?? null,
    agentArgs: agent.agent_args,
    mcpCommand: agent.mcp_command,
    turnTimeoutSeconds: agent.turn_timeout_seconds,
    idleTimeoutSeconds: agent.idle_timeout_seconds,
    maxTurnDurationSeconds: agent.max_turn_duration_seconds,
    parallelism: agent.parallelism,
    systemPrompt: agent.system_prompt,
    avatarUrl: agent.avatar_url ?? null,
    model: agent.model,
    modelSource: agent.model_source ?? null,
    provider: agent.provider ?? null,
    personaOutOfDate: agent.persona_out_of_date ?? false,
    personaOrphaned: agent.persona_orphaned ?? false,
    needsRestart: agent.needs_restart ?? false,
    restartDiff: agent.restart_diff ?? [],
    envVars: agent.env_vars ?? {},
    status: agent.status,
    pid: agent.pid,
    createdAt: agent.created_at,
    updatedAt: agent.updated_at,
    lastStartedAt: agent.last_started_at,
    lastStoppedAt: agent.last_stopped_at,
    lastExitCode: agent.last_exit_code,
    lastError: agent.last_error,
    lastErrorCode: agent.last_error_code ?? null,
    logPath: agent.log_path,
    startOnAppLaunch: agent.start_on_app_launch,
    autoRestartOnConfigChange: agent.auto_restart_on_config_change ?? true,
    backend: agent.backend,
    backendAgentId: agent.backend_agent_id,
    respondTo: agent.respond_to ?? "owner-only",
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  };
}

export async function listManagedAgents(): Promise<ManagedAgent[]> {
  return (await invokeTauri<RawManagedAgent[]>("list_managed_agents")).map(
    fromRawManagedAgent,
  );
}

export async function createManagedAgent(input: CreateManagedAgentInput) {
  const response = await invokeTauri<RawCreateManagedAgentResponse>(
    "create_managed_agent",
    {
      input: {
        name: input.name,
        personaId: input.personaId,
        teamId: input.teamId,
        relayUrl: input.relayUrl,
        acpCommand: input.acpCommand,
        agentCommand: input.agentCommand,
        harnessOverride: input.harnessOverride ?? false,
        agentArgs: input.agentArgs,
        mcpCommand: input.mcpCommand,
        turnTimeoutSeconds: input.turnTimeoutSeconds,
        idleTimeoutSeconds: input.idleTimeoutSeconds,
        maxTurnDurationSeconds: input.maxTurnDurationSeconds,
        parallelism: input.parallelism,
        systemPrompt: input.systemPrompt,
        avatarUrl: input.avatarUrl,
        model: input.model,
        provider: input.provider,
        envVars: input.envVars ?? {},
        spawnAfterCreate: input.spawnAfterCreate,
        startOnAppLaunch: input.startOnAppLaunch,
        backend: input.backend,
        respondTo: input.respondTo,
        respondToAllowlist: input.respondToAllowlist,
        relayMesh: input.relayMesh,
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

export async function deleteManagedAgent(
  pubkey: string,
  forceRemoteDelete?: boolean,
): Promise<void> {
  await invokeTauri("delete_managed_agent", {
    pubkey,
    forceRemoteDelete: forceRemoteDelete ?? null,
  });
}

export async function getManagedAgentLog(pubkey: string, lineCount?: number) {
  const response = await invokeTauri<RawManagedAgentLog>(
    "get_managed_agent_log",
    {
      pubkey,
      lineCount,
    },
  );

  return {
    content: response.content,
    logPath: response.log_path,
  };
}

type RawUpdateManagedAgentResponse = {
  agent: RawManagedAgent;
  profile_sync_error: string | null;
};

export async function updateManagedAgent(
  input: UpdateManagedAgentInput,
): Promise<{ agent: ManagedAgent; profileSyncError: string | null }> {
  const response = await invokeTauri<RawUpdateManagedAgentResponse>(
    "update_managed_agent",
    { input },
  );
  return {
    agent: fromRawManagedAgent(response.agent),
    profileSyncError: response.profile_sync_error,
  };
}

export async function startManagedAgent(
  pubkey: string,
  options?: {
    /** Tenant scope captured by the caller before its first await; the
     * backend fails closed before any spawn/deploy side effect when the
     * active community no longer matches. */
    expectedRelayUrl?: string;
    /** Signer identity captured with the relay scope; the backend fails
     * closed when the active workspace identity no longer matches. */
    expectedSignerPubkey?: string;
  },
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
    expectedRelayUrl: options?.expectedRelayUrl ?? null,
    expectedSignerPubkey: options?.expectedSignerPubkey ?? null,
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

/**
 * B5: persist the canonical startup effort for a local managed agent. Applied
 * as `BUZZ_ACP_EFFORT_LEVEL` at the next spawn. Pass `null` to clear (reverts
 * to the adapter default). Rejects non-local agents.
 */
export async function persistAgentEffortLevel(
  pubkey: string,
  effortLevel: string | null,
): Promise<void> {
  return invokeTauri<void>("persist_agent_effort_level", {
    pubkey,
    effortLevel,
  });
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
