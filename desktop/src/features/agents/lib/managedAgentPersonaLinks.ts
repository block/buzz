import type { ManagedAgentReference } from "@/shared/api/agentReferences";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function buildManagedAgentPersonaLinks(
  agents: readonly Pick<ManagedAgent, "pubkey" | "personaId">[],
  references: readonly ManagedAgentReference[],
) {
  const byPubkey = new Map(
    references.map((reference) => [
      normalizePubkey(reference.pubkey),
      reference.personaId,
    ]),
  );
  const personaIds = new Set(
    references.map((reference) => reference.personaId),
  );
  for (const agent of agents) {
    if (!agent.personaId) continue;
    byPubkey.set(normalizePubkey(agent.pubkey), agent.personaId);
    personaIds.add(agent.personaId);
  }
  return { byPubkey, personaIds };
}

export function findRemotePersonaAgent(
  personaId: string,
  localPubkeys: ReadonlySet<string>,
  references: readonly ManagedAgentReference[],
) {
  const normalizedLocalPubkeys = new Set(
    [...localPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );
  return references.find(
    (reference) =>
      reference.personaId === personaId &&
      !normalizedLocalPubkeys.has(normalizePubkey(reference.pubkey)),
  );
}
