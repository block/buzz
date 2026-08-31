import { hermesProfileNameFromAgent } from "./hermesProfileBinding";
import type { ManagedAgent } from "@/shared/api/types";

export function hermesTeamAgentForPersona({
  runtimeId,
  personaId,
  personaName,
  teamId,
  agents,
}: {
  runtimeId: string;
  personaId: string;
  personaName: string;
  teamId: string;
  agents: readonly ManagedAgent[];
}): ManagedAgent | null {
  if (runtimeId !== "hermes") return null;
  const candidates = agents.filter(
    (agent) =>
      agent.personaId === personaId &&
      hermesProfileNameFromAgent(agent) !== null,
  );
  if (candidates.length === 0) {
    throw new Error(
      `${personaName} uses Hermes. Connect its local profile before deploying this Team.`,
    );
  }
  if (candidates.length > 1) {
    throw new Error(
      `Multiple Buzz agents are bound to ${personaName}. Keep one profile-backed instance before deploying this Team.`,
    );
  }
  const agent = candidates[0];
  if (agent.teamId && agent.teamId !== teamId) {
    throw new Error(
      `${personaName} is already bound to another Team. A Hermes profile can belong to only one Team context.`,
    );
  }
  return agent;
}
