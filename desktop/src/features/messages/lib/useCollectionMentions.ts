import * as React from "react";

import { useGroupsQuery } from "@/features/groups/groupHooks";
import type { UserGroup } from "@/shared/api/relayGroups";
import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import { hasMention } from "./hasMention";
import {
  buildGroupMentionCandidates,
  buildTeamMentionCandidates,
  type MentionCandidate,
} from "./mentionCandidates";

type UseCollectionMentionsOptions = {
  baseCandidates: MentionCandidate[];
  includeGroups: boolean;
  personas: AgentPersona[];
  teams: AgentTeam[];
};

export function useCollectionMentions({
  baseCandidates,
  includeGroups,
  personas,
  teams,
}: UseCollectionMentionsOptions) {
  const groupsQuery = useGroupsQuery();
  const [selectedGroupHandles, setSelectedGroupHandles] = React.useState<
    string[]
  >([]);

  const candidates = React.useMemo(
    () => [
      ...baseCandidates,
      ...buildTeamMentionCandidates(teams, personas, baseCandidates),
      ...(includeGroups
        ? buildGroupMentionCandidates(groupsQuery.data ?? [])
        : []),
    ],
    [baseCandidates, groupsQuery.data, includeGroups, personas, teams],
  );

  const searchableNames = React.useMemo(() => {
    const names: string[] = [];
    const seen = new Set<string>();

    for (const candidate of candidates) {
      for (const name of [
        candidate.displayName,
        candidate.personaName,
        candidate.secondaryLabel,
      ]) {
        const trimmed = name?.trim();
        if (trimmed && !seen.has(trimmed.toLowerCase())) {
          names.push(trimmed);
          seen.add(trimmed.toLowerCase());
        }
      }
    }

    return names;
  }, [candidates]);

  const selectGroup = React.useCallback(
    (groupId: string, handle: string): boolean => {
      const group = (groupsQuery.data ?? []).find(
        (candidate) => candidate.id === groupId,
      );
      if (!group) return false;

      setSelectedGroupHandles((current) =>
        current.some(
          (candidate) => candidate.toLowerCase() === handle.toLowerCase(),
        )
          ? current
          : [...current, handle],
      );
      return true;
    },
    [groupsQuery.data],
  );

  const extractGroups = React.useCallback(
    (text: string, selectedPeople: Iterable<string>): UserGroup[] => {
      const groups: UserGroup[] = [];
      const selectedPersonHandles = new Set(
        [...selectedPeople].map((handle) => handle.trim().toLowerCase()),
      );
      for (const group of groupsQuery.data ?? []) {
        if (
          !selectedPersonHandles.has(group.handle.toLowerCase()) &&
          hasMention(text, group.handle)
        ) {
          groups.push(group);
        }
      }
      return groups;
    },
    [groupsQuery.data],
  );

  const clear = React.useCallback(() => {
    setSelectedGroupHandles([]);
  }, []);

  return {
    candidates,
    clear,
    extractGroups,
    searchableNames,
    selectGroup,
    selectedGroupHandles,
  };
}
