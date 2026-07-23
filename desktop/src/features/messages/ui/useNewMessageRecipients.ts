import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  coalesceAgentAutocompleteCandidates,
  getMentionableAgentPubkeys,
  getSharedChannelIds,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
  useUserSearchFetchMoreOnScroll,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { rankUserCandidatesBySearch } from "@/features/profile/lib/userCandidateSearch";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent, UserSearchResult } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

/** Maximum recipients (excluding the current user) a DM can address. */
export const NEW_MESSAGE_RECIPIENT_LIMIT = 8;

const DIRECTORY_PAGE_SIZE = 50;

export type NewMessageRecipientCandidate = UserSearchResult & {
  isManagedAgent?: boolean;
  isMember?: boolean;
  personaId?: string | null;
};

export function formatRecipientName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

function candidateWithAgentMetadata(
  candidate: UserSearchResult,
  managedAgentsByPubkey: ReadonlyMap<string, ManagedAgent>,
): NewMessageRecipientCandidate {
  const agent = managedAgentsByPubkey.get(normalizePubkey(candidate.pubkey));
  return {
    ...candidate,
    isManagedAgent: Boolean(agent),
    personaId: agent?.personaId,
  };
}

/**
 * Shared recipient-picker state for the new-message compose surface: search
 * query, selected recipients (chips), the ranked candidate directory, and the
 * paging/owner-profile helpers the row UI needs. Extracted from the former
 * NewDirectMessageDialog so the compose page (and any future surface) share a
 * single, tested selection model.
 */
export function useNewMessageRecipients({
  active,
  currentPubkey,
}: {
  /** When false, the directory/agent queries stay idle. */
  active: boolean;
  currentPubkey?: string;
}) {
  const [searchQuery, setSearchQuery] = React.useState("");
  const [directoryIdentityQuery, setDirectoryIdentityQuery] = React.useState<
    string | null
  >(null);
  const [selectedUsers, setSelectedUsers] = React.useState<UserSearchResult[]>(
    [],
  );
  const selectedUsersCountRef = React.useRef(0);
  const deferredSearchQuery = React.useDeferredValue(searchQuery.trim());
  const hasReachedRecipientLimit =
    selectedUsers.length >= NEW_MESSAGE_RECIPIENT_LIMIT;

  const selectedPubkeys = React.useMemo(
    () => new Set(selectedUsers.map((user) => normalizePubkey(user.pubkey))),
    [selectedUsers],
  );

  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: active });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: active });
  const channelsQuery = useChannelsQuery({ enabled: active });
  const userSearchQuery = useInfiniteUserSearchQuery(deferredSearchQuery, {
    allowEmpty: true,
    enabled:
      active && (!hasReachedRecipientLimit || deferredSearchQuery.length > 0),
    limit: DIRECTORY_PAGE_SIZE,
  });
  const userSearchResults = useFlattenedUserSearchResults(userSearchQuery.data);
  const isArchivedDiscovery = useIsArchivedPredicate();

  const searchResults = React.useMemo(() => {
    const candidatesByPubkey = new Map<string, NewMessageRecipientCandidate>();
    const managedAgentsByPubkey = new Map(
      (managedAgentsQuery.data ?? []).map((agent) => [
        normalizePubkey(agent.pubkey),
        agent,
      ]),
    );
    const currentPubkeyNormalized = currentPubkey
      ? normalizePubkey(currentPubkey)
      : null;
    const eligibleAgentPubkeys = getMentionableAgentPubkeys({
      currentPubkey,
      managedAgentPubkeys: (managedAgentsQuery.data ?? []).map(
        (agent) => agent.pubkey,
      ),
      relayAgents: relayAgentsQuery.data,
      sharedChannelIds: getSharedChannelIds(channelsQuery.data),
    });

    const addCandidate = (
      candidate: NewMessageRecipientCandidate,
      options: { includeSelected?: boolean } = {},
    ) => {
      const pubkey = normalizePubkey(candidate.pubkey);

      if (
        pubkey === currentPubkeyNormalized ||
        (!options.includeSelected && selectedPubkeys.has(pubkey)) ||
        isArchivedDiscovery(pubkey) ||
        (candidate.isAgent && !eligibleAgentPubkeys.has(pubkey))
      ) {
        return;
      }

      const current = candidatesByPubkey.get(pubkey);
      if (!current) {
        candidatesByPubkey.set(pubkey, { ...candidate, pubkey });
        return;
      }

      const candidateName = candidate.displayName?.trim() || null;
      const currentName = current.displayName?.trim() || null;

      candidatesByPubkey.set(pubkey, {
        pubkey,
        avatarUrl: current.avatarUrl ?? candidate.avatarUrl ?? null,
        displayName:
          candidate.isAgent && candidateName
            ? candidateName
            : current.isAgent
              ? currentName
              : (currentName ?? candidateName),
        nip05Handle: current.nip05Handle ?? candidate.nip05Handle ?? null,
        ownerPubkey: current.ownerPubkey ?? candidate.ownerPubkey ?? null,
        isAgent: current.isAgent || candidate.isAgent,
        isManagedAgent: current.isManagedAgent || candidate.isManagedAgent,
        isMember: current.isMember || candidate.isMember,
        personaId: current.personaId ?? candidate.personaId,
      });
    };

    for (const user of userSearchResults) {
      addCandidate(candidateWithAgentMetadata(user, managedAgentsByPubkey), {
        includeSelected: deferredSearchQuery.length > 0,
      });
    }

    for (const agent of relayAgentsQuery.data ?? []) {
      if (!eligibleAgentPubkeys.has(normalizePubkey(agent.pubkey))) {
        continue;
      }

      addCandidate(
        {
          pubkey: agent.pubkey,
          displayName: agent.name,
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
          isAgent: true,
        },
        { includeSelected: deferredSearchQuery.length > 0 },
      );
    }

    for (const agent of managedAgentsQuery.data ?? []) {
      addCandidate(
        {
          pubkey: agent.pubkey,
          displayName: agent.name,
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: currentPubkey ?? null,
          isAgent: true,
          isManagedAgent: true,
          personaId: agent.personaId,
        },
        { includeSelected: deferredSearchQuery.length > 0 },
      );
    }

    const coalescedCandidates = coalesceAgentAutocompleteCandidates(
      [...candidatesByPubkey.values()],
      {
        currentPubkey,
        getLabel: formatRecipientName,
      },
    );

    return rankUserCandidatesBySearch({
      allowEmptyQuery: true,
      candidates: coalescedCandidates,
      getLabel: formatRecipientName,
      limit: Math.max(DIRECTORY_PAGE_SIZE, coalescedCandidates.length),
      query: deferredSearchQuery,
    });
  }, [
    channelsQuery.data,
    currentPubkey,
    deferredSearchQuery,
    isArchivedDiscovery,
    managedAgentsQuery.data,
    relayAgentsQuery.data,
    selectedPubkeys,
    userSearchResults,
  ]);

  const isDirectoryLoading =
    userSearchQuery.isLoading ||
    managedAgentsQuery.isLoading ||
    relayAgentsQuery.isLoading ||
    channelsQuery.isLoading;
  React.useEffect(() => {
    if (isDirectoryLoading) {
      return;
    }

    setDirectoryIdentityQuery(deferredSearchQuery);
  }, [deferredSearchQuery, isDirectoryLoading]);
  const isDirectorySettling =
    isDirectoryLoading || directoryIdentityQuery !== deferredSearchQuery;
  const handleDirectoryScroll = useUserSearchFetchMoreOnScroll(
    userSearchQuery,
    !hasReachedRecipientLimit || deferredSearchQuery.length > 0,
  );

  const searchOwnerPubkeys = React.useMemo(
    () => [
      ...new Set(
        searchResults
          .map((user) => user.ownerPubkey)
          .filter((pubkey): pubkey is string =>
            Boolean(
              pubkey &&
                pubkey.toLowerCase() !==
                  identityQuery.data?.pubkey?.toLowerCase(),
            ),
          ),
      ),
    ],
    [identityQuery.data?.pubkey, searchResults],
  );
  const ownerProfilesQuery = useUsersBatchQuery(searchOwnerPubkeys, {
    enabled: active && searchOwnerPubkeys.length > 0,
  });

  // Clearing the query on each new chip mirrors the modal's behavior: the
  // search box empties so the next recipient can be typed immediately.
  React.useEffect(() => {
    if (selectedUsers.length > selectedUsersCountRef.current) {
      setSearchQuery("");
    }
    selectedUsersCountRef.current = selectedUsers.length;
  }, [selectedUsers.length]);

  const selectUser = React.useCallback(
    (user: UserSearchResult) => {
      if (selectedUsers.length >= NEW_MESSAGE_RECIPIENT_LIMIT) {
        return;
      }

      setSelectedUsers((current) => {
        const pubkey = normalizePubkey(user.pubkey);
        if (
          current.some(
            (candidate) => normalizePubkey(candidate.pubkey) === pubkey,
          )
        ) {
          return current;
        }

        return [...current, user];
      });
      setSearchQuery("");
    },
    [selectedUsers.length],
  );

  const removeUser = React.useCallback((pubkey: string) => {
    setSelectedUsers((current) =>
      current.filter((candidate) => candidate.pubkey !== pubkey),
    );
  }, []);

  const reset = React.useCallback(() => {
    setSearchQuery("");
    setSelectedUsers([]);
    selectedUsersCountRef.current = 0;
  }, []);

  return {
    currentPubkey: currentPubkey ?? identityQuery.data?.pubkey,
    deferredSearchQuery,
    handleDirectoryScroll,
    hasReachedRecipientLimit,
    isDirectoryLoading: isDirectorySettling,
    ownerProfiles: ownerProfilesQuery.data?.profiles,
    removeUser,
    reset,
    searchError:
      userSearchQuery.error instanceof Error ? userSearchQuery.error : null,
    searchQuery,
    searchResults,
    selectUser,
    selectedUsers,
    setSearchQuery,
  };
}
