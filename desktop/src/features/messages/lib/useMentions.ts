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
  filterAdmittedMentionPubkeys,
  filterCachedAgentSuggestions,
  getAdmittedAgentPubkeys,
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
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { trimMapToSize } from "@/shared/lib/trimMapToSize";
import { useActiveAgentPubkeys } from "./useActiveAgentPubkeys";
import { useDefaultAgentSuggestion } from "./useDefaultAgentSuggestion";
import { flushMentionDebounce, isPlainSpace } from "./flushMentionDebounce";
import { useAgentMentionRevalidation } from "./agentMentionRevalidation";
import { extractMentionPubkeys } from "./extractMentionPubkeys";
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
  type MentionCandidate,
} from "./mentionCandidates";
import { buildMentionCandidates } from "./buildMentionCandidates";
const MENTION_DEBOUNCE_MS = 120,
  MENTION_SUGGESTION_LIMIT = 50;
type UseMentionsOptions = {
  channelType?: ChannelType | null;
  recentMentionPubkeys?: readonly string[];
};
export function useMentions(
  channelId: string | null,
  externalMembers?: ChannelMember[],
  profiles?: UserProfileLookup,
  options?: UseMentionsOptions,
) {
  const [mentionQuery, setMentionQuery] = React.useState<string | null>(null);
  const [mentionStartIndex, setMentionStartIndex] = React.useState(0);
  const mentionPickerOriginRef = React.useRef<"inline" | "explicit" | null>(
    null,
  );
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
  const previousSuggestionsRef = React.useRef<MentionSuggestion[]>([]);
  const mentionSearchQuery = mentionQuery?.trim() ?? "";
  const canSearchGlobalPeople = mentionSearchQuery.length > 0;
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : null;
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
        eligibilityScope: mentionChannelId
          ? { type: "channel", channelId: mentionChannelId }
          : { type: "managed-only" },
        managedAgentPubkeys,
        relayAgents: relayAgentsQuery.data,
        sharedChannelIds,
      }),
    [
      currentPubkey,
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
  const admittedAgentPubkeys = React.useMemo(
    () => getAdmittedAgentPubkeys(mentionCandidates),
    [mentionCandidates],
  );
  const mentionCandidatesWithTeams = React.useMemo(
    () => [
      ...mentionCandidates,
      ...buildTeamMentionCandidates(
        teamsQuery.data ?? [],
        personasQuery.data ?? [],
        mentionCandidates,
      ),
    ],
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
  const debounceTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const latestValueRef = React.useRef<string>("");
  const latestCursorRef = React.useRef<number>(0);
  const flushedMentionStartIndexRef = React.useRef<number | null>(null);
  const searchableNamesLowerRef = React.useRef<string[]>(searchableNamesLower);
  searchableNamesLowerRef.current = searchableNamesLower;
  React.useEffect(
    () => () => {
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }
    },
    [],
  );
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
  const getDefaultAgentSuggestion = useDefaultAgentSuggestion({
    activePersonaIds,
    agentProvenanceReady: agentDirectoriesReady,
    candidates: mentionCandidates,
    channelType: options?.channelType,
    currentPubkey,
    ownerProfiles: ownerProfilesQuery.data?.profiles,
    profiles,
    recentMentionPubkeys: options?.recentMentionPubkeys,
  });
  const fetchMoreSuggestions = React.useCallback(() => {
    if (userSearchQuery.hasNextPage && !userSearchQuery.isFetchingNextPage) {
      void userSearchQuery.fetchNextPage();
    }
  }, [userSearchQuery]);
  const suggestions = React.useMemo<MentionSuggestion[]>(() => {
    if (mentionQuery === null) {
      return [];
    }
    if (matchingSuggestions.length > 0) {
      return matchingSuggestions;
    }
    if (userSearchQuery.isFetching) {
      return filterCachedAgentSuggestions(
        previousSuggestionsRef.current,
        mentionCandidatesWithTeams,
      );
    }
    return [];
  }, [
    matchingSuggestions,
    mentionCandidatesWithTeams,
    mentionQuery,
    userSearchQuery.isFetching,
  ]);
  React.useEffect(() => {
    if (mentionQuery === null) {
      previousSuggestionsRef.current = [];
      return;
    }
    if (matchingSuggestions.length > 0) {
      previousSuggestionsRef.current = matchingSuggestions;
    } else if (!userSearchQuery.isFetching) {
      previousSuggestionsRef.current = [];
    }
  }, [matchingSuggestions, mentionQuery, userSearchQuery.isFetching]);
  const mentionSelection = useMentionSelection(suggestions);
  const { mentionSelectedIndex, setMentionSelectedIndex: setSelected } =
    mentionSelection;
  const isMentionOpen = mentionQuery !== null && suggestions.length > 0;
  const insertMention = React.useCallback(
    (suggestion: MentionSuggestion, selectionEnd: number): AutocompleteEdit => {
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
      const displayName = suggestion.displayName;
      const teamMembers =
        suggestion.kind === "team" ? suggestion.teamMembers : null;
      const insertText = teamMembers
        ? formatTeamMention(displayName, teamMembers)
        : `@${displayName} `;
      const mentions = mentionMapRef.current;
      const personaMentions = personaMentionMapRef.current;
      const selectedMentions = teamMembers ?? [suggestion];
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
      mentionPickerOriginRef.current = null;
      setMentionQuery(null);
      setSelected(0);
      const startIndex =
        flushedMentionStartIndexRef.current ?? mentionStartIndex;
      flushedMentionStartIndexRef.current = null;
      return {
        replaceFromOffset: startIndex,
        replaceToOffset: selectionEnd,
        insertText,
      };
    },
    [knownAgentPubkeys, mentionStartIndex, setSelected],
  );
  const registerMentionPubkey = React.useCallback(
    (displayName: string, pubkey: string, options?: { isAgent?: boolean }) => {
      const trimmedName = displayName.trim();
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
        selectedAgentMentionNamesRef.current = appendUniqueName(
          selectedAgentMentionNamesRef.current,
          trimmedName,
        );
        setSelectedAgentMentionNames(selectedAgentMentionNamesRef.current);
      }
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
      registerMentionPubkey(displayName, pubkey, { isAgent });
      return {
        replaceFromOffset,
        replaceToOffset,
        insertText: `@${displayName.trim()} `,
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
    () => mentionPickerOriginRef.current === "inline",
    [],
  );
  const autocompleteGenerationRef = React.useRef(0);
  const updateMentionQuery = React.useCallback(
    (value: string, cursorPosition: number) => {
      mentionSelection.clearAgentSelectionPreference();
      const generation = ++autocompleteGenerationRef.current;
      latestValueRef.current = value;
      latestCursorRef.current = cursorPosition;
      const activeInlineMention = detectPrefixQuery(
        "@",
        value,
        cursorPosition,
        searchableNamesLowerRef.current,
      );
      if (activeInlineMention) {
        mentionPickerOriginRef.current = "inline";
      } else if (mentionPickerOriginRef.current === "inline") {
        mentionPickerOriginRef.current = null;
      }
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }
      debounceTimerRef.current = setTimeout(() => {
        debounceTimerRef.current = null;
        if (generation !== autocompleteGenerationRef.current) return;
        const mention = detectPrefixQuery(
          "@",
          latestValueRef.current,
          latestCursorRef.current,
          searchableNamesLowerRef.current,
        );
        if (mention) {
          mentionPickerOriginRef.current = "inline";
          setMentionQuery(mention.query);
          setMentionStartIndex(mention.startIndex);
          setSelected(0);
        } else {
          setMentionQuery(null);
        }
      }, MENTION_DEBOUNCE_MS);
    },
    [mentionSelection.clearAgentSelectionPreference, setSelected],
  );
  const openMentionPicker = React.useCallback(
    (cursorPosition: number, preference: MentionPickerMode = null) => {
      autocompleteGenerationRef.current += 1;
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
      flushedMentionStartIndexRef.current = null;
      mentionPickerOriginRef.current = "explicit";
      if (preference === "preserve") {
        setMentionStartIndex(cursorPosition);
        return;
      }
      mentionSelection.prepareSelectionPreference(preference);
      setMentionQuery("");
      setMentionStartIndex(cursorPosition);
      setSelected(0);
    },
    [mentionSelection.prepareSelectionPreference, setSelected],
  );
  const extractMentionPubkeysForCurrentMentions = React.useCallback(
    (text: string): string[] => {
      const extracted = extractMentionPubkeys({
        text,
        selectedMentions: mentionMapRef.current,
        selectedDisplayNames: personaMentionMapRef.current.keys(),
        memberCandidates: mentionCandidates,
      });
      return filterAdmittedMentionPubkeys(
        extracted,
        new Set([
          ...agentIdentityPubkeys,
          ...selectedAgentMentionPubkeysRef.current,
        ]),
        admittedAgentPubkeys,
      );
    },
    [admittedAgentPubkeys, agentIdentityPubkeys, mentionCandidates],
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
      ),
    [activePersonaById],
  );
  const cancelMentionAutocomplete = React.useCallback(() => {
    autocompleteGenerationRef.current += 1;
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    flushedMentionStartIndexRef.current = null;
    mentionPickerOriginRef.current = null;
    mentionSelection.clearAgentSelectionPreference();
    setMentionQuery(null);
    setSelected(0);
  }, [mentionSelection.clearAgentSelectionPreference, setSelected]);
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
      mentionMapRef,
      personaMentionMapRef,
      selectedAgentNamesRef: selectedAgentMentionNamesRef,
      cancelAutocomplete: cancelMentionAutocomplete,
      setSelectedNames: setSelectedMentionNames,
      setSelectedAgentNames: setSelectedAgentMentionNames,
    });
  const handleMentionKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent,
      // `isCodeContext` is only consulted for Space: inside code the typed
      // text must stay literal, so Space is left to the editor.
      opts?: { isCodeContext?: () => boolean },
    ): { handled: boolean; suggestion?: MentionSuggestion } => {
      const exactMentionSpace =
        isPlainSpace(event.nativeEvent) && !opts?.isCodeContext?.();
      if (!isMentionOpen && !exactMentionSpace) return { handled: false };
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelected((current) =>
          current < suggestions.length - 1 ? current + 1 : 0,
        );
        return { handled: true };
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelected((current) =>
          current > 0 ? current - 1 : suggestions.length - 1,
        );
        return { handled: true };
      }
      if (
        exactMentionSpace ||
        event.key === "Tab" ||
        (event.key === "Enter" &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey &&
          !event.shiftKey)
      ) {
        if (debounceTimerRef.current !== null || exactMentionSpace) {
          const flushed = flushMentionDebounce({
            debounceTimerRef,
            latestValueRef,
            latestCursorRef,
            searchableNamesLowerRef,
            candidates: mentionCandidatesWithTeams,
            activePersonaIds,
            agentProvenanceReady: agentDirectoriesReady,
            channelType: options?.channelType,
            currentPubkey,
            ownerProfiles: ownerProfilesQuery.data?.profiles,
            profiles,
            requireExact: exactMentionSpace,
          });
          if (exactMentionSpace && flushed?.type !== "match")
            return { handled: false };
          event.preventDefault();
          if (flushed?.type === "match") {
            flushedMentionStartIndexRef.current = flushed.startIndex;
            mentionPickerOriginRef.current = "inline";
            setMentionQuery(null); // reset so dropdown closes
            return { handled: true, suggestion: flushed.suggestion };
          }
          if (flushed?.type === "no-match") {
            setMentionQuery(null);
            return { handled: true };
          }
        }
        event.preventDefault();
        return { handled: true, suggestion: suggestions[mentionSelectedIndex] };
      }
      if (event.key === "Escape") {
        event.preventDefault();
        cancelMentionAutocomplete(); // full cancel incl. pending debounce
        return { handled: true };
      }
      return { handled: false };
    },
    [
      activePersonaIds,
      agentDirectoriesReady,
      cancelMentionAutocomplete,
      currentPubkey,
      isMentionOpen,
      mentionCandidatesWithTeams,
      mentionSelectedIndex,
      options?.channelType,
      ownerProfilesQuery.data?.profiles,
      profiles,
      setSelected,
      suggestions,
    ],
  );
  return {
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
    knownNames: highlightNames,
    memberPubkeys,
    mentionSelectedIndex,
    mentionStartIndex,
    openMentionPicker,
    registerMentionPubkey,
    restoreDraftMentionRefs,
    suggestions,
    fetchMoreSuggestions,
    hasMoreSuggestions: Boolean(userSearchQuery.hasNextPage),
    isFetchingMoreSuggestions: userSearchQuery.isFetchingNextPage,
    updateMentionQuery,
  };
}
export type UseMentionsResult = ReturnType<typeof useMentions>;
