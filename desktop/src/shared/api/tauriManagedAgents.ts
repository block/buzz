import {
  fromRawManagedAgent,
  invokeTauri,
  type RawManagedAgent,
} from "@/shared/api/tauri";
import type {
  ManagedAgent,
  ManagedAgentRuntimeStatus,
  SelfUpdateField,
} from "@/shared/api/types";

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
    /** Unix-seconds replay floor for a publish-first mention send: the
     * spawned harness's first REQ replays at least back to this moment, so
     * the already-published triggering message lands in its window however
     * long the spawn takes. Local spawns receive it as process env; provider
     * deploys carry it in the payload's launch.policy_env. */
    replayFloorUnix?: number;
  },
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
    expectedRelayUrl: options?.expectedRelayUrl ?? null,
    expectedSignerPubkey: options?.expectedSignerPubkey ?? null,
    replayFloorUnix: options?.replayFloorUnix ?? null,
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

export async function setManagedAgentSelfUpdateFields(
  pubkey: string,
  selfUpdateFields: readonly SelfUpdateField[],
): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>(
    "set_managed_agent_self_update_fields",
    {
      pubkey,
      selfUpdateFields,
    },
  );
  return fromRawManagedAgent(response);
}

/** The update half of an `agent_management_request`, as the backend reads it. */
export type AgentSelfUpdateDraft = {
  channelId: string;
  agentName: string;
  displayName?: string;
  systemPrompt?: string;
  runtime?: string;
  provider?: string;
  model?: string;
  respondTo?: string;
};

export type AgentSelfUpdateOutcome =
  | {
      outcome: "applied";
      agent_pubkey: string;
      persona_id: string;
      display_name: string;
      fields: SelfUpdateField[];
    }
  | { outcome: "review"; reason: string };

/**
 * Ask the backend to apply an agent's own draft-update under its
 * `selfUpdateFields` policy. `review` is the normal answer and means the
 * caller should open the owner-review form exactly as before; `applied`
 * means the definition was saved and the auto-restart policy owns the rest.
 */
export async function applyAgentSelfUpdate(
  agentPubkey: string,
  draft: AgentSelfUpdateDraft,
  issuedAt: string | null,
): Promise<AgentSelfUpdateOutcome> {
  return invokeTauri<AgentSelfUpdateOutcome>("apply_agent_self_update", {
    agentPubkey,
    draft,
    issuedAt,
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
