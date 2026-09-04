import {
  isMentionActionable,
  markMentionCollisions,
} from "./mentionPresentation";
import * as React from "react";
import {
  useManagedAgentsQuery,
  usePersonasQuery,
  useRelayAgentsQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import {
  useChannelMembersQuery,
  useChannelsQuery,
} from "@/features/channels/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import {
  getAgentIdentityPubkeys,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
  isAgentDirectoryReady,
  isAgentMentionChannelType,
  rememberSelectedAgentPubkeys,
  uniqueAutocompleteLabels,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import {
  useInfiniteUserSearchQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { AutocompleteEdit } from "./useRichTextEditor";
import type { ChannelMember, ChannelType } from "@/shared/api/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useMentionQuery } from "./useMentionQuery";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { trimMapToSize } from "@/shared/lib/trimMapToSize";
import { useActiveAgentPubkeys } from "./useActiveAgentPubkeys";
import { useDefaultAgentSuggestion } from "./useDefaultAgentSuggestion";
import { useAgentMentionRevalidation } from "./agentMentionRevalidation";
import {
  extractMentionPubkeys,
  mentionMatchCandidates,
  selectedMentionLabel,
  selectedMentionLabels,
} from "./extractMentionPubkeys";
import {
  extractMentionPersonasFromMaps,
  type PersonaMentionTarget,
} from "./extractMentionPersonas";
import { useDraftMentionRouting } from "./useDraftMentionRouting";
import {
  type MentionPickerMode,
  useMentionSelection,
} from "./useMentionSelection";
import { rankMentionCandidates } from "./mentionRanking";
import { mapMentionCandidateToSuggestion } from "./mentionSuggestionMapping";
import { getMentionMemberPubkeys } from "./mentionMemberPubkeys";
import {
  appendUniqueName,
  buildTeamMentionCandidates,
  formatTeamMention,
  sameTeamMentionRecipients,
  type MentionCandidate,
} from "./mentionCandidates";
import { buildMentionCandidates } from "./buildMentionCandidates";
const MENTION_SUGGESTION_LIMIT = 50;
type UseMentionsOptions = {
  channelType?: ChannelType | null;
  recentMentionPubkeys?: readonly string[];
  /** Read document and selection from one live editor state at commit time. */
  getEditorSnapshot?: () => { text: string; cursor: number };
};
export function useMentions(
  channelId: string | null,
  externalMembers?: ChannelMember[],
  profiles?: UserProfileLookup,
  options?: UseMentionsOptions,
) {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : null;
  const admissionScope = React.useMemo(
    () => ({ currentPubkey, channelId }),
    [currentPubkey, channelId],
  );
  const query = useMentionQuery(options?.getEditorSnapshot, admissionScope);
  const mentionQuery = query.request?.query ?? null;
  const mentionStartIndex = query.request?.startIndex ?? 0;
  const { searchableNamesLowerRef, currentPrefix: currentMentionPrefix } =
    query;
  const [selectedMentionNames, setSelectedMentionNames] = React.useState<
    string[]
  >([]);
  const [selectedAgentMentionNames, setSelectedAgentMentionNames] =
    React.useState<string[]>([]);
  const selectedAgentMentionNamesRef = React.useRef<string[]>([]);
  const selectedAgentMentionPubkeysRef = React.useRef<Set<string>>(new Set());
  selectedAgentMentionNamesRef.current = selectedAgentMentionNames;
  const mentionMapRef = React.useRef<Map<string, string>>(new Map());
  const personaMentionMapRef = React.useRef<Map<string, string>>(new Map());
  const mentionSearchQuery = mentionQuery?.trim() ?? "";
  const canSearchGlobalPeople = mentionSearchQuery.length > 0;
  const membersQuery = useChannelMembersQuery(channelId);
  const members = externalMembers ?? membersQuery.data;
  const isArchivedDiscovery = useIsArchivedPredicate();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const managedAgentDirectoryReady = isAgentDirectoryReady(managedAgentsQuery);
  const relayAgentDirectoryReady = isAgentDirectoryReady(relayAgentsQuery);
  const agentDirectoriesReady =
    managedAgentDirectoryReady && relayAgentDirectoryReady;
  const canSearchGlobalUsers = canSearchGlobalPeople && agentDirectoriesReady;
  const userSearchQuery = useInfiniteUserSearchQuery(mentionQuery ?? "", {
    allowEmpty: true,
    enabled: canSearchGlobalUsers && mentionQuery !== null,
    limit: MENTION_SUGGESTION_LIMIT,
  });
  const userSearchResults = React.useMemo(
    () => userSearchQuery.data?.pages.flatMap((page) => page.users) ?? [],
    [userSearchQuery.data],
  );
  const managedAgentNamesByPubkey = React.useMemo(
    () =>
      new Map(
        (managedAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent.name,
        ]),
      ),
    [managedAgentsQuery.data],
  );
  const managedAgentPersonaIdsByPubkey = React.useMemo(
    () =>
      new Map(
        (managedAgentsQuery.data ?? [])
          .filter((agent) => Boolean(agent.personaId))
          .map((agent) => [
            normalizePubkey(agent.pubkey),
            agent.personaId as string,
          ]),
      ),
    [managedAgentsQuery.data],
  );
  const managedAgentPersonaIds = React.useMemo(
    () =>
      new Set(
        (managedAgentsQuery.data ?? [])
          .map((agent) => agent.personaId)
          .filter((personaId): personaId is string => Boolean(personaId)),
      ),
    [managedAgentsQuery.data],
  );
  const managedAgentPubkeys = React.useMemo(
    () =>
      new Set(
        (managedAgentsQuery.data ?? []).map((agent) =>
          normalizePubkey(agent.pubkey),
        ),
      ),
    [managedAgentsQuery.data],
  );
  const relayAgentNamesByPubkey = React.useMemo(
    () =>
      new Map(
        (relayAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent.name,
        ]),
      ),
    [relayAgentsQuery.data],
  );
  const activeAgentPubkeys = useActiveAgentPubkeys(
    managedAgentsQuery.data,
    relayAgentsQuery.data,
  );
  const sharedChannelIds = React.useMemo(
    () => getSharedChannelIds(channelsQuery.data),
    [channelsQuery.data],
  );
  const mentionChannelId = isAgentMentionChannelType(options?.channelType)
    ? channelId
    : null;
  const mentionableAgentPubkeys = React.useMemo(
    () =>
      getMentionableAgentPubkeys({
        currentPubkey,
        phase: "prepare",
        eligibilityScope: mentionChannelId
          ? { type: "channel", channelId: mentionChannelId }
          : options?.channelType === "dm"
            ? { type: "owned", channelId }
            : { type: "managed-only" },
        managedAgentPubkeys,
        relayAgents: relayAgentsQuery.data,
        sharedChannelIds,
      }),
    [
      currentPubkey,
      channelId,
      options?.channelType,
      managedAgentPubkeys,
      mentionChannelId,
      relayAgentsQuery.data,
      sharedChannelIds,
    ],
  );
  const personaNameByPubkey = React.useMemo(() => {
    const agents = managedAgentsQuery.data ?? [];
    const personas = personasQuery.data ?? [];
    const personaById = new Map(personas.map((p) => [p.id, p.displayName]));
    const lookup = new Map<string, string>();
    for (const agent of agents) {
      if (agent.personaId) {
        const name = personaById.get(agent.personaId);
        if (name) lookup.set(normalizePubkey(agent.pubkey), name);
      }
    }
    return lookup;
  }, [managedAgentsQuery.data, personasQuery.data]);
  const knownAgentPubkeys = React.useMemo(
    () => new Set([...mentionableAgentPubkeys, ...managedAgentPubkeys]),
    [managedAgentPubkeys, mentionableAgentPubkeys],
  );
  const activePersonas = React.useMemo(
    () => (personasQuery.data ?? []).filter((persona) => persona.isActive),
    [personasQuery.data],
  );
  const activePersonaById = React.useMemo(
    () => new Map(activePersonas.map((persona) => [persona.id, persona])),
    [activePersonas],
  );
  const activePersonaIds = React.useMemo(
    () => new Set(activePersonas.map((persona) => persona.id)),
    [activePersonas],
  );
  const memberPubkeys = React.useMemo(
    () => getMentionMemberPubkeys(channelId, channelsQuery.data, members),
    [channelId, channelsQuery.data, members],
  );
  const agentIdentityPubkeys = React.useMemo(
    () =>
      getAgentIdentityPubkeys({
        managedAgentPubkeys,
        relayAgents: relayAgentsQuery.data ?? [],
        members: members ?? [],
        profileIsAgent: (pubkey) => profiles?.[pubkey]?.isAgent === true,
      }),
    [managedAgentPubkeys, members, profiles, relayAgentsQuery.data],
  );
  const mentionCandidates = React.useMemo<MentionCandidate[]>(
    () =>
      buildMentionCandidates({
        activeAgentPubkeys,
        activePersonaById,
        activePersonas,
        canSearchGlobalUsers,
        currentPubkey,
        isArchived: isArchivedDiscovery,
        managedAgentDirectoryReady,
        managedAgentNamesByPubkey,
        managedAgentPersonaIds,
        managedAgentPersonaIdsByPubkey,
        managedAgents: managedAgentsQuery.data,
        memberPubkeys,
        members,
        mentionChannelId,
        mentionableAgentPubkeys,
        personaNameByPubkey,
        profiles,
        relayAgentDirectoryReady,
        relayAgentNamesByPubkey,
        relayAgents: relayAgentsQuery.data,
        userSearchResults,
      }),
    [
      activePersonaById,
      activeAgentPubkeys,
      activePersonas,
      userSearchResults,
      canSearchGlobalUsers,
      currentPubkey,
      isArchivedDiscovery,
      managedAgentDirectoryReady,
      managedAgentNamesByPubkey,
      managedAgentPersonaIds,
      managedAgentPersonaIdsByPubkey,
      managedAgentsQuery.data,
      memberPubkeys,
      members,
      mentionChannelId,
      mentionableAgentPubkeys,
      personaNameByPubkey,
      profiles,
      relayAgentDirectoryReady,
      relayAgentNamesByPubkey,
      relayAgentsQuery.data,
    ],
  );
  const mentionCandidatesWithTeams = React.useMemo(
    () =>
      markMentionCollisions([
        ...mentionCandidates,
        ...buildTeamMentionCandidates(
          teamsQuery.data ?? [],
          personasQuery.data ?? [],
          mentionCandidates,
        ),
      ]),
    [mentionCandidates, personasQuery.data, teamsQuery.data],
  );
  const ownerPubkeys = React.useMemo(
    () => [
      ...new Set(
        mentionCandidates
          .map((candidate) => candidate.ownerPubkey)
          .filter((pubkey): pubkey is string => Boolean(pubkey)),
      ),
    ],
    [mentionCandidates],
  );
  const ownerProfilesQuery = useUsersBatchQuery(ownerPubkeys, {
    enabled: ownerPubkeys.length > 0,
  });
  const searchableNames = React.useMemo(
    () => uniqueAutocompleteLabels(mentionCandidatesWithTeams),
    [mentionCandidatesWithTeams],
  );
  const highlightNames = React.useMemo<string[]>(() => {
    const names: string[] = [];
    const seen = new Set<string>();
    for (const name of selectedMentionNames) {
      const trimmed = name.trim();
      if (trimmed && !seen.has(trimmed.toLowerCase())) {
        names.push(trimmed);
        seen.add(trimmed.toLowerCase());
      }
    }
    return names;
  }, [selectedMentionNames]);
  const agentHighlightNames = React.useMemo<string[]>(() => {
    const names: string[] = [];
    const seen = new Set<string>();
    for (const name of selectedAgentMentionNames) {
      const trimmed = name.trim();
      if (trimmed && !seen.has(trimmed.toLowerCase())) {
        names.push(trimmed);
        seen.add(trimmed.toLowerCase());
      }
    }
    return names;
  }, [selectedAgentMentionNames]);
  const searchableNamesLower = React.useMemo<string[]>(
    () => searchableNames.map((n) => n.toLowerCase()),
    [searchableNames],
  );
  searchableNamesLowerRef.current = searchableNamesLower;
  const matchingSuggestions = React.useMemo<MentionSuggestion[]>(() => {
    if (mentionQuery === null) {
      return [];
    }
    return rankMentionCandidates(
      mentionCandidatesWithTeams,
      mentionQuery,
      activePersonaIds,
    )
      .slice(0, MENTION_SUGGESTION_LIMIT)
      .map(({ candidate, label }) =>
        mapMentionCandidateToSuggestion({
          agentProvenanceReady: agentDirectoriesReady,
          candidate,
          label,
          channelType: options?.channelType,
          currentPubkey,
          ownerProfiles: ownerProfilesQuery.data?.profiles,
          profiles,
        }),
      );
  }, [
    activePersonaIds,
    agentDirectoriesReady,
    currentPubkey,
    mentionCandidatesWithTeams,
    mentionQuery,
    options?.channelType,
    ownerProfilesQuery.data?.profiles,
    profiles,
  ]);
  const defaultAgentSuggestion = useDefaultAgentSuggestion({
    activePersonaIds,
    agentProvenanceReady: agentDirectoriesReady,
    candidates: mentionCandidates,
    channelType: options?.channelType,
    currentPubkey,
    ownerProfiles: ownerProfilesQuery.data?.profiles,
    profiles,
    recentMentionPubkeys: options?.recentMentionPubkeys,
  });
  const getDefaultAgentSuggestion = defaultAgentSuggestion;
  // Search hooks are keyed by the requested text. Wait for that request's
  // first page and initial directories, then keep exactly one displayed set.
  const resultsReady =
    (channelId === null ||
      !!externalMembers ||
      (!membersQuery.isPending && !membersQuery.isFetching)) &&
    !managedAgentsQuery.isPending &&
    !managedAgentsQuery.isFetching &&
    !relayAgentsQuery.isPending &&
    !relayAgentsQuery.isFetching &&
    !personasQuery.isPending &&
    !personasQuery.isFetching &&
    !teamsQuery.isPending &&
    !teamsQuery.isFetching &&
    (!canSearchGlobalUsers ||
      (!userSearchQuery.isPending && !userSearchQuery.isFetching));
  const mentionSelection = useMentionSelection(
    query.request,
    matchingSuggestions,
    resultsReady,
  );
  const {
    suggestions,
    mentionSelectedIndex,
    isLoading: isMentionLoading,
  } = mentionSelection;
  const isMentionOpen = mentionQuery !== null;
  // Recheck against this render's exact-key evidence even if a child retained
  // an older row/callback. A rejected selection must not establish draft intent.
  const admissionRef = React.useRef({
    scope: admissionScope,
    candidates: mentionCandidatesWithTeams,
  });
  admissionRef.current = {
    scope: admissionScope,
    candidates: mentionCandidatesWithTeams,
  };
  const canSelectMention = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const current = admissionRef.current.candidates.find((candidate) =>
        suggestion.pubkey
          ? candidate.pubkey === normalizePubkey(suggestion.pubkey)
          : suggestion.teamId
            ? candidate.teamId === suggestion.teamId
            : !!suggestion.personaId &&
              candidate.personaId === suggestion.personaId,
      );
      return (
        admissionRef.current.scope === admissionScope &&
        !!current &&
        (current.kind !== "team" ||
          (suggestion.kind === "team" &&
            !!suggestion.teamMembers?.length &&
            sameTeamMentionRecipients(
              suggestion.teamMembers,
              current.teamMembers,
            ) &&
            suggestion.teamMembers.every((member) => {
              const matches = (target: {
                pubkey?: string;
                personaId?: string | null;
              }) =>
                member.pubkey
                  ? target.pubkey === normalizePubkey(member.pubkey)
                  : !!member.personaId &&
                    !target.pubkey &&
                    target.personaId === member.personaId;
              return (
                current.teamMembers?.some(matches) &&
                admissionRef.current.candidates.some(
                  (target) => matches(target) && isMentionActionable(target),
                )
              );
            }))) &&
        isMentionActionable(current) &&
        isMentionActionable(suggestion)
      );
    },
    [admissionScope],
  );
  const insertMention = React.useCallback(
    (suggestion: MentionSuggestion, selectionEnd: number): AutocompleteEdit => {
      const prefix = currentMentionPrefix();
      if (
        !query.isCurrent() ||
        !suggestions.includes(suggestion) ||
        !canSelectMention(suggestion) ||
        selectionEnd !== query.read().cursor ||
        (prefix && prefix.startIndex > selectionEnd) ||
        (!prefix && !query.request?.explicit)
      )
        return {
          replaceFromOffset: selectionEnd,
          replaceToOffset: selectionEnd,
          insertText: "",
        };
      const [boundSuggestion] = selectedMentionLabels(
        [suggestion],
        mentionMapRef.current,
      );
      const displayName = boundSuggestion.displayName;
      const teamMembers =
        suggestion.kind === "team" && suggestion.teamMembers
          ? selectedMentionLabels(suggestion.teamMembers, mentionMapRef.current)
          : null;
      const insertText = teamMembers
        ? formatTeamMention(displayName, teamMembers)
        : `@${displayName} `;
      const mentions = mentionMapRef.current;
      const personaMentions = personaMentionMapRef.current;
      const selectedMentions = teamMembers ?? [boundSuggestion];
      for (const selected of selectedMentions) {
        if (selected.kind === "persona" && selected.personaId) {
          personaMentions.set(selected.displayName, selected.personaId);
          mentions.delete(selected.displayName);
        } else if (selected.pubkey) {
          mentions.set(selected.displayName, selected.pubkey);
          personaMentions.delete(selected.displayName);
        }
      }
      setSelectedMentionNames((current) => {
        const known = new Set(current.map((name) => name.toLowerCase()));
        return [
          ...current,
          ...selectedMentions
            .map((selected) => selected.displayName)
            .filter((name) => !known.has(name.toLowerCase())),
        ];
      });
      const isAgentMention =
        suggestion.kind === "persona" ||
        suggestion.kind === "team" ||
        suggestion.isAgent === true ||
        (suggestion.pubkey
          ? knownAgentPubkeys.has(normalizePubkey(suggestion.pubkey))
          : false);
      rememberSelectedAgentPubkeys(
        selectedAgentMentionPubkeysRef.current,
        selectedMentions,
        isAgentMention,
      );
      if (isAgentMention) {
        setSelectedAgentMentionNames((current) => {
          const known = new Set(current.map((name) => name.toLowerCase()));
          const next = [
            ...current,
            ...selectedMentions
              .map((selected) => selected.displayName)
              .filter((name) => !known.has(name.toLowerCase())),
          ];
          selectedAgentMentionNamesRef.current = next;
          return next;
        });
      }
      trimMapToSize(mentions, 200);
      trimMapToSize(personaMentions, 200);
      query.cancel();
      const startIndex = prefix?.startIndex ?? selectionEnd;
      return {
        replaceFromOffset: startIndex,
        replaceToOffset: selectionEnd,
        insertText,
      };
    },
    [
      canSelectMention,
      currentMentionPrefix,
      knownAgentPubkeys,
      query,
      suggestions,
    ],
  );
  const registerMentionPubkey = React.useCallback(
    (displayName: string, pubkey: string, options?: { isAgent?: boolean }) => {
      const trimmedName = selectedMentionLabel(
        displayName.trim(),
        pubkey,
        mentionMapRef.current,
      );
      if (!trimmedName) {
        return;
      }
      mentionMapRef.current.set(trimmedName, pubkey);
      personaMentionMapRef.current.delete(trimmedName);
      trimMapToSize(mentionMapRef.current, 200);
      setSelectedMentionNames((current) =>
        appendUniqueName(current, trimmedName),
      );
      if (options?.isAgent) {
        selectedAgentMentionPubkeysRef.current.add(normalizePubkey(pubkey));
        selectedAgentMentionNamesRef.current = appendUniqueName(
          selectedAgentMentionNamesRef.current,
          trimmedName,
        );
        setSelectedAgentMentionNames(selectedAgentMentionNamesRef.current);
      }
      return trimmedName;
    },
    [],
  );
  const insertResolvedMention = React.useCallback(
    ({
      displayName,
      pubkey,
      replaceFromOffset,
      replaceToOffset,
      isAgent = false,
    }: {
      displayName: string;
      pubkey: string;
      replaceFromOffset: number;
      replaceToOffset: number;
      isAgent?: boolean;
    }): AutocompleteEdit => {
      const label = registerMentionPubkey(displayName, pubkey, { isAgent });
      return {
        replaceFromOffset,
        replaceToOffset,
        insertText: `@${label ?? displayName.trim()} `,
      };
    },
    [registerMentionPubkey],
  );
  const getMentionDisplayName = React.useCallback(
    (pubkey: string): string | null => {
      const normalizedPubkey = normalizePubkey(pubkey);
      for (const [displayName, mentionPubkey] of mentionMapRef.current) {
        if (normalizePubkey(mentionPubkey) === normalizedPubkey) {
          return displayName;
        }
      }
      const candidate = mentionCandidates.find(
        (item) =>
          item.pubkey !== undefined &&
          normalizePubkey(item.pubkey) === normalizedPubkey,
      );
      return candidate?.displayName ?? null;
    },
    [mentionCandidates],
  );
  const isAgentPubkey = React.useCallback(
    (pubkey: string): boolean => knownAgentPubkeys.has(normalizePubkey(pubkey)),
    [knownAgentPubkeys],
  );
  const isManagedAgentPubkey = React.useCallback(
    (pubkey: string): boolean =>
      managedAgentPubkeys.has(normalizePubkey(pubkey)),
    [managedAgentPubkeys],
  );
  const isInlineMentionSelection = React.useCallback(
    () => !!query.request && !query.request.explicit,
    [query.request],
  );
  const updateMentionQuery = query.update;
  const openMentionPicker = React.useCallback(
    (cursorPosition: number, preference: MentionPickerMode = null) => {
      query.open(cursorPosition, preference === "first-agent");
    },
    [query.open],
  );
  const extractMentionPubkeysForCurrentMentions = React.useCallback(
    (text: string): string[] => {
      const extracted = extractMentionPubkeys({
        text,
        selectedMentions: mentionMapRef.current,
        selectedDisplayNames: personaMentionMapRef.current.keys(),
        memberCandidates: mentionCandidates,
      });
      // Selections are intent, not cached authorization. Never discard a
      // selected key because a refresh removed it from the picker.
      return extracted;
    },
    [mentionCandidates],
  );
  const getSelectedAgentPubkeys = React.useRef(
    () => selectedAgentMentionPubkeysRef.current,
  ).current;
  const revalidateMentionPubkeys = useAgentMentionRevalidation({
    agentPubkeys: agentIdentityPubkeys,
    getSelectedAgentPubkeys,
    currentPubkey,
    eligibilityScope: mentionChannelId
      ? { type: "channel", channelId: mentionChannelId }
      : options?.channelType === "dm"
        ? { type: "owned", channelId }
        : { type: "managed-only" },
    sharedChannelIds,
    refetchManagedAgents: managedAgentsQuery.refetch,
  });
  const extractMentionPersonas = React.useCallback(
    (text: string): PersonaMentionTarget[] =>
      extractMentionPersonasFromMaps(
        text,
        personaMentionMapRef.current,
        activePersonaById,
        mentionMatchCandidates({
          selectedMentions: mentionMapRef.current,
          selectedDisplayNames: personaMentionMapRef.current.keys(),
          memberCandidates: mentionCandidates,
        }).map((candidate) => candidate.displayName),
      ),
    [activePersonaById, mentionCandidates],
  );
  const cancelMentionAutocomplete = query.cancel;
  const clearMentions = React.useCallback(() => {
    cancelMentionAutocomplete();
    mentionMapRef.current.clear();
    personaMentionMapRef.current.clear();
    selectedAgentMentionNamesRef.current = [];
    selectedAgentMentionPubkeysRef.current.clear();
    setSelectedMentionNames([]);
    setSelectedAgentMentionNames([]);
  }, [cancelMentionAutocomplete]);
  const { getDraftMentionRefs, restoreDraftMentionRefs } =
    useDraftMentionRouting({
      memberCandidates: mentionCandidates,
      mentionMapRef,
      personaMentionMapRef,
      selectedAgentNamesRef: selectedAgentMentionNamesRef,
      selectedAgentPubkeysRef: selectedAgentMentionPubkeysRef,
      cancelAutocomplete: cancelMentionAutocomplete,
      setSelectedNames: setSelectedMentionNames,
      setSelectedAgentNames: setSelectedAgentMentionNames,
    });
  const handleMentionKeyDown = (
    event: React.KeyboardEvent,
    opts?: { isCodeContext?: () => boolean },
  ): { handled: boolean; suggestion?: MentionSuggestion } => {
    if (!isMentionOpen) return { handled: false };
    if (!query.isCurrent()) {
      query.cancel();
      return { handled: false };
    }
    if (event.key === "Escape") {
      event.preventDefault();
      query.cancel();
      return { handled: true };
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      mentionSelection.move(event.key === "ArrowDown" ? 1 : -1);
      return { handled: true };
    }
    const exactSpace =
      event.key === " " &&
      !event.shiftKey &&
      !event.altKey &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.nativeEvent.isComposing &&
      !opts?.isCodeContext?.();
    const selectKey =
      (event.key === "Tab" && !event.shiftKey) ||
      (event.key === "Enter" &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey &&
        !event.shiftKey);
    if (!exactSpace && !selectKey) return { handled: false };
    let chosen = suggestions[mentionSelectedIndex];
    if (exactSpace) {
      // Space remains literal for partial/multi-word/ambiguous names. Unlike
      // Tab, it is implicit completion, not a choice of the highlighted row.
      const name = mentionQuery?.trim().toLowerCase();
      const exact = suggestions.filter(
        (s) => s.displayName.trim().toLowerCase() === name,
      );
      if (
        !name ||
        exact.length !== 1 ||
        exact[0].hasNameCollision ||
        searchableNamesLower.some((s) => s.startsWith(`${name} `)) ||
        userSearchQuery.hasNextPage ||
        userSearchQuery.isFetching ||
        !userSearchQuery.isSuccess ||
        !canSelectMention(exact[0])
      )
        return { handled: false };
      chosen = exact[0];
    }
    event.preventDefault();
    return chosen && canSelectMention(chosen)
      ? { handled: true, suggestion: chosen }
      : { handled: true };
  };
  return {
    canSelectMention,
    cancelMentionAutocomplete,
    clearMentions,
    getDefaultAgentSuggestion,
    extractMentionPersonas,
    extractMentionPubkeys: extractMentionPubkeysForCurrentMentions,
    revalidateMentionPubkeys,
    getDraftMentionRefs,
    getMentionDisplayName,
    handleMentionKeyDown,
    hasResolvedMembers: members !== undefined,
    insertMention,
    insertResolvedMention,
    agentKnownNames: agentHighlightNames,
    isAgentPubkey,
    isManagedAgentPubkey,
    isInlineMentionSelection,
    isMentionOpen,
    isMentionLoading,
    knownNames: highlightNames,
    memberPubkeys,
    mentionSelectedIndex,
    mentionStartIndex,
    openMentionPicker,
    registerMentionPubkey,
    restoreDraftMentionRefs,
    suggestions,
    updateMentionQuery,
  };
}
export type UseMentionsResult = ReturnType<typeof useMentions>;
