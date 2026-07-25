import * as React from "react";

import {
  type ActiveChannelTurnSummary,
  useActiveAgentTurnsBridge,
  useActiveAgentTurnsByChannel,
} from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import type {
  ManagedAgent,
  RelayAgent,
  UserProfileSummary,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type WorkingAgentName = Pick<ManagedAgent, "pubkey" | "name">;
type WorkingAgent = Pick<ManagedAgent, "pubkey" | "name" | "status">;
type PillDiagProfile = UserProfileSummary & {
  pubkey?: string;
};
type OwnedRelayWorkingAgent = Pick<RelayAgent, "pubkey" | "name"> & {
  status: "deployed";
};

export function resolveActiveWorkingChannelNames(
  summary: ActiveChannelTurnSummary,
  workingAgents: readonly WorkingAgentName[],
): ActiveChannelTurnSummary {
  const namesByPubkey = new Map(
    workingAgents.map((agent) => [normalizePubkey(agent.pubkey), agent.name]),
  );

  return {
    ...summary,
    agentNames: summary.agentPubkeys.flatMap((pubkey) => {
      const name = namesByPubkey.get(normalizePubkey(pubkey));
      return name ? [name] : [];
    }),
  };
}

export function getOwnedRelayWorkingAgents(
  relayAgents: readonly Pick<RelayAgent, "pubkey" | "name">[],
  profiles: Record<string, UserProfileSummary> | undefined,
  currentPubkey: string | undefined,
): OwnedRelayWorkingAgent[] {
  if (!currentPubkey) return [];

  return relayAgents.flatMap((agent) => {
    const profile = profiles?.[normalizePubkey(agent.pubkey)];
    if (!ownsAuthorAgent(profile, currentPubkey)) {
      return [];
    }

    return [{ pubkey: agent.pubkey, name: agent.name, status: "deployed" }];
  });
}

function agentCanStartObserver(agent: WorkingAgent) {
  return agent.status === "running" || agent.status === "deployed";
}

export function mergeWorkingAgents(
  managedAgents: readonly WorkingAgent[],
  ownedRelayAgents: readonly WorkingAgent[],
): WorkingAgent[] {
  const mergedByPubkey = new Map<string, WorkingAgent>();

  for (const agent of managedAgents) {
    mergedByPubkey.set(normalizePubkey(agent.pubkey), agent);
  }

  for (const agent of ownedRelayAgents) {
    const pubkey = normalizePubkey(agent.pubkey);
    const managedAgent = mergedByPubkey.get(pubkey);
    if (!managedAgent || !agentCanStartObserver(managedAgent)) {
      mergedByPubkey.set(pubkey, agent);
    }
  }

  return [...mergedByPubkey.values()];
}

function summarizeAgent(agent: WorkingAgentName & { status?: string }) {
  return {
    pubkey: normalizePubkey(agent.pubkey),
    name: agent.name,
    status: agent.status ?? null,
  };
}

function summarizeRelayProfile(
  pubkey: string,
  profile: PillDiagProfile | undefined,
  currentPubkey: string | undefined,
) {
  return {
    pubkey: normalizePubkey(pubkey),
    profileKeyPubkey: profile?.pubkey ? normalizePubkey(profile.pubkey) : null,
    displayName: profile?.displayName ?? null,
    isAgent: profile?.isAgent ?? null,
    ownerPubkey: profile?.ownerPubkey
      ? normalizePubkey(profile.ownerPubkey)
      : null,
    ownsAuthorAgent: ownsAuthorAgent(profile, currentPubkey),
  };
}

function logPillDiagnostics({
  currentPubkey,
  managedAgents,
  relayAgents,
  profiles,
  ownedRelayAgents,
  workingAgents,
  activeWorkingChannels,
}: {
  currentPubkey: string | undefined;
  managedAgents: readonly WorkingAgent[];
  relayAgents: readonly Pick<RelayAgent, "pubkey" | "name">[];
  profiles: Record<string, UserProfileSummary> | undefined;
  ownedRelayAgents: readonly WorkingAgent[];
  workingAgents: readonly WorkingAgent[];
  activeWorkingChannels: readonly ActiveChannelTurnSummary[];
}) {
  const normalizedProfiles = Object.fromEntries(
    relayAgents.map((agent) => {
      const pubkey = normalizePubkey(agent.pubkey);
      const profile = profiles?.[pubkey] as PillDiagProfile | undefined;
      return [
        pubkey,
        summarizeRelayProfile(agent.pubkey, profile, currentPubkey),
      ];
    }),
  );

  console.groupCollapsed("[pill-diag] active working channels inputs", {
    currentPubkey: currentPubkey ? normalizePubkey(currentPubkey) : null,
    managedAgentCount: managedAgents.length,
    relayAgentCount: relayAgents.length,
    ownedRelayAgentCount: ownedRelayAgents.length,
    workingAgentCount: workingAgents.length,
    activeChannelCount: activeWorkingChannels.length,
  });
  console.log("[pill-diag] currentPubkey", {
    currentPubkey: currentPubkey ? normalizePubkey(currentPubkey) : null,
  });
  console.table(managedAgents.map(summarizeAgent));
  console.log("[pill-diag] managedAgents", managedAgents.map(summarizeAgent));
  console.table(relayAgents.map(summarizeAgent));
  console.log("[pill-diag] relayAgents", relayAgents.map(summarizeAgent));
  console.log("[pill-diag] profilesByRelayAgent", normalizedProfiles);
  console.table(ownedRelayAgents.map(summarizeAgent));
  console.log(
    "[pill-diag] ownedRelayAgents",
    ownedRelayAgents.map(summarizeAgent),
  );
  console.table(workingAgents.map(summarizeAgent));
  console.log("[pill-diag] workingAgents", workingAgents.map(summarizeAgent));
  console.log("[pill-diag] activeWorkingChannels", activeWorkingChannels);
  console.groupEnd();
}

export function useActiveWorkingChannelsById(): ReadonlyMap<
  string,
  ActiveChannelTurnSummary
> {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = React.useMemo(
    () => managedAgentsQuery.data ?? [],
    [managedAgentsQuery.data],
  );
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgents = React.useMemo(
    () => relayAgentsQuery.data ?? [],
    [relayAgentsQuery.data],
  );
  const relayAgentPubkeys = React.useMemo(
    () => relayAgents.map((agent) => agent.pubkey),
    [relayAgents],
  );
  const relayAgentProfilesQuery = useUsersBatchQuery(relayAgentPubkeys, {
    enabled: relayAgentPubkeys.length > 0,
  });
  const ownedRelayAgents = React.useMemo(
    () =>
      getOwnedRelayWorkingAgents(
        relayAgents,
        relayAgentProfilesQuery.data?.profiles,
        currentPubkey,
      ),
    [currentPubkey, relayAgentProfilesQuery.data?.profiles, relayAgents],
  );
  const workingAgents = React.useMemo(
    () => mergeWorkingAgents(managedAgents, ownedRelayAgents),
    [managedAgents, ownedRelayAgents],
  );

  useManagedAgentObserverBridge(workingAgents);
  useActiveAgentTurnsBridge(workingAgents);

  const activeWorkingChannels = useActiveAgentTurnsByChannel();

  React.useEffect(() => {
    logPillDiagnostics({
      currentPubkey,
      managedAgents,
      relayAgents,
      profiles: relayAgentProfilesQuery.data?.profiles,
      ownedRelayAgents,
      workingAgents,
      activeWorkingChannels,
    });
  }, [
    activeWorkingChannels,
    currentPubkey,
    managedAgents,
    ownedRelayAgents,
    relayAgentProfilesQuery.data?.profiles,
    relayAgents,
    workingAgents,
  ]);

  return React.useMemo(
    () =>
      new Map(
        activeWorkingChannels.map((summary) => {
          const resolvedSummary = resolveActiveWorkingChannelNames(
            summary,
            workingAgents,
          );
          return [resolvedSummary.channelId, resolvedSummary];
        }),
      ),
    [activeWorkingChannels, workingAgents],
  );
}
