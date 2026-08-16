import { agentDisplayGroupKey } from "@/features/agents/lib/agentIdentity";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Pick the instance that represents a persona throughout the UI.
 *
 * A persona can have several historical agent instances. Keeping this rule in
 * one place prevents an avatar click on an older message from opening a
 * different detail surface than the card in the Agents library.
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

/**
 * Pick the instance a profile request should actually land on.
 *
 * Collapsing onto the persona's representative instance is only correct while
 * the instances are interchangeable presentations of that persona. Once the
 * owner has renamed one, opening its card must open *it* — otherwise the
 * Agents library shows a card the profile panel refuses to open.
 *
 * Scoping happens before archive filtering rather than after: the display group
 * decides *which* siblings are candidates, and `pickProfileAgent` then drops the
 * archived ones. A group whose every instance is archived falls back to the
 * persona-wide list, so this never resolves to nothing where the unscoped
 * selector would have found a live instance.
 */
export function pickCanonicalProfileAgent(
  personaInstances: readonly ManagedAgent[],
  requested: ManagedAgent | undefined,
  isArchived: (pubkey: string) => boolean,
) {
  if (!requested) return pickProfileAgent(personaInstances, isArchived);

  const key = agentDisplayGroupKey(requested);
  const sameLabel = personaInstances.filter(
    (instance) => agentDisplayGroupKey(instance) === key,
  );

  return (
    pickProfileAgent(sameLabel, isArchived) ??
    pickProfileAgent(personaInstances, isArchived) ??
    requested
  );
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
 * panel matches the Agents library. The `isArchived` predicate keeps that
 * redirect from ever landing on an archived sibling.
 *
 * The redirect resolves through `pickCanonicalProfileAgent`, so it also stays
 * inside the clicked instance's display group: a retired instance the owner
 * renamed falls back to an active sibling carrying *that* name, never to a
 * differently named one. Redirecting across names would reopen the bug that
 * made a renamed agent unreachable from its own card.
 */
export function pickDirectProfileAgent(
  directAgent: ManagedAgent,
  personaInstances: readonly ManagedAgent[],
  isArchived: (pubkey: string) => boolean,
) {
  if (isManagedAgentActive(directAgent)) return directAgent;
  const canonical = pickCanonicalProfileAgent(
    personaInstances,
    directAgent,
    isArchived,
  );
  return canonical && isManagedAgentActive(canonical) ? canonical : directAgent;
}
