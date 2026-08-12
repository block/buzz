import type { AcpRuntime, AgentPersona, ManagedAgent } from "@/shared/api/types";
import {
  getDefaultPersonaRuntime,
  resolvePersonaRuntime,
} from "./resolvePersonaRuntime";

export type TeamDeployRuntimePick = Pick<
  AcpRuntime,
  "id" | "label" | "command" | "defaultArgs" | "mcpCommand"
>;

export type TeamPersonaDeployRuntime = {
  runtime: TeamDeployRuntimePick;
  harnessOverride: boolean;
};

/**
 * Prefer the personal (non-team) instance's explicit harness pin when
 * minting a team clone. Team deploy has no runtime selector, so without
 * this the clone falls back to `resolvePersonaRuntime` + the global
 * default and silently drops `agent_command_override`.
 */
export function sourceAgentForPersona(
  managedAgents: readonly ManagedAgent[],
  personaId: string,
): ManagedAgent | undefined {
  const matches = managedAgents.filter(
    (agent) => agent.personaId === personaId && !agent.teamId,
  );
  return (
    matches.find((agent) => Boolean(agent.agentCommandOverride?.trim())) ??
    matches[0]
  );
}

export function runtimeForTeamPersonaDeploy(input: {
  persona: AgentPersona;
  runtimes: readonly AcpRuntime[];
  defaultProvider: AcpRuntime | undefined;
  managedAgents: readonly ManagedAgent[];
}): TeamPersonaDeployRuntime | null {
  const source = sourceAgentForPersona(input.managedAgents, input.persona.id);
  const override = source?.agentCommandOverride?.trim();
  if (override) {
    const matching = input.runtimes.find(
      (runtime) => runtime.command === override || runtime.id === override,
    );
    if (matching) {
      return { runtime: matching, harnessOverride: true };
    }
    return {
      runtime: {
        id: "custom",
        label: override,
        command: override,
        defaultArgs: [],
        mcpCommand: source?.mcpCommand || "",
      },
      harnessOverride: true,
    };
  }

  const { runtime: personaRuntime } = resolvePersonaRuntime(
    input.persona.runtime,
    input.runtimes,
    input.defaultProvider,
  );
  const runtime = personaRuntime ?? input.defaultProvider;
  if (!runtime) {
    return null;
  }
  return { runtime, harnessOverride: false };
}

export function defaultTeamDeployRuntime(
  runtimes: readonly AcpRuntime[],
  preferredRuntimeId?: string | null,
): AcpRuntime | undefined {
  return getDefaultPersonaRuntime(runtimes, preferredRuntimeId);
}
