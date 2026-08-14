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
