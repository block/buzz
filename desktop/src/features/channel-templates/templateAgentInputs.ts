import type { CreateChannelManagedAgentInput } from "@/features/agents/channelAgents";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type {
  AcpRuntime,
  AgentPersona,
  AgentTeam,
  ChannelTemplate,
} from "@/shared/api/types";

type InitialLoadQuery<T> = {
  data: T | undefined;
  refetch: () => Promise<{ data: T | undefined; error: unknown }>;
};

/** Return cached query data, or wait for its initial request to finish. */
export async function dataAfterInitialLoad<T>(
  query: InitialLoadQuery<T>,
): Promise<T> {
  if (query.data !== undefined) return query.data;

  const result = await query.refetch();
  if (result.data !== undefined) return result.data;

  throw (
    result.error ?? new Error("Required template data could not be loaded.")
  );
}

/**
 * TemplateBackend omits `config` — supply an empty object for provider backends.
 */
function toManagedBackend(
  backend: ChannelTemplate["agents"]["personas"][number]["backend"],
): CreateChannelManagedAgentInput["backend"] {
  if (!backend || backend.type === "local") return { type: "local" };
  return { type: "provider", id: backend.id, config: {} };
}

export function buildTemplateAgentInputs({
  template,
  personas,
  teams,
  runtimes,
  lastRuntimeId,
}: {
  template: ChannelTemplate;
  personas: AgentPersona[];
  teams: readonly AgentTeam[];
  runtimes: readonly AcpRuntime[];
  lastRuntimeId: string | null;
}): CreateChannelManagedAgentInput[] {
  const defaultProvider =
    runtimes.find((runtime) => runtime.id === lastRuntimeId) ??
    runtimes[0] ??
    null;
  if (!defaultProvider) return [];

  const directPersonaIds = new Set<string>();
  const teamPersonaKeys = new Set<string>();
  const inputs: CreateChannelManagedAgentInput[] = [];

  for (const entry of template.agents.personas) {
    const persona = personas.find(
      (candidate) => candidate.id === entry.personaId,
    );
    if (!persona || directPersonaIds.has(persona.id)) continue;
    directPersonaIds.add(persona.id);
    const resolved = resolvePersonaRuntime(
      entry.runtime ?? persona.runtime,
      runtimes,
      defaultProvider,
    );
    inputs.push({
      runtime: resolved.runtime ?? defaultProvider,
      name: persona.displayName,
      personaId: persona.id,
      systemPrompt: persona.systemPrompt,
      avatarUrl: persona.avatarUrl ?? undefined,
      model: entry.model ?? persona.model ?? undefined,
      role: "bot",
      backend: toManagedBackend(entry.backend),
    });
  }

  for (const teamEntry of template.agents.teams) {
    const team = teams.find((candidate) => candidate.id === teamEntry.teamId);
    if (!team) continue;
    const { resolvedPersonas } = resolveTeamPersonas(team, personas);
    for (const persona of resolvedPersonas) {
      const teamPersonaKey = `${team.id}\0${persona.id}`;
      if (teamPersonaKeys.has(teamPersonaKey)) continue;
      teamPersonaKeys.add(teamPersonaKey);
      const resolved = resolvePersonaRuntime(
        teamEntry.runtime ?? persona.runtime,
        runtimes,
        defaultProvider,
      );
      inputs.push({
        runtime: resolved.runtime ?? defaultProvider,
        name: persona.displayName,
        personaId: persona.id,
        teamId: team.id,
        forceNewInstance: true,
        systemPrompt: persona.systemPrompt,
        avatarUrl: persona.avatarUrl ?? undefined,
        model: teamEntry.model ?? persona.model ?? undefined,
        role: "bot",
        backend: toManagedBackend(teamEntry.backend),
      });
    }
  }

  return inputs;
}
