import * as React from "react";

import { pickProfileAgent } from "@/features/agents/lib/pickProfileAgent";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * An explicit public key always names exactly that identity, whether active,
 * stopped, archived, or absent from this device's managed inventory. Only a
 * persona target may choose an archive-aware representative. A persona link is
 * not an identity alias and never grants local management of a different key.
 */
export function resolveCanonicalManagedAgent(input: {
  directManagedAgent: ManagedAgent | undefined;
  isArchived: (pubkey: string) => boolean;
  personaInstances: readonly ManagedAgent[];
  pubkey: string | undefined;
}): ManagedAgent | undefined {
  const { directManagedAgent, isArchived, personaInstances, pubkey } = input;
  if (pubkey) return directManagedAgent;
  return pickProfileAgent(personaInstances, isArchived);
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
  managedAgents: readonly ManagedAgent[] | undefined;
  personaId: string | undefined;
  pubkey: string | undefined;
}) {
  const { managedAgents, personaId, pubkey } = input;
  const directManagedAgent = React.useMemo(() => {
    if (!pubkey) return undefined;
    const target = normalizePubkey(pubkey);
    return managedAgents?.find(
      (agent) => normalizePubkey(agent.pubkey) === target,
    );
  }, [managedAgents, pubkey]);
  // Explicit identity targets can use only their own local definition link.
  // Relay-only identities must not inherit a local persona's Start/Edit actions.
  const linkedPersonaId = pubkey ? directManagedAgent?.personaId : personaId;
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
        pubkey,
      }),
    [directManagedAgent, isArchived, personaInstances, pubkey],
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
