import * as React from "react";

import { useGroupsQuery } from "@/features/groups/groupHooks";
import type { UserGroup } from "@/shared/api/relayGroups";
import type { AgentPersona, AgentTeam, ChannelType } from "@/shared/api/types";
import { trimMapToSize } from "@/shared/lib/trimMapToSize";
import { hasMention } from "./hasMention";
import {
  buildGroupMentionCandidates,
  buildTeamMentionCandidates,
  type MentionCandidate,
} from "./mentionCandidates";

type UseCollectionMentionsOptions = {
  baseCandidates: MentionCandidate[];
  channelType?: ChannelType | null;
  personas: AgentPersona[];
  teams: AgentTeam[];
};

export function useCollectionMentions({
  baseCandidates,
  channelType,
  personas,
  teams,
}: UseCollectionMentionsOptions) {
  const groupsQuery = useGroupsQuery();
  const selectedGroupsRef = React.useRef<Map<string, UserGroup>>(new Map());
  const [selectedGroupHandles, setSelectedGroupHandles] = React.useState<
    string[]
  >([]);

  const candidates = React.useMemo(
    () => [
      ...baseCandidates,
      ...buildTeamMentionCandidates(teams, personas, baseCandidates),
      ...(channelType === "forum"
        ? []
        : buildGroupMentionCandidates(groupsQuery.data ?? [])),
    ],
    [baseCandidates, channelType, groupsQuery.data, personas, teams],
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

      selectedGroupsRef.current.set(handle, group);
      trimMapToSize(selectedGroupsRef.current, 200);
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
    (text: string): UserGroup[] => {
      const groups: UserGroup[] = [];
      const seen = new Set<string>();
      for (const [handle, group] of selectedGroupsRef.current) {
        if (hasMention(text, handle)) {
          groups.push(group);
          seen.add(group.id);
        }
      }
      for (const group of groupsQuery.data ?? []) {
        if (!seen.has(group.id) && hasMention(text, group.handle)) {
          groups.push(group);
        }
      }
      return groups;
    },
    [groupsQuery.data],
  );

  const clear = React.useCallback(() => {
    selectedGroupsRef.current.clear();
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
