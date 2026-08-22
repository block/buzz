import type {
  AcpRuntime,
  AgentPersona,
  CreateManagedAgentInput,
  ManagedAgent,
} from "@/shared/api/types";
import { commandsMatch } from "../agentReuse";
import {
  getDefaultPersonaRuntime,
  resolvePersonaRuntime,
} from "./resolvePersonaRuntime";

export type TeamDeployRuntimePick = Pick<
  AcpRuntime,
  "id" | "label" | "command" | "defaultArgs" | "mcpCommand"
>;

export type TeamPersonaDeployReady = {
  status: "ready";
  runtime: TeamDeployRuntimePick;
  agentCommand: string;
  harnessOverride: boolean;
  agentArgs: string[];
};

export type TeamPersonaDeploySetupRequired = {
  status: "setup-required";
  reason: string;
};

export type TeamPersonaDeployPlan =
  | TeamPersonaDeployReady
  | TeamPersonaDeploySetupRequired;

export type TeamDeploySourceQuery = {
  isPending: boolean;
  isError: boolean;
  isFetched: boolean;
};

export type TeamDeploySourcePick =
  | { status: "none" }
  | { status: "ready"; source: ManagedAgent }
  | { status: "ambiguous" };

function sourceRuntimeConfiguration(agent: ManagedAgent): string {
  return JSON.stringify([
    agent.agentCommandOverride?.trim() || null,
    agent.agentArgs ?? [],
  ]);
}

/**
 * Personal (non-team) **local** instance for this persona. Provider-backed
 * agents are not portable runtime intent: their command/path is an execution
 * target, not a harness pin we can mint onto a new local clone.
 */
export function sourceConfigurationForPersona(
  managedAgents: readonly ManagedAgent[],
  personaId: string,
): TeamDeploySourcePick {
  const matches = managedAgents.filter(
    (agent) =>
      agent.personaId === personaId &&
      !agent.teamId &&
      agent.backend?.type === "local",
  );
  const source = matches[0];
  if (!source) {
    return { status: "none" };
  }
  const configuration = sourceRuntimeConfiguration(source);
  if (
    matches.some(
      (candidate) => sourceRuntimeConfiguration(candidate) !== configuration,
    )
  ) {
    return { status: "ambiguous" };
  }
  return { status: "ready", source };
}

export function isTeamDeploySourceReady(query: TeamDeploySourceQuery): {
  ready: boolean;
  blockReason: "loading" | "error" | null;
} {
  if (query.isPending || !query.isFetched) {
    return { ready: false, blockReason: "loading" };
  }
  if (query.isError) {
    return { ready: false, blockReason: "error" };
  }
  return { ready: true, blockReason: null };
}

/**
 * Prefer the personal local instance's explicit harness pin when minting a
 * team clone. Team deploy has no runtime selector. An unresolved pin must
 * fail with Setup required instead of silently falling back to the global
 * default (block/buzz#5694).
 */
export function runtimeForTeamPersonaDeploy(input: {
  persona: AgentPersona;
  runtimes: readonly AcpRuntime[];
  defaultProvider: AcpRuntime | null;
  managedAgents: readonly ManagedAgent[];
}): TeamPersonaDeployPlan {
  const sourcePick = sourceConfigurationForPersona(
    input.managedAgents,
    input.persona.id,
  );
  if (sourcePick.status === "ambiguous") {
    return {
      status: "setup-required",
      reason: `Setup required: ${input.persona.displayName}'s personal agents use different runtime settings.`,
    };
  }
  const source = sourcePick.status === "ready" ? sourcePick.source : undefined;
  const override = source?.agentCommandOverride?.trim();
  if (override) {
    const matching = input.runtimes.find(
      (runtime) =>
        (runtime.command != null && commandsMatch(runtime.command, override)) ||
        runtime.id === override,
    );
    if (!matching) {
      return {
        status: "setup-required",
        reason: `Setup required: ${input.persona.displayName}'s pinned runtime is not available locally.`,
      };
    }
    return {
      status: "ready",
      runtime: matching,
      agentCommand: override,
      harnessOverride: true,
      agentArgs: [...(source?.agentArgs ?? [])],
    };
  }

  const { runtime: personaRuntime } = resolvePersonaRuntime(
    input.persona.runtime,
    input.runtimes,
    input.defaultProvider,
  );
  const runtime = personaRuntime ?? input.defaultProvider;
  if (!runtime?.command) {
    return {
      status: "setup-required",
      reason: `Setup required: no runnable local runtime is available for ${input.persona.displayName}.`,
    };
  }
  return {
    status: "ready",
    runtime,
    agentCommand: runtime.command,
    harnessOverride: false,
    agentArgs: [],
  };
}

/**
 * Create-input fields that `provisionChannelManagedAgent` must forward so
 * spawn derives command/args/MCP from the same pin the source local agent
 * used. `mcpCommand` is omitted: create derives MCP from the catalog by
 * command and ignores the request field.
 */
export function createInputForTeamPersonaDeploy(input: {
  persona: AgentPersona;
  teamId: string;
  plan: TeamPersonaDeployReady;
}): Pick<
  CreateManagedAgentInput,
  | "agentCommand"
  | "harnessOverride"
  | "agentArgs"
  | "backend"
  | "personaId"
  | "teamId"
  | "name"
> {
  return {
    name: input.persona.displayName,
    personaId: input.persona.id,
    teamId: input.teamId,
    agentCommand: input.plan.agentCommand,
    harnessOverride: input.plan.harnessOverride,
    agentArgs: input.plan.agentArgs,
    backend: { type: "local" },
  };
}

export function runtimeLabelForTeamDeployPlan(
  plan: TeamPersonaDeployPlan | undefined,
  sourceBlockReason: "loading" | "error" | null,
): string {
  if (!plan) {
    if (sourceBlockReason === "loading") {
      return "Loading runtime…";
    }
    if (sourceBlockReason === "error") {
      return "Runtime unavailable";
    }
    return "Runtime pending";
  }
  switch (plan.status) {
    case "ready":
      return plan.runtime.label;
    case "setup-required":
      return "Setup required";
    default: {
      const _exhaustive: never = plan;
      return _exhaustive;
    }
  }
}

export function defaultTeamDeployRuntime(
  runtimes: readonly AcpRuntime[],
  preferredRuntimeId?: string | null,
): AcpRuntime | null {
  return getDefaultPersonaRuntime(runtimes, preferredRuntimeId);
}
