import { invokeTauri } from "@/shared/api/tauri";
import type { GlobalAgentConfigSaveResult } from "@/shared/api/types";

/** What Settings shows about the nono agent sandbox. */
export type AgentSandboxStatus = {
  /** The platform supports the sandbox (macOS and Linux). */
  supported: boolean;
  /** Agents launch sandboxed. */
  enabled: boolean;
  /** Why sandboxed agents cannot start right now; null when they can. */
  problem: string | null;
  /** The user-editable sandbox profile file. */
  profile_path: string | null;
};

/** Read the sandbox setting and run the checks a sandboxed launch runs. */
export async function getAgentSandboxStatus(): Promise<AgentSandboxStatus> {
  return invokeTauri<AgentSandboxStatus>("get_agent_sandbox_status");
}

/**
 * Turn the agent sandbox on or off. Running local agents restart so they
 * relaunch with the new setting.
 */
export async function setAgentSandboxEnabled(
  enabled: boolean,
): Promise<GlobalAgentConfigSaveResult> {
  return invokeTauri<GlobalAgentConfigSaveResult>("set_agent_sandbox_enabled", {
    enabled,
  });
}

/** Open the user-editable sandbox profile in the default app. */
export async function openAgentSandboxProfile(): Promise<void> {
  return invokeTauri<void>("open_agent_sandbox_profile");
}

/** Restore the user-editable sandbox profile to its default. */
export async function resetAgentSandboxProfile(): Promise<void> {
  return invokeTauri<void>("reset_agent_sandbox_profile");
}
