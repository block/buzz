import type { ManagedAgent } from "@/shared/api/types";
import type { BestieAssignment } from "./api";

/** Resolve a durable assignment only when its local managed agent still exists. */
export function findAssignedLocalAgent(
  agents: ManagedAgent[],
  assignment: BestieAssignment | null | undefined,
) {
  if (!assignment) return null;
  return (
    agents.find(
      (agent) =>
        agent.backend.type === "local" &&
        agent.pubkey.toLowerCase() === assignment.agentPubkey.toLowerCase(),
    ) ?? null
  );
}
