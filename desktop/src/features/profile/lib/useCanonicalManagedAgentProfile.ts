import * as React from "react";

import {
  pickDirectProfileAgent,
  pickProfileAgent,
} from "@/features/agents/lib/pickProfileAgent";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUserProfileQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { useOwnedManagedAgentPersonaId } from "@/features/profile/lib/useOwnedManagedAgentPersonaId";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Resolve the single managed instance a profile surface represents, honouring
 * the archive-aware target-provenance rules. Pure so the resolution matrix is
 * testable without mounting the panel; the hook supplies the live inputs.
 *
 * - `preserveRequestedInstance` + a direct match pins that exact record (an
 *   explicit Runtime → Instances selection).
 * - A deliberately requested archived pubkey stays EXACT — exactness beats
 *   canonicalization iff the requested pubkey is archived — so its archive
 *   controller can unarchive that identity even when a live sibling exists.
 *   Returns the managed record when one exists; otherwise `undefined`, so the
 *   panel falls back to the requested pubkey verbatim (a historical archived
 *   key with no current managed record still resolves to itself).
 * - `preferDirectManagedAgent` (the panel's own default) keeps a directly
 *   opened active instance exact so an access edit targets it, only redirecting
 *   an inactive click to a live sibling — see `pickDirectProfileAgent`.
 * - Otherwise persona-target and non-archived historical navigation resolve
 *   through the shared archive-aware selector: all instances archived yields
 *   `undefined` (persona-only mode), else the canonical live instance.
 */
export function resolveCanonicalManagedAgent(input: {
  directManagedAgent: ManagedAgent | undefined;
  isArchived: (pubkey: string) => boolean;
  personaInstances: readonly ManagedAgent[];
  preferDirectManagedAgent: boolean;
  preserveRequestedInstance: boolean;
  pubkey: string | undefined;
}): ManagedAgent | undefined {
  const {
    directManagedAgent,
    isArchived,
    personaInstances,
    preferDirectManagedAgent,
    preserveRequestedInstance,
    pubkey,
  } = input;
  if (preserveRequestedInstance && directManagedAgent) {
    return directManagedAgent;
  }
  if (pubkey && isArchived(pubkey)) {
    return directManagedAgent;
  }
  if (preferDirectManagedAgent && directManagedAgent) {
    return pickDirectProfileAgent(
      directManagedAgent,
      personaInstances,
      isArchived,
    );
  }
  return pickProfileAgent(personaInstances, isArchived) ?? directManagedAgent;
}

/**
 * Split a persona's instances into live and archived buckets off the same
 * archive predicate the selector uses — one policy, no duplication. Fail-open
 * is inherited: while the archive snapshot loads `isArchived` returns `false`,
 * so every instance lands in `live` and nothing is labeled or hidden.
 */
export function bucketPersonaInstances(
  personaInstances: readonly ManagedAgent[],
  isArchived: (pubkey: string) => boolean,
): { live: ManagedAgent[]; archived: ManagedAgent[] } {
  const live: ManagedAgent[] = [];
  const archived: ManagedAgent[] = [];
  for (const instance of personaInstances) {
    (isArchived(instance.pubkey) ? archived : live).push(instance);
  }
  return { live, archived };
}

export function useCanonicalManagedAgentProfile(input: {
  currentPubkey: string | undefined;
  managedAgents: readonly ManagedAgent[] | undefined;
  personaId: string | undefined;
  preferDirectManagedAgent?: boolean;
  preserveRequestedInstance?: boolean;
  pubkey: string | undefined;
}) {
  const {
    currentPubkey,
    managedAgents,
    personaId,
    preferDirectManagedAgent = false,
    preserveRequestedInstance = false,
    pubkey,
  } = input;
  const directManagedAgent = React.useMemo(() => {
    if (!pubkey) return undefined;
    const target = normalizePubkey(pubkey);
    return managedAgents?.find(
      (agent) => normalizePubkey(agent.pubkey) === target,
    );
  }, [managedAgents, pubkey]);
  const requestedProfileQuery = useUserProfileQuery(pubkey);
  const historicalPersonaId = useOwnedManagedAgentPersonaId({
    agentPubkey: pubkey,
    enabled: Boolean(
      pubkey &&
        !directManagedAgent &&
        ownsAuthorAgent(requestedProfileQuery.data, currentPubkey),
    ),
    ownerPubkey: currentPubkey,
  });
  const linkedPersonaId =
    personaId ?? directManagedAgent?.personaId ?? historicalPersonaId;
  const personaInstances = React.useMemo(() => {
    if (!linkedPersonaId) {
      return directManagedAgent ? [directManagedAgent] : [];
    }
    return (managedAgents ?? []).filter(
      (agent) => agent.personaId === linkedPersonaId,
    );
  }, [directManagedAgent, linkedPersonaId, managedAgents]);
  const isArchived = useIsArchivedPredicate();
  const managedAgent = React.useMemo(
    () =>
      resolveCanonicalManagedAgent({
        directManagedAgent,
        isArchived,
        personaInstances,
        preferDirectManagedAgent,
        preserveRequestedInstance,
        pubkey,
      }),
    [
      directManagedAgent,
      isArchived,
      personaInstances,
      preferDirectManagedAgent,
      preserveRequestedInstance,
      pubkey,
    ],
  );
  // Split the roster for the Instances list off the same predicate the selector
  // uses — see `bucketPersonaInstances` for the fail-open semantics.
  const instanceBuckets = React.useMemo(
    () => bucketPersonaInstances(personaInstances, isArchived),
    [isArchived, personaInstances],
  );

  return {
    instanceBuckets,
    linkedPersonaId,
    managedAgent,
  };
}
