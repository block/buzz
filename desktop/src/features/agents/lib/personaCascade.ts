import type { ManagedAgent } from "@/shared/api/types";

/**
 * A cascade instance whose deployment outlives its record.
 *
 * `unitId` is the provider-side identifier written once on a successful deploy
 * (`backend_agent_id` — the systemd unit for the SSH provider). Nothing clears
 * it, because the provider protocol has no undeploy, which is precisely why
 * these instances need naming before a cascade: deleting them removes what this
 * app knows about them and nothing else.
 */
export type PersonaRemoteCascadeInstance = {
  name: string;
  unitId: string;
};

/**
 * Provider-backed instances in a persona's cascade that have a live deployment.
 *
 * Provider-backed instances that never completed a deploy are excluded — they
 * have no remote unit, so deleting them costs nothing and needs no disclosure.
 */
export function collectPersonaRemoteCascadeInstances(
  managedAgents: readonly ManagedAgent[],
  personaId: string,
): PersonaRemoteCascadeInstance[] {
  const instances: PersonaRemoteCascadeInstance[] = [];
  for (const agent of managedAgents) {
    if (agent.personaId !== personaId) continue;
    if (agent.backend.type !== "provider") continue;
    if (!agent.backendAgentId) continue;
    instances.push({ name: agent.name, unitId: agent.backendAgentId });
  }
  return instances;
}
