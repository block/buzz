import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import { compareMembersByRole } from "./memberUtils";

export function useClassifiedMembers(
  members: ChannelMember[],
  currentPubkey?: string,
) {
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const isArchived = useIsArchivedPredicate();

  const managedAgents = managedAgentsQuery.data ?? [];
  const relayAgents = relayAgentsQuery.data ?? [];

  const managedAgentPubkeys = React.useMemo(
    () => new Set(managedAgents.map((agent) => normalizePubkey(agent.pubkey))),
    [managedAgents],
  );
  const relayAgentPubkeys = React.useMemo(
    () => new Set(relayAgents.map((agent) => normalizePubkey(agent.pubkey))),
    [relayAgents],
  );

  const isBot = React.useCallback(
    (member: ChannelMember) => {
      const normalized = normalizePubkey(member.pubkey);
      return (
        member.role === "bot" ||
        managedAgentPubkeys.has(normalized) ||
        relayAgentPubkeys.has(normalized)
      );
    },
    [managedAgentPubkeys, relayAgentPubkeys],
  );

  const isMyBot = React.useCallback(
    (member: ChannelMember) => {
      return managedAgentPubkeys.has(normalizePubkey(member.pubkey));
    },
    [managedAgentPubkeys],
  );

  // Archived wins over bot: a zombie agent should fold into "Archived", not
  // appear as an active "Bot". This is NIP-IA's headline use case. Peel
  // archived FIRST, then split the remainder into people/bots.
  const { people, bots, archived } = React.useMemo(() => {
    const peopleList: ChannelMember[] = [];
    const botList: ChannelMember[] = [];
    const archivedList: ChannelMember[] = [];

    for (const member of members) {
      if (isArchived(member.pubkey)) {
        archivedList.push(member);
        continue;
      }
      if (isBot(member)) {
        botList.push(member);
      } else {
        peopleList.push(member);
      }
    }

    const sort = (list: ChannelMember[]) =>
      [...list].sort((left, right) =>
        compareMembersByRole(left, right, currentPubkey),
      );

    return {
      people: sort(peopleList),
      bots: sort(botList),
      archived: sort(archivedList),
    };
  }, [currentPubkey, isArchived, isBot, members]);

  return {
    people,
    bots,
    archived,
    peopleCount: people.length,
    botCount: bots.length,
    archivedCount: archived.length,
    isBot,
    isMyBot,
    managedAgentsQuery,
    relayAgentsQuery,
  };
}
