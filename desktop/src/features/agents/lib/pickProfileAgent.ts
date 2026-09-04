import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Pick the instance that represents a persona throughout the UI.
 *
 * A persona can have several historical agent instances. Keeping this rule in
 * one place keeps persona navigation consistent. Explicit pubkey navigation
 * never uses this selector: older messages still name their exact author.
 *
 * Relay-archived instances are never eligible, so an archived record early in
 * file order can't hijack the persona target. Returns `undefined` when every
 * instance is archived — the card then renders in persona-only mode. The
 * `isArchived` predicate is fail-open (returns `false` while the relay archive
 * snapshot loads), so a cold start never briefly picks nothing.
 */
export function pickProfileAgent(
  agents: readonly ManagedAgent[],
  isArchived: (pubkey: string) => boolean,
) {
  return [...agents]
    .filter((agent) => !isArchived(agent.pubkey))
    .sort((left, right) => {
      const activeDiff =
        Number(isManagedAgentActive(right)) -
        Number(isManagedAgentActive(left));
      if (activeDiff !== 0) return activeDiff;
      return left.name.localeCompare(right.name);
    })[0];
}
