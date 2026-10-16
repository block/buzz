import { isManagedAgentLive } from "@/features/agents/lib/managedAgentControlActions";
import type { ManagedAgent, PresenceStatus } from "@/shared/api/types";

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
 *
 * Pass `getAvailability` where the surface has presence: a provider's
 * `deployed` receipt outlives a shut-down or reaped body and must not outrank
 * a sibling that is actually running.
 */
export function pickProfileAgent(
  agents: readonly ManagedAgent[],
  isArchived: (pubkey: string) => boolean,
  getAvailability?: (pubkey: string) => PresenceStatus | undefined,
) {
  const isActive = (agent: ManagedAgent) =>
    isManagedAgentLive(agent, getAvailability?.(agent.pubkey));
  return [...agents]
    .filter((agent) => !isArchived(agent.pubkey))
    .sort((left, right) => {
      const activeDiff = Number(isActive(right)) - Number(isActive(left));
      if (activeDiff !== 0) return activeDiff;
      return left.name.localeCompare(right.name);
    })[0];
}
