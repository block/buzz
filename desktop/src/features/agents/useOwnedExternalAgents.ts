import * as React from "react";

import { usePresenceQuery } from "@/features/presence/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type {
  ManagedAgent,
  RelayAgent,
  UserProfileSummary,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type OwnedExternalAgent = {
  avatarUrl: string | null;
  name: string;
  pubkey: string;
};

type SelectOwnedExternalAgentsInput = {
  currentPubkey: string | null | undefined;
  managedAgents: readonly Pick<ManagedAgent, "pubkey">[];
  relayAgents: readonly Pick<RelayAgent, "name" | "pubkey">[];
  profiles: Readonly<
    Record<
      string,
      Pick<UserProfileSummary, "avatarUrl" | "displayName" | "ownerPubkey">
    >
  >;
};

export function selectOwnedExternalAgents({
  currentPubkey,
  managedAgents,
  relayAgents,
  profiles,
}: SelectOwnedExternalAgentsInput): OwnedExternalAgent[] {
  if (!currentPubkey) return [];

  const ownerPubkey = normalizePubkey(currentPubkey);
  const managedPubkeys = new Set(
    managedAgents.map((agent) => normalizePubkey(agent.pubkey)),
  );
  const seen = new Set<string>();
  const externalAgents: OwnedExternalAgent[] = [];

  for (const relayAgent of relayAgents) {
    const pubkey = normalizePubkey(relayAgent.pubkey);
    if (managedPubkeys.has(pubkey) || seen.has(pubkey)) continue;

    const profile = profiles[pubkey];
    if (
      !profile?.ownerPubkey ||
      normalizePubkey(profile.ownerPubkey) !== ownerPubkey
    ) {
      continue;
    }

    seen.add(pubkey);
    externalAgents.push({
      avatarUrl: profile.avatarUrl,
      name: profile.displayName?.trim() || relayAgent.name.trim() || pubkey,
      pubkey,
    });
  }

  return externalAgents.sort((left, right) =>
    left.name.localeCompare(right.name),
  );
}

export function useOwnedExternalAgents(
  managedAgents: readonly ManagedAgent[],
  relayAgents: readonly RelayAgent[],
) {
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const relayAgentPubkeys = React.useMemo(
    () => relayAgents.map((agent) => normalizePubkey(agent.pubkey)),
    [relayAgents],
  );
  const profilesQuery = useUsersBatchQuery(relayAgentPubkeys, {
    enabled: Boolean(currentPubkey) && relayAgentPubkeys.length > 0,
  });
  const agents = React.useMemo(
    () =>
      selectOwnedExternalAgents({
        currentPubkey,
        managedAgents,
        relayAgents,
        profiles: profilesQuery.data?.profiles ?? {},
      }),
    [currentPubkey, managedAgents, profilesQuery.data?.profiles, relayAgents],
  );
  const presenceQuery = usePresenceQuery(
    React.useMemo(() => agents.map((agent) => agent.pubkey), [agents]),
  );

  return { agents, presenceQuery, profilesQuery };
}
