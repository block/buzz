import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Pick the instance that represents a persona throughout the UI.
 *
 * A persona can have several historical agent instances. Keeping this rule in
 * one place prevents an avatar click on an older message from opening a
 * different detail surface than the card in the Agents library.
 */
export function pickProfileAgent(agents: readonly ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const activeDiff =
      Number(isManagedAgentActive(right)) - Number(isManagedAgentActive(left));
    if (activeDiff !== 0) return activeDiff;
    return left.name.localeCompare(right.name);
  })[0];
}

/**
 * Resolve which instance a profile panel opened for `directAgent` should
 * show, given every instance of the same persona.
 *
 * Access edits must target the exact instance the user clicked — resolving a
 * running sidebar member to an alphabetically-earlier sibling would let a
 * "tighten access" save widen the wrong agent. But when the clicked instance
 * is inactive and the persona has an active instance elsewhere (an avatar on
 * an old message from a retired instance), redirect to the active one so the
 * panel matches the Agents library.
 */
export function pickDirectProfileAgent(
  directAgent: ManagedAgent,
  personaInstances: readonly ManagedAgent[],
) {
  if (isManagedAgentActive(directAgent)) return directAgent;
  const canonical = pickProfileAgent(personaInstances);
  return canonical && isManagedAgentActive(canonical) ? canonical : directAgent;
}
