import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Bot, UserRoundPlus, X } from "lucide-react";
import {
  invalidateChannelState,
  useAddChannelMembersMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import {
  coalesceAgentAutocompleteCandidates,
  isAgentIdentityInManagedList,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useClassifiedMembers } from "@/features/channels/lib/useClassifiedMembers";
import { formatMemberName } from "@/features/channels/lib/memberUtils";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
  useUserSearchFetchMoreOnScroll,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { formatOwnerLabel } from "@/features/profile/lib/identity";
import { rankUserCandidatesBySearch } from "@/features/profile/lib/userCandidateSearch";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { changeChannelMemberRole } from "@/shared/api/tauri";
import type {
  AddChannelMembersResult,
  Channel,
  ChannelMember,
  ManagedAgent,
  UserSearchResult,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { MembersSidebarMemberCard } from "./MembersSidebarMemberCard";
import { useManagedAgentRuntimesQuery } from "@/features/agents/managedAgentRuntimeHooks";
import {
  findManagedAgentRuntime,
  managedAgentPairAction,
} from "@/features/agents/managedAgentRuntimeStatus";
import { EditRespondToDialog } from "./EditRespondToDialog";
import { useMembersSidebarActions } from "./useMembersSidebarActions";
import { useMembersSidebarModeration } from "./useMembersSidebarModeration";
const MEMBER_ADD_RESULT_LIMIT = 50;
const MEMBER_SEARCH_MIN_QUERY_LENGTH = 2;
const MEMBER_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

function formatAddCandidateName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}
type AddMemberSearchCandidate = UserSearchResult & {
  isManagedAgent?: boolean;
  isMember?: boolean;
  personaId?: string | null;
};
function addMemberCandidatePersonaId(
  candidate: UserSearchResult,
  managedAgentsByPubkey: ReadonlyMap<string, ManagedAgent>,
) {
  return managedAgentsByPubkey.get(normalizePubkey(candidate.pubkey))
    ?.personaId;
}
function addMemberCandidateIsManagedAgent(
  candidate: UserSearchResult,
  managedAgentsByPubkey: ReadonlyMap<string, ManagedAgent>,
) {
  return managedAgentsByPubkey.has(normalizePubkey(candidate.pubkey));
}
function addMemberCandidateWithAgentMetadata(
  candidate: UserSearchResult,
  managedAgentsByPubkey: ReadonlyMap<string, ManagedAgent>,
): AddMemberSearchCandidate {
  return {
    ...candidate,
    isManagedAgent: addMemberCandidateIsManagedAgent(
      candidate,
      managedAgentsByPubkey,
    ),
    personaId: addMemberCandidatePersonaId(candidate, managedAgentsByPubkey),
  };
}

function memberModalRoleRank(member: ChannelMember) {
  if (member.role === "owner") return 0;
  if (member.role === "admin") return 1;
  return 2;
}
function compareMembersForModal(
  currentPubkey: string | undefined,
  left: ChannelMember,
  right: ChannelMember,
) {
  const rankDelta = memberModalRoleRank(left) - memberModalRoleRank(right);
  if (rankDelta !== 0) {
    return rankDelta;
  }

  if (currentPubkey && left.pubkey === currentPubkey) return -1;
  if (currentPubkey && right.pubkey === currentPubkey) return 1;

  return formatMemberName(left).localeCompare(formatMemberName(right));
}

type MembersSidebarProps = {
  channel: Channel | null;
  currentPubkey?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onViewActivity?: (pubkey: string) => void;
  relayUrl?: string;
};

export function MembersSidebar({
  channel,
  currentPubkey,
  open,
  onOpenChange,
  onViewActivity,
  relayUrl,
}: MembersSidebarProps) {
  const channelId = channel?.id ?? null;
  const managedAgentRuntimesQuery = useManagedAgentRuntimesQuery({
    enabled: open,
  });
  const queryClient = useQueryClient();
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const [searchQuery, setSearchQuery] = React.useState("");
  const [inviteSubmissionErrors, setInviteSubmissionErrors] = React.useState<
    AddChannelMembersResult["errors"]
  >([]);
  const [addingMemberPubkeys, setAddingMemberPubkeys] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const identityQuery = useIdentityQuery();
  const membersQuery = useChannelMembersQuery(channelId, open);
  const addMembersMutation = useAddChannelMembersMutation(channelId);
  const changeRoleMutation = useMutation({
    mutationFn: async ({ pubkey, role }: { pubkey: string; role: string }) => {
      if (!channelId) throw new Error("No channel selected.");
      await changeChannelMemberRole(channelId, pubkey, role);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["channels", channelId],
      });
    },
  });
  const changeRoleError =
    changeRoleMutation.error instanceof Error
      ? changeRoleMutation.error.message
      : null;

  const rawMembers = membersQuery.data ?? [];
  const selfMember =
    rawMembers.find((member) => member.pubkey === currentPubkey) ?? null;
  const {
    people,
    bots,
    archived,
    isBot,
    isMyBot,
    managedAgentsQuery,
    relayAgentsQuery,
  } = useClassifiedMembers(rawMembers, currentPubkey);
  const activeMembers = React.useMemo(
    () =>
      [...people, ...bots].sort((left, right) =>
        compareMembersForModal(currentPubkey, left, right),
      ),
    [bots, currentPubkey, people],
  );

  const allMemberPubkeys = React.useMemo(
    () => rawMembers.map((member) => member.pubkey),
    [rawMembers],
  );
  const memberPresenceQuery = usePresenceQuery(allMemberPubkeys, {
    enabled: open && rawMembers.length > 0,
  });
  const memberProfilesQuery = useUsersBatchQuery(allMemberPubkeys, {
    enabled: open && rawMembers.length > 0,
  });
  const normalizedSearchQuery = searchQuery.trim().toLowerCase();
  const deferredSearchQuery = React.useDeferredValue(searchQuery.trim());
  const normalizedDeferredSearchQuery = deferredSearchQuery.toLowerCase();
  const filteredActiveMembers = React.useMemo(() => {
    if (!normalizedSearchQuery) {
      return activeMembers;
    }

    const profiles = memberProfilesQuery.data?.profiles ?? {};

    return activeMembers.filter((member) => {
      const normalizedPubkey = normalizePubkey(member.pubkey);
      const profile = profiles[normalizedPubkey] ?? null;
      const memberIsBot = isBot(member);
      const labels = [
        formatMemberName(member, currentPubkey),
        member.displayName ?? "",
        profile?.displayName ?? "",
        memberIsBot ? "agent" : "",
        member.role,
        normalizedPubkey,
      ];

      return labels.some((label) =>
        label.toLowerCase().includes(normalizedSearchQuery),
      );
    });
  }, [
    activeMembers,
    currentPubkey,
    isBot,
    memberProfilesQuery.data?.profiles,
    normalizedSearchQuery,
  ]);
  const memberPubkeys = React.useMemo(
    () => new Set(rawMembers.map((member) => normalizePubkey(member.pubkey))),
    [rawMembers],
  );
  const canAddMembers =
    (selfMember !== null || channel?.visibility === "open") &&
    channel?.channelType !== "dm";
  const userSearchQuery = useInfiniteUserSearchQuery(deferredSearchQuery, {
    allowEmpty: false,
    enabled:
      open &&
      canAddMembers &&
      deferredSearchQuery.length >= MEMBER_SEARCH_MIN_QUERY_LENGTH,
    limit: MEMBER_ADD_RESULT_LIMIT,
  });
  const userSearchResults = useFlattenedUserSearchResults(userSearchQuery.data);
  const isArchivedDiscovery = useIsArchivedPredicate();
  const addSearchResults = React.useMemo(() => {
    if (!canAddMembers || normalizedDeferredSearchQuery.length === 0) {
      return [];
    }

    const candidatesByPubkey = new Map<string, AddMemberSearchCandidate>();
    const managedAgentsByPubkey = new Map(
      (managedAgentsQuery.data ?? []).map((agent) => [
        normalizePubkey(agent.pubkey),
        agent,
      ]),
    );
    const memberAgentLabels = new Set(
      rawMembers
        .filter((member) => member.isAgent === true || member.role === "bot")
        .map((member) => member.displayName?.trim().toLowerCase())
        .filter((label): label is string => Boolean(label)),
    );
    const managedAgentPubkeys = new Set(managedAgentsByPubkey.keys());

    const addCandidate = (candidate: AddMemberSearchCandidate) => {
      const pubkey = normalizePubkey(candidate.pubkey);
      if (
        (candidate.isAgent &&
          memberAgentLabels.has(
            formatAddCandidateName(candidate).toLowerCase(),
          )) ||
        memberPubkeys.has(pubkey) ||
        isArchivedDiscovery(pubkey) ||
        !isAgentIdentityInManagedList(candidate, managedAgentPubkeys)
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
      addCandidate(
        addMemberCandidateWithAgentMetadata(user, managedAgentsByPubkey),
      );
    }

    for (const agent of relayAgentsQuery.data ?? []) {
      addCandidate({
        pubkey: agent.pubkey,
        displayName: agent.name,
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: null,
        isAgent: true,
      });
    }

    for (const agent of managedAgentsQuery.data ?? []) {
      addCandidate({
        pubkey: agent.pubkey,
        displayName: agent.name,
        avatarUrl: null,
        nip05Handle: null,
        ownerPubkey: currentPubkey ?? null,
        isAgent: true,
        isManagedAgent: true,
        personaId: agent.personaId,
      });
    }

    const coalescedCandidates = coalesceAgentAutocompleteCandidates(
      [...candidatesByPubkey.values()],
      {
        currentPubkey,
        getLabel: formatAddCandidateName,
        preferredPubkeys: memberPubkeys,
      },
    );

    return rankUserCandidatesBySearch({
      candidates: coalescedCandidates,
      getLabel: formatAddCandidateName,
      limit: Math.max(MEMBER_ADD_RESULT_LIMIT, coalescedCandidates.length),
      query: normalizedDeferredSearchQuery,
    });
  }, [
    canAddMembers,
    isArchivedDiscovery,
    currentPubkey,
    managedAgentsQuery.data,
    memberPubkeys,
    normalizedDeferredSearchQuery,
    relayAgentsQuery.data,
    userSearchResults,
    rawMembers,
  ]);
  const isAddSearchLoading =
    userSearchQuery.isLoading ||
    managedAgentsQuery.isLoading ||
    relayAgentsQuery.isLoading;
  const handlePeopleSearchScroll = useUserSearchFetchMoreOnScroll(
    userSearchQuery,
    canAddMembers && normalizedDeferredSearchQuery.length > 0,
  );
  const addSearchOwnerPubkeys = React.useMemo(
    () => [
      ...new Set(
        addSearchResults
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
    [addSearchResults, identityQuery.data?.pubkey],
  );
  const addSearchOwnerProfilesQuery = useUsersBatchQuery(
    addSearchOwnerPubkeys,
    {
      enabled: open && addSearchOwnerPubkeys.length > 0,
    },
  );

  const filteredArchivedMembers = React.useMemo(() => {
    if (!normalizedSearchQuery) {
      return archived;
    }

    const profiles = memberProfilesQuery.data?.profiles ?? {};

    return archived.filter((member) => {
      const normalizedPubkey = normalizePubkey(member.pubkey);
      const profile = profiles[normalizedPubkey] ?? null;
      const memberIsBot = isBot(member);
      const labels = [
        formatMemberName(member, currentPubkey),
        member.displayName ?? "",
        profile?.displayName ?? "",
        memberIsBot ? "agent" : "",
        member.role,
        normalizedPubkey,
      ];

      return labels.some((label) =>
        label.toLowerCase().includes(normalizedSearchQuery),
      );
    });
  }, [
    archived,
    currentPubkey,
    isBot,
    memberProfilesQuery.data?.profiles,
    normalizedSearchQuery,
  ]);

  const canManageMembers =
    selfMember?.role === "owner" || selfMember?.role === "admin";

  const {
    canModerate,
    isModerationPending,
    moderationStateByPubkey,
    onBan,
    onUnban,
    onTimeout,
    onUntimeout,
  } = useMembersSidebarModeration(open);

  const isArchived =
    channel?.archivedAt !== null && channel?.archivedAt !== undefined;
  const managedAgentByPubkey = React.useMemo(
    () =>
      new Map(
        (managedAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent,
        ]),
      ),
    [managedAgentsQuery.data],
  );
  const controllableManagedBots = React.useMemo(
    () =>
      bots.flatMap((member) => {
        const agent = managedAgentByPubkey.get(normalizePubkey(member.pubkey));
        return agent ? [agent] : [];
      }),
    [bots, managedAgentByPubkey],
  );
  const canRemoveMember = React.useCallback(
    (member: ChannelMember) => {
      return (
        (selfMember?.role === "admin" && member.pubkey !== currentPubkey) ||
        (selfMember?.role === "owner" && member.role !== "owner") ||
        Boolean(selfMember && isMyBot(member)) ||
        member.pubkey === currentPubkey
      );
    },
    [currentPubkey, isMyBot, selfMember],
  );
  const removableManagedBots = React.useMemo(
    () =>
      bots.flatMap((member) => {
        if (!canRemoveMember(member)) {
          return [];
        }

        const agent = managedAgentByPubkey.get(normalizePubkey(member.pubkey));
        return agent ? [agent] : [];
      }),
    [bots, canRemoveMember, managedAgentByPubkey],
  );
  const {
    actionErrorMessage,
    actionNoticeMessage,
    handleLifecycleAction: handleAgentLifecycleAction,
    handleRemoveMember,
    isActionPending,
  } = useMembersSidebarActions({
    channelId,
    controllableManagedBots,
    removableManagedBots,
    currentPubkey,
    onOpenChange,
    relayUrl,
  });

  useFeedbackToasts(actionNoticeMessage, actionErrorMessage);

  const { openProfilePanel } = useProfilePanel();
  const handleOpenProfile = React.useMemo(
    () =>
      openProfilePanel
        ? (pubkey: string) => {
            onOpenChange(false);
            openProfilePanel(pubkey);
          }
        : undefined,
    [onOpenChange, openProfilePanel],
  );

  const [editRespondToAgent, setEditRespondToAgent] =
    React.useState<ManagedAgent | null>(null);

  React.useEffect(() => {
    if (!open) {
      setSearchQuery("");
      setInviteSubmissionErrors([]);
      setAddingMemberPubkeys(new Set());
      return;
    }

    searchInputRef.current?.focus({ preventScroll: true });
  }, [open]);

  if (!channel) {
    return null;
  }

  async function handleAddSearchResult(user: UserSearchResult) {
    if (addingMemberPubkeys.has(user.pubkey)) {
      return;
    }

    setAddingMemberPubkeys((prev) => new Set(prev).add(user.pubkey));

    try {
      // A local managed agent needs a running harness pair in this community,
      // not just channel membership — a bare membership add leaves it deaf
      // until someone @mentions it or manually starts the pair. Route it
      // through the attach helper, which adds membership AND ensures the active
      // community's pair is running (idempotent: a live pair is left alone).
      // Humans and provider agents keep the plain membership add.
      const managedAgent = managedAgentByPubkey.get(
        normalizePubkey(user.pubkey),
      );
      if (channelId && managedAgent?.backend.type === "local") {
        try {
          await attachManagedAgentToChannel(channelId, {
            agent: managedAgent,
            ensureRunning: true,
          });
          await invalidateChannelState(queryClient, channelId);
        } catch (error) {
          setInviteSubmissionErrors((prev) => [
            ...prev,
            {
              pubkey: user.pubkey,
              error:
                error instanceof Error ? error.message : "Failed to add agent.",
            },
          ]);
        }
        return;
      }

      const result = await addMembersMutation.mutateAsync({
        pubkeys: [user.pubkey],
        role: user.isAgent ? "bot" : "member",
      });
      setInviteSubmissionErrors((prev) => [...prev, ...result.errors]);
    } finally {
      setAddingMemberPubkeys((prev) => {
        const next = new Set(prev);
        next.delete(user.pubkey);
        return next;
      });
    }
  }

  function renderMemberCard(member: ChannelMember, memberIsBot: boolean) {
    const memberProfile =
      memberProfilesQuery.data?.profiles[member.pubkey.toLowerCase()];
    const viewerIsOwner = Boolean(
      memberProfile?.ownerPubkey &&
        currentPubkey &&
        memberProfile.ownerPubkey.toLowerCase() === currentPubkey.toLowerCase(),
    );
    const managedAgent = memberIsBot
      ? managedAgentByPubkey.get(normalizePubkey(member.pubkey))
      : undefined;
    const managedAgentRuntime =
      memberIsBot && relayUrl
        ? findManagedAgentRuntime(
            managedAgentRuntimesQuery.data ?? [],
            member.pubkey,
            relayUrl,
          )
        : undefined;
    // Mirrors the dispatch condition in useMembersSidebarActions: local
    // agents in a community context act on the pair; provider agents keep
    // the agent-wide deploy/!shutdown action.
    const pairAction =
      managedAgent?.backend.type === "local" && relayUrl
        ? managedAgentPairAction(managedAgentRuntime)
        : undefined;
    return (
      <div className="content-visibility-auto" key={member.pubkey}>
        <MembersSidebarMemberCard
          canChangeRole={canManageMembers && member.pubkey !== currentPubkey}
          canModerate={canModerate && member.pubkey !== currentPubkey}
          canRemoveMember={canRemoveMember(member)}
          isActionPending={
            isActionPending ||
            changeRoleMutation.isPending ||
            isModerationPending
          }
          isArchived={isArchived}
          managedAgent={managedAgent}
          managedAgentRuntime={managedAgentRuntime}
          member={member}
          memberIsBot={memberIsBot}
          memberAvatarLabel={
            member.displayName ?? truncatePubkey(member.pubkey)
          }
          memberLabel={formatMemberName(member, currentPubkey)}
          moderationState={moderationStateByPubkey.get(
            normalizePubkey(member.pubkey),
          )}
          onBan={onBan}
          onChangeRole={(m, role) => {
            void changeRoleMutation.mutateAsync({ pubkey: m.pubkey, role });
          }}
          onEditRespondTo={memberIsBot ? setEditRespondToAgent : undefined}
          onManagedAgentAction={(agent) => {
            void handleAgentLifecycleAction(agent, managedAgentRuntime);
          }}
          onOpenProfile={handleOpenProfile}
          onRemoveMember={handleRemoveMember}
          onTimeout={onTimeout}
          onUnban={onUnban}
          onUntimeout={onUntimeout}
          onViewActivity={
            onViewActivity
              ? (pubkey: string) => {
                  onOpenChange(false);
                  onViewActivity(pubkey);
                }
              : undefined
          }
          pairAction={pairAction}
          presenceStatus={
            memberPresenceQuery.data?.[member.pubkey.toLowerCase()] ?? null
          }
          profileAvatarUrl={memberProfile?.avatarUrl ?? null}
          viewerIsOwner={viewerIsOwner}
        />
      </div>
    );
  }

  return (
    <>
      <Dialog onOpenChange={onOpenChange} open={open}>
        <DialogContent
          aria-describedby={undefined}
          className="max-w-xl gap-0 overflow-hidden border-0 px-6 pb-0 pt-6"
          data-testid="members-sidebar"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            searchInputRef.current?.focus({ preventScroll: true });
          }}
          showCloseButton={false}
        >
          <DialogHeader className="space-y-0 pb-5">
            <div className="flex items-center justify-between gap-4">
              <DialogTitle>Channel members</DialogTitle>
              <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
                <X className="h-4 w-4" />
                <span className="sr-only">Close</span>
              </DialogClose>
            </div>
            <label
              className={MODAL_SEARCH_SHELL_CLASS}
              htmlFor="channel-management-search-users"
            >
              <UserRoundPlus className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
              <input
                autoCapitalize="none"
                autoCorrect="off"
                className={MODAL_SEARCH_INPUT_CLASS}
                data-testid="channel-management-search-users"
                disabled={isArchived}
                id="channel-management-search-users"
                onChange={(event) => {
                  setSearchQuery(event.target.value);
                }}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || addSearchResults.length === 0) {
                    return;
                  }

                  event.preventDefault();
                  void handleAddSearchResult(addSearchResults[0]);
                }}
                placeholder={
                  canAddMembers
                    ? "Add people and agents"
                    : "Search people and agents"
                }
                ref={searchInputRef}
                spellCheck={false}
                type="text"
                value={searchQuery}
              />
            </label>
          </DialogHeader>

          <div className="max-h-[calc(100vh-12rem)] overflow-y-auto pb-6">
            <section>
              <div
                className="h-[min(50vh,24rem)] overflow-y-auto rounded-xl border border-border/70 bg-background/70"
                data-testid="members-sidebar-people"
                onScroll={handlePeopleSearchScroll}
              >
                <SearchResultSectionTitle>
                  {normalizedSearchQuery
                    ? "Members"
                    : `Members · ${activeMembers.length}`}
                </SearchResultSectionTitle>
                {normalizedSearchQuery ? (
                  <div>
                    {filteredActiveMembers.map((member) =>
                      renderMemberCard(member, isBot(member)),
                    )}
                    {canAddMembers ? (
                      <>
                        {addSearchResults.length > 0 || isAddSearchLoading ? (
                          <SearchResultSectionTitle>
                            Not in this channel
                          </SearchResultSectionTitle>
                        ) : null}
                        {addSearchResults.map((user) => (
                          <AddMemberSearchResultRow
                            disabled={
                              addingMemberPubkeys.has(user.pubkey) || isArchived
                            }
                            key={user.pubkey}
                            onSelect={(selectedUser) => {
                              void handleAddSearchResult(selectedUser);
                            }}
                            ownerLabel={formatOwnerLabel(
                              user.ownerPubkey,
                              identityQuery.data?.pubkey,
                              addSearchOwnerProfilesQuery.data?.profiles,
                            )}
                            user={user}
                          />
                        ))}
                        {isAddSearchLoading ? (
                          <p className="px-4 py-3 text-sm text-muted-foreground">
                            Searching...
                          </p>
                        ) : null}
                      </>
                    ) : null}
                    {filteredActiveMembers.length === 0 &&
                    addSearchResults.length === 0 &&
                    !isAddSearchLoading ? (
                      <p className="px-4 py-3 text-sm text-muted-foreground">
                        No matching people or agents.
                      </p>
                    ) : null}
                  </div>
                ) : filteredActiveMembers.length > 0 ? (
                  <div>
                    {filteredActiveMembers.map((member) =>
                      renderMemberCard(member, isBot(member)),
                    )}
                  </div>
                ) : (
                  <p className="px-4 py-3 text-sm text-muted-foreground">
                    {membersQuery.isLoading
                      ? "Loading members..."
                      : normalizedSearchQuery
                        ? "No members match your search."
                        : "No members found."}
                  </p>
                )}
              </div>
            </section>

            {archived.length > 0 ? (
              <section className="mt-4">
                <details
                  className="group/archived"
                  data-testid="members-sidebar-archived"
                >
                  <summary className="flex cursor-pointer items-center gap-2 list-none [&::-webkit-details-marker]:hidden">
                    <h2 className="text-sm font-semibold tracking-tight text-muted-foreground">
                      Archived
                    </h2>
                    <span
                      className="text-muted-foreground"
                      data-testid="members-sidebar-archived-count"
                    >
                      ({archived.length})
                    </span>
                    <span className="ml-auto text-xs text-muted-foreground transition-transform group-open/archived:rotate-90">
                      ▸
                    </span>
                  </summary>
                  <div
                    className="mt-2 space-y-2"
                    data-testid="members-sidebar-archived-list"
                  >
                    {filteredArchivedMembers.map((member) =>
                      renderMemberCard(member, isBot(member)),
                    )}
                    {filteredArchivedMembers.length === 0 ? (
                      <p className="text-sm text-muted-foreground">
                        No archived members match your search.
                      </p>
                    ) : null}
                  </div>
                </details>
              </section>
            ) : null}

            {changeRoleError ? (
              <p
                className="mt-4 text-sm text-destructive"
                data-testid="members-sidebar-action-error"
              >
                {changeRoleError}
              </p>
            ) : null}

            {addMembersMutation.error instanceof Error ? (
              <p className="mt-4 text-sm text-destructive">
                {addMembersMutation.error.message}
              </p>
            ) : null}

            {inviteSubmissionErrors.length > 0 ? (
              <div className="mt-4 space-y-1 text-sm text-destructive">
                {inviteSubmissionErrors.map((error) => (
                  <p key={`${error.pubkey}-${error.error}`}>
                    {truncatePubkey(error.pubkey)}: {error.error}
                  </p>
                ))}
              </div>
            ) : null}
          </div>
        </DialogContent>
      </Dialog>
      <EditRespondToDialog
        agent={editRespondToAgent}
        currentPubkey={currentPubkey}
        onOpenChange={(dialogOpen) => {
          if (!dialogOpen) setEditRespondToAgent(null);
        }}
        open={editRespondToAgent !== null}
      />
    </>
  );
}

function SearchResultSectionTitle({
  action,
  children,
}: {
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="sticky top-0 z-10 mr-3 flex min-h-9 items-center gap-2 bg-background/95 px-4 pb-1.5 pt-3 text-xs font-medium text-muted-foreground/75 backdrop-blur supports-[backdrop-filter]:bg-background/80">
      <span>{children}</span>
      {action ? <span>{action}</span> : null}
    </div>
  );
}

function AddMemberSearchResultRow({
  disabled,
  onSelect,
  ownerLabel,
  user,
}: {
  disabled: boolean;
  onSelect: (user: UserSearchResult) => void;
  ownerLabel?: string | null;
  user: UserSearchResult;
}) {
  return (
    <div
      className={cn(
        "group/add-result relative isolate flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
        MEMBER_ROW_INSET_DIVIDER_CLASS,
      )}
      data-testid={`channel-user-search-result-${user.pubkey}`}
    >
      <button
        aria-label={`Select ${formatAddCandidateName(user)}`}
        className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        disabled={disabled}
        onClick={() => onSelect(user)}
        type="button"
      />
      <UserAvatar
        avatarUrl={user.avatarUrl}
        className="pointer-events-none relative z-10 h-8 w-8 text-xs shadow-none"
        displayName={formatAddCandidateName(user)}
        size="sm"
      />
      <div className="pointer-events-none relative z-10 min-w-0 flex-1">
        {user.isAgent ? (
          <div className="min-w-0">
            <div className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium tracking-tight">
                {formatAddCandidateName(user)}
              </span>
              <span className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                <Bot aria-hidden="true" className="h-4 w-4" />
                agent
              </span>
            </div>
            {ownerLabel ? (
              <span className="block truncate text-xs text-muted-foreground">
                managed by {ownerLabel}
              </span>
            ) : null}
          </div>
        ) : (
          <span className="block truncate text-sm font-medium tracking-tight">
            {formatAddCandidateName(user)}
          </span>
        )}
      </div>
      <Button
        className="relative z-20 shrink-0"
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          onSelect(user);
        }}
        size="sm"
        type="button"
      >
        Add
      </Button>
    </div>
  );
}
