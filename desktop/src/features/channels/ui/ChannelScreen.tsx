import * as React from "react";
import { useAppShell } from "@/app/AppShellContext";
import { cacheSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useActiveChannelHeader } from "@/features/channels/useActiveChannelHeader";
import { useChannelPaneHandlers } from "@/features/channels/useChannelPaneHandlers";
import { useMessageEventProfilePubkeys } from "@/features/channels/useMessageEventProfilePubkeys";
import { useMessageOwnerProfiles } from "@/features/channels/useMessageOwnerProfiles";
import { useThreadTargetSync } from "@/features/channels/useThreadTargetSync";
import {
  useChannelMembersQuery,
  useJoinChannelMutation,
} from "@/features/channels/hooks";
import {
  MSG_PREFIX,
  THREAD_PREFIX,
} from "@/features/channels/readState/readStateFormat";
import { ChannelScreenEmptyState } from "@/features/channels/ui/ChannelScreenEmptyState";
import { ChannelScreenHeader } from "@/features/channels/ui/ChannelScreenHeader";
import { ChannelPane } from "@/features/channels/ui/ChannelScreenLazyViews";
import { WelcomeAgentCreateDialog } from "@/features/channels/ui/WelcomeAgentCreateDialog";
import { ForumChannelContent } from "@/features/channels/ui/ForumChannelContent";
import { MembersSidebar } from "@/features/channels/ui/MembersSidebar";
import {
  useManagedAgentsQuery,
  usePersonasQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { mergeChannelKnownAgentPubkeys } from "@/features/agents/knownAgentPubkeys";
import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { pickWelcomeGuideAgent } from "@/features/onboarding/welcomeGuide";
import { useWelcomeKickoffEntrance } from "@/features/onboarding/useWelcomeKickoffEntrance";
import { useWelcomeKickoffStagePresence } from "@/features/onboarding/useWelcomeKickoffStagePresence";
import { useWelcomeAgentCreate } from "@/features/channels/useWelcomeAgentCreate";
import {
  mergeMessages,
  useChannelMessagesQuery,
  useChannelSubscription,
  useChannelWindowQuery,
  useDeleteMessageMutation,
  useEditMessageMutation,
  useSendMessageMutation,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import {
  channelWindowThreadSummaries,
  type ChannelWindowThreadSummary,
} from "@/features/messages/lib/channelWindowStore";
import { getThreadReference } from "@/features/messages/lib/threading";
import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import {
  resolveTimelineLoadingLatch,
  selectTimelineLoadingState,
} from "@/features/messages/lib/timelineLoadingState";
import { useFetchOlderMessages } from "@/features/messages/useFetchOlderMessages";
import { useIndependentThreadPanel } from "@/features/messages/useIndependentThreadPanel";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import { useChannelTyping } from "@/features/messages/useChannelTyping";
import type { TimelineMessage } from "@/features/messages/types";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import type { RelayEvent, RespondToMode, SearchHit } from "@/shared/api/types";
import { useChannelFind } from "@/features/search/useChannelFind";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { AgentSessionProvider } from "@/shared/context/AgentSessionContext";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useMainInsetRef } from "@/shared/layout/MainInsetContext";
import { channelContentTopPaddingMeasurement } from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { useElementWidth } from "@/shared/hooks/use-mobile";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX } from "@/shared/layout/AuxiliaryPanel";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useChannelActivityTyping } from "./useChannelActivityTyping";
import { useChannelAgentSessions } from "./useChannelAgentSessions";
import { useMessageProfiles } from "./useMessageProfiles";
import { useChannelPanelHistoryState } from "./useChannelPanelHistoryState";
import { useChannelProfilePanel } from "./useChannelProfilePanel";
import { useChannelRouteTarget } from "./useChannelRouteTarget";
import { useChannelUnreadState } from "./useChannelUnreadState";
import type { ChannelScreenProps } from "./ChannelScreen.types";
const HEADER_ACTIONS_COMPACT_BREAKPOINT_PX = 760,
  EMPTY_RELAY_EVENTS: RelayEvent[] = [];
export function ChannelScreen({
  activeChannel,
  autoSendDraftKey,
  currentIdentity,
  currentProfile,
  onCloseForumPost,
  onSelectForumPost,
  selectedForumPostId,
  targetForumReplyId,
  targetMessageEvents,
  targetMessageId,
}: ChannelScreenProps) {
  const { goHome } = useAppNavigation();
  const {
    markChannelRead,
    markChannelUnread,
    getChannelReadAt,
    getMessageReadAt,
    markMessageRead,
    setContextParentResolver,
    openBrowseChannels,
    openCreateChannel,
    openChannelManagement: openGlobalChannelManagement,
    followThread,
    unfollowThread,
    isFollowingThread,
    isNotifiedForThread,
    isThreadMuted,
    readStateVersion,
  } = useAppShell();
  const {
    channelManagementOpen,
    clearAutoSend,
    clearMessageRouteTarget,
    openAgentSessionChannelId,
    openAgentSessionPubkey,
    openThreadHeadId,
    profilePanelPubkey,
    profilePanelTab,
    profilePanelView,
    setChannelManagementOpen,
    setOpenAgentSessionChannelId,
    setOpenAgentSessionPubkey,
    setOpenThreadHeadId,
    setProfilePanelTab,
    setProfilePanelPubkey,
    setProfilePanelView,
  } = useChannelPanelHistoryState();
  const {
    canReset: canResetThreadPanelWidth,
    onResetWidth: handleThreadPanelWidthReset,
    onResizeStart: handleThreadPanelResizeStart,
    widthPx: threadPanelWidthPx,
  } = useThreadPanelWidth();
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  const [isAddBotOpen, setIsAddBotOpen] = React.useState(false);
  const [channelContentRef, channelContentWidthPx] =
    useElementWidth<HTMLDivElement>();
  const [expandedThreadReplyIds, setExpandedThreadReplyIds] = React.useState(
    () => new Set<string>(),
  );
  const [threadScrollTargetId, setThreadScrollTargetId] = React.useState<
    string | null
  >(null);
  const [threadReplyTargetId, setThreadReplyTargetId] = React.useState<
    string | null
  >(null);
  const [editTargetId, setEditTargetId] = React.useState<string | null>(null);
  // URL-backed thread state catches up after navigation; this override keeps urgent open/close renders responsive.
  const [optimisticOpenThreadHeadId, setOptimisticOpenThreadHeadId] =
    React.useState<string | null | undefined>(undefined);
  const clearOptimisticThreadOverride = React.useCallback(() => {
    setOptimisticOpenThreadHeadId(undefined);
  }, []);
  const mainInsetRef = useMainInsetRef();
  const currentPubkey = currentIdentity?.pubkey;
  const activeChannelId = activeChannel?.id ?? null;
  const relaySelfPubkey = useRelaySelfQuery(activeChannel !== null).data;
  const effectiveOpenThreadHeadId =
    optimisticOpenThreadHeadId === undefined
      ? openThreadHeadId
      : optimisticOpenThreadHeadId;
  const isNotifiedForEffectiveThread =
    effectiveOpenThreadHeadId != null
      ? isNotifiedForThread(effectiveOpenThreadHeadId)
      : false;
  const previousActiveChannelIdRef = React.useRef(activeChannelId);
  React.useEffect(() => {
    const didChangeChannel =
      previousActiveChannelIdRef.current !== activeChannelId;
    previousActiveChannelIdRef.current = activeChannelId;
    setOptimisticOpenThreadHeadId((current) => {
      if (current === undefined) {
        return current;
      }
      return didChangeChannel || openThreadHeadId === current
        ? undefined
        : current;
    });
  }, [activeChannelId, openThreadHeadId]);
  const messagesQuery = useChannelMessagesQuery(activeChannel);
  const windowQuery = useChannelWindowQuery(activeChannel);
  const threadRepliesQuery = useThreadReplies(
    activeChannel,
    effectiveOpenThreadHeadId,
  );
  useChannelSubscription(activeChannel);
  const { fetchOlder, hasOlderMessages, historyExhausted, isFetchingOlder } =
    useFetchOlderMessages(activeChannel);
  const latestActiveMessage = React.useMemo(() => {
    const messages = messagesQuery.data;
    if (!messages) return null;
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (getThreadReference(messages[index].tags).parentId === null) {
        return messages[index];
      }
    }
    return null;
  }, [messagesQuery.data]);
  const activeReadAt = latestActiveMessage
    ? new Date(latestActiveMessage.created_at * 1_000).toISOString()
    : null;
  React.useEffect(() => {
    if (!activeChannelId || activeChannel?.isMember === false) {
      return;
    }
    markChannelRead(activeChannelId, activeReadAt, { topLevelOnly: true });
  }, [activeChannel?.isMember, activeChannelId, activeReadAt, markChannelRead]);
  React.useEffect(() => {
    if (!activeChannelId) {
      setContextParentResolver(null);
      return;
    }
    setContextParentResolver((contextId) =>
      contextId.startsWith(THREAD_PREFIX) || contextId.startsWith(MSG_PREFIX)
        ? activeChannelId
        : null,
    );
    return () => setContextParentResolver(null);
  }, [activeChannelId, setContextParentResolver]);
  const {
    activeChannelTitle,
    activeDmAvatarUrl,
    activeDmHeaderParticipants,
    activeDmPresenceStatus,
    activeChannelEphemeralDisplay,
  } = useActiveChannelHeader(activeChannel, currentPubkey);
  const sendMessageMutation = useSendMessageMutation(
    activeChannel,
    currentIdentity,
  );
  const toggleReactionMutation = useToggleReactionMutation();
  const deleteMessageMutation = useDeleteMessageMutation(activeChannel);
  const editMessageMutation = useEditMessageMutation(activeChannel);
  const joinChannelMutation = useJoinChannelMutation(activeChannelId);
  const [findEvents, setFindEvents] = React.useState<RelayEvent[]>([]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: clear spliced find results exactly when the active channel changes.
  React.useEffect(() => {
    setFindEvents([]);
  }, [activeChannelId]);
  const resolvedMessages = React.useMemo(() => {
    const currentMessages = messagesQuery.data ?? [];
    const extraEvents = [...targetMessageEvents, ...findEvents];
    if (!activeChannel || extraEvents.length === 0) {
      return currentMessages;
    }
    return extraEvents.reduce(mergeMessages, currentMessages);
  }, [activeChannel, findEvents, messagesQuery.data, targetMessageEvents]);
  const threadReplyEvents = threadRepliesQuery.data ?? EMPTY_RELAY_EVENTS;
  const {
    entranceMessageId: welcomeEntranceMessageId,
    handleEntranceComplete: handleWelcomeEntranceComplete,
  } = useWelcomeKickoffEntrance(
    activeChannel,
    resolvedMessages,
    threadReplyEvents,
  );

  const messageEventProfilePubkeys = useMessageEventProfilePubkeys(
    resolvedMessages,
    threadReplyEvents,
    relaySelfPubkey,
  );
  const latestMessageEvent = React.useMemo(
    () => resolvedMessages[resolvedMessages.length - 1] ?? null,
    [resolvedMessages],
  );
  const typingEntries = useChannelTyping(
    activeChannel,
    currentPubkey,
    latestMessageEvent,
    relaySelfPubkey,
  );
  const activeDmParticipantPubkeys = React.useMemo(
    () =>
      activeChannel?.channelType === "dm"
        ? activeChannel.participantPubkeys
        : [],
    [activeChannel],
  );
  const channelMembersQuery = useChannelMembersQuery(activeChannel?.id ?? null);
  const channelMembers = channelMembersQuery.data;
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data ?? [];
  const welcomeGuideAgent = React.useMemo(
    () => pickWelcomeGuideAgent(managedAgents),
    [managedAgents],
  );
  const welcomeAgentCreate = useWelcomeAgentCreate({
    activeChannel,
    currentIdentity,
    welcomeGuideAgent,
  });
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgents = relayAgentsQuery.data ?? [];
  const knownAgentPubkeys = React.useMemo(
    () =>
      mergeChannelKnownAgentPubkeys(channelMembers, managedAgents, relayAgents),
    [channelMembers, managedAgents, relayAgents],
  );
  const messageProfilePubkeys = React.useMemo(
    () => [
      ...new Set([
        ...messageEventProfilePubkeys,
        ...activeDmParticipantPubkeys,
        ...knownAgentPubkeys,
        ...typingEntries.map((entry) => entry.pubkey),
      ]),
    ],
    [
      activeDmParticipantPubkeys,
      knownAgentPubkeys,
      messageEventProfilePubkeys,
      typingEntries,
    ],
  );
  const messageProfilesQuery = useUsersBatchQuery(messageProfilePubkeys, {
    enabled: messageProfilePubkeys.length > 0,
  });
  const agentPubkeysPending =
    activeChannel?.channelType === "dm" &&
    (channelMembersQuery.isPending ||
      managedAgentsQuery.isPending ||
      relayAgentsQuery.isPending ||
      (messageProfilePubkeys.length > 0 &&
        (messageProfilesQuery.isPending ||
          messageProfilesQuery.isPlaceholderData)));
  const {
    agentSessionCandidates,
    botTypingEntries,
    humanTypingPubkeys,
    threadTypingPubkeys,
  } = useChannelActivityTyping({
    activeChannel,
    activeChannelId,
    channelMembers,
    managedAgents,
    openThreadHeadId: effectiveOpenThreadHeadId,
    relayAgents,
    typingEntries,
  });
  const messageProfiles = useMessageProfiles({
    channelMembers,
    currentProfile,
    currentPubkey,
    managedAgents,
    profiles: messageProfilesQuery.data?.profiles,
    relayAgents,
  });
  const messageOwnerProfiles = useMessageOwnerProfiles(messageProfiles);
  // Agent set for ChannelPane's own consumers (DM huddle member resolution,
  // the agents list): the community-scoped baseline shared by every surface,
  // widened with channel-member roles and this screen's profile lookup.
  // Message rows no longer take this — MessageRow derives agent-ness itself
  // from useKnownAgentPubkeys + per-pubkey profile checks.
  const communityAgentPubkeys = useKnownAgentPubkeys();
  const agentPubkeys = React.useMemo(() => {
    const pubkeys = new Set([...communityAgentPubkeys, ...knownAgentPubkeys]);
    for (const [pubkey, profile] of Object.entries(messageProfiles)) {
      if (profile.isAgent) {
        pubkeys.add(normalizePubkey(pubkey));
      }
    }
    return pubkeys;
  }, [knownAgentPubkeys, messageProfiles, communityAgentPubkeys]);
  const personasQuery = usePersonasQuery();
  const { personaLookup, respondToLookup } = React.useMemo(() => {
    const agents = managedAgentsQuery.data ?? [];
    const personaById = new Map(
      (personasQuery.data ?? []).map((p) => [p.id, p.displayName]),
    );
    const pLookup = new Map<string, string>();
    const rLookup = new Map<string, RespondToMode>();
    for (const agent of agents) {
      const key = agent.pubkey.toLowerCase();
      rLookup.set(key, agent.respondTo);
      const pName = agent.personaId ? personaById.get(agent.personaId) : null;
      if (pName) pLookup.set(key, pName);
    }
    return { personaLookup: pLookup, respondToLookup: rLookup };
  }, [managedAgentsQuery.data, personasQuery.data]);
  const timelineMessages = React.useMemo(
    () =>
      formatTimelineMessages(
        resolvedMessages,
        activeChannel,
        currentPubkey,
        currentProfile?.avatarUrl ?? null,
        messageProfiles,
        channelMembers,
        personaLookup,
        respondToLookup,
        relaySelfPubkey,
        messageOwnerProfiles,
      ),
    [
      activeChannel,
      channelMembers,
      currentProfile?.avatarUrl,
      currentPubkey,
      messageProfiles,
      messageOwnerProfiles,
      personaLookup,
      relaySelfPubkey,
      respondToLookup,
      resolvedMessages,
    ],
  );
  const threadSummaries: ReadonlyMap<string, ChannelWindowThreadSummary> =
    React.useMemo(
      () =>
        windowQuery.data
          ? channelWindowThreadSummaries(windowQuery.data)
          : new Map(),
      [windowQuery.data],
    );
  const handleFindSearchHit = React.useCallback((hit: SearchHit) => {
    const event = cacheSearchHitEvent(hit);
    setFindEvents((currentEvents) =>
      currentEvents.some((currentEvent) => currentEvent.id === event.id)
        ? currentEvents
        : [...currentEvents, event],
    );
  }, []);
  const channelFind = useChannelFind({
    channelId: activeChannelId,
    messages: timelineMessages,
    onSearchHit: handleFindSearchHit,
  });
  const threadPanelData = useIndependentThreadPanel({
    activeChannel,
    channelEvents: resolvedMessages,
    threadReplyEvents,
    rootId: effectiveOpenThreadHeadId,
    replyTargetId: threadReplyTargetId,
    expandedReplyIds: expandedThreadReplyIds,
    currentPubkey,
    currentAvatarUrl: currentProfile?.avatarUrl ?? null,
    profiles: messageProfiles,
    ownerProfiles: messageOwnerProfiles,
    members: channelMembers,
    personaLookup,
    respondToLookup,
    relaySelfPubkey,
  });
  const {
    firstUnreadMessageId,
    getFirstReplyIdForMessage,
    getReplyDescendantIdsForMessage,
    handleMarkMessageRead,
    handleMarkMessageUnread,
    isMessageUnread,
    markRevealedRepliesRead,
    openThreadHeadMessage,
    threadFirstUnreadReplyId,
    threadReplyTargetMessage,
    threadReplyUnreadCounts,
    threadUnreadCounts,
    unreadCount,
  } = useChannelUnreadState({
    activeChannelId,
    timelineMessages,
    currentPubkey,
    openThreadHeadId: effectiveOpenThreadHeadId,
    threadReplyTargetId,
    expandedThreadReplyIds,
    openThreadMessages: threadPanelData.visibleReplies,
    getChannelReadAt,
    getMessageReadAt,
    markChannelUnread,
    markMessageRead,
    isThreadMuted,
    readStateVersion,
  });
  const editTargetMessage = React.useMemo(
    () =>
      timelineMessages.find((message) => message.id === editTargetId) ?? null,
    [editTargetId, timelineMessages],
  );
  const {
    handleCancelEdit,
    handleCancelThreadReply,
    handleCloseThread,
    handleDelete,
    handleEdit,
    handleEditSave,
    handleExpandThreadReplies,
    handleOpenThread,
    handleSendMessage,
    handleSendThreadReply,
    handleSelectThreadReplyTarget,
    handleToggleReaction,
  } = useChannelPaneHandlers({
    deleteMessageMutation,
    editMessageMutation,
    editTargetId,
    expandedThreadReplyIds,
    getFirstReplyIdForMessage,
    getReplyDescendantIdsForMessage,
    markRevealedRepliesRead,
    openThreadHeadId: effectiveOpenThreadHeadId,
    onOptimisticOpenThreadHeadIdChange: setOptimisticOpenThreadHeadId,
    sendMessageMutation,
    setExpandedThreadReplyIds,
    setEditTargetId,
    setOpenThreadHeadId,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    threadReplyTargetId,
    toggleReactionMutation,
  });
  const effectiveToggleReaction = React.useMemo(
    () =>
      activeChannel && !activeChannel.archivedAt && activeChannel.isMember
        ? handleToggleReaction
        : undefined,
    [activeChannel, handleToggleReaction],
  );
  const handleMessageMarkUnread = React.useCallback(
    (message: TimelineMessage) => handleMarkMessageUnread(message.id),
    [handleMarkMessageUnread],
  );
  const handleMessageMarkRead = React.useCallback(
    (message: TimelineMessage) => handleMarkMessageRead(message.id),
    [handleMarkMessageRead],
  );
  const sendMessageMutateAsync = sendMessageMutation.mutateAsync;
  const handleSendVideoReviewComment = React.useCallback(
    async (
      message: { id: string },
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      parentEventId?: string,
    ) => {
      await sendMessageMutateAsync({
        content,
        mediaTags,
        mentionPubkeys,
        parentEventId: parentEventId ?? message.id,
      });
    },
    [sendMessageMutateAsync],
  );
  const effectiveSendVideoReviewComment =
    activeChannel && !activeChannel.archivedAt && activeChannel.isMember
      ? handleSendVideoReviewComment
      : undefined;
  const handleOpenAddBot = React.useCallback(
    (options?: { beforeSend?: () => void }) =>
      welcomeAgentCreate.openAddAgent(() => setIsAddBotOpen(true), options),
    [welcomeAgentCreate],
  );
  const handleOpenMembersSidebar = React.useCallback(
    () => setIsMembersSidebarOpen(true),
    [],
  );
  const handleCloseChannelManagement = React.useCallback(
    () => setChannelManagementOpen(false),
    [setChannelManagementOpen],
  );
  const handleChannelManagementDeleted = React.useCallback(() => {
    setChannelManagementOpen(false);
    void goHome({ replace: true });
  }, [setChannelManagementOpen, goHome]);
  const {
    agentSessionAgents,
    backFromAgentSession: handleBackFromAgentSession,
    channelAgentSessionAgents,
    closeAgentSession: handleCloseAgentSession,
    hasAgentSessionReturnTarget,
    openAgentSession: handleOpenAgentSession,
    openThreadAndCloseAgentSession: handleOpenThreadAndCloseAgentSession,
  } = useChannelAgentSessions({
    activeChannel,
    activeChannelId,
    agentsLoaded:
      !channelMembersQuery.isLoading &&
      !managedAgentsQuery.isLoading &&
      !relayAgentsQuery.isLoading,
    channelMembers,
    handleOpenThread,
    managedAgents: agentSessionCandidates,
    openAgentSessionPubkey,
    openThreadHeadId: effectiveOpenThreadHeadId,
    profilePanelPubkey,
    setChannelManagementOpen,
    setExpandedThreadReplyIds,
    setOpenAgentSessionChannelId,
    setOpenAgentSessionPubkey,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  });
  const { handleOpenProfilePanel, handleCloseProfilePanel, handleOpenDm } =
    useChannelProfilePanel({
      closeAgentSession: handleCloseAgentSession,
      setChannelManagementOpen,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    });
  const settledChannelIdRef = React.useRef<string | null>(null);
  const hasSettledThisChannel =
    activeChannelId !== null && settledChannelIdRef.current === activeChannelId;
  const timelineLoadingNow =
    activeChannel !== null &&
    activeChannel.channelType !== "forum" &&
    selectTimelineLoadingState(
      {
        isPending: messagesQuery.isPending,
        isFetching: messagesQuery.isFetching,
        isPlaceholderData: messagesQuery.isPlaceholderData,
        dataLength: messagesQuery.data?.length ?? null,
      },
      hasSettledThisChannel,
    );
  const { settledChannelId, isLoading: isTimelineLoading } =
    resolveTimelineLoadingLatch(
      settledChannelIdRef.current,
      activeChannelId,
      timelineLoadingNow,
    );
  settledChannelIdRef.current = settledChannelId;
  const { welcomeKickoffStage, welcomeKickoffSettingUp } =
    useWelcomeKickoffStagePresence(
      activeChannel,
      timelineMessages,
      isTimelineLoading,
    );
  const resetComposerTargets = React.useCallback(
    (_channelId: string | null) => {
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      setEditTargetId(null);
    },
    [],
  );
  const handleThreadScrollTargetResolved = React.useCallback(() => {
    setThreadScrollTargetId(null);
  }, []);
  const handleTargetReached = React.useCallback(() => {
    clearMessageRouteTarget({ replace: true });
  }, [clearMessageRouteTarget]);
  React.useEffect(() => {
    resetComposerTargets(activeChannelId);
  }, [activeChannelId, resetComposerTargets]);
  const mainTimelineTargetMessageId = useChannelRouteTarget({
    activeChannel,
    activeChannelId,
    closeAgentSession: handleCloseAgentSession,
    setEditTargetId,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    targetMessageId,
    timelineMessages,
  });
  useThreadTargetSync({
    clearOptimisticThreadOverride,
    editTargetId,
    editTargetMessage,
    isTimelineLoading,
    openThreadHeadId,
    openThreadHeadMessage,
    setEditTargetId,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    threadReplyTargetId,
    threadReplyTargetMessage,
  });

  const hasAuxiliaryPanel = Boolean(
    effectiveOpenThreadHeadId ||
      openAgentSessionPubkey ||
      profilePanelPubkey ||
      channelManagementOpen,
  );
  const displayedThreadHeadMessage = threadPanelData.threadHead;
  const displayedThreadMessages = threadPanelData.visibleReplies;
  const displayedThreadReplyTargetMessage = threadPanelData.replyTargetMessage;
  const displayedThreadFirstUnreadReplyId = displayedThreadHeadMessage
    ? threadFirstUnreadReplyId
    : null;
  const shouldShowThreadSkeleton = Boolean(
    effectiveOpenThreadHeadId && activeChannel && !displayedThreadHeadMessage,
  );
  const isNarrowPanelViewport =
    channelContentWidthPx > 0 &&
    channelContentWidthPx < AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX;
  const isSinglePanelView =
    isNarrowPanelViewport &&
    activeChannel?.channelType !== "forum" &&
    hasAuxiliaryPanel;
  const shouldCompactHeaderActions =
    hasAuxiliaryPanel &&
    channelContentWidthPx > 0 &&
    channelContentWidthPx < HEADER_ACTIONS_COMPACT_BREAKPOINT_PX;
  const channelHeaderChromeRef = useMeasuredCssVariable({
    targetRef: mainInsetRef,
    ...channelContentTopPaddingMeasurement,
    resetKey: activeChannelId,
    enabled: !isSinglePanelView,
  });

  const handleManageChannel = React.useCallback(() => {
    if (activeChannel?.channelType === "forum") {
      openGlobalChannelManagement();
      return;
    }

    if (channelManagementOpen) {
      setChannelManagementOpen(false);
      return;
    }

    setOpenThreadHeadId(null);
    setExpandedThreadReplyIds(new Set());
    setThreadScrollTargetId(null);
    setThreadReplyTargetId(null);
    handleCloseAgentSession();
    setProfilePanelPubkey(null);
    setChannelManagementOpen(true);
  }, [
    activeChannel?.channelType,
    channelManagementOpen,
    openGlobalChannelManagement,
    setChannelManagementOpen,
    setOpenThreadHeadId,
    handleCloseAgentSession,
    setProfilePanelPubkey,
  ]);
  const handleToggleMembers = React.useCallback(
    () => setIsMembersSidebarOpen((prev) => !prev),
    [],
  );

  const channelHeader = React.useMemo(
    () => (
      <ChannelScreenHeader
        activeChannel={activeChannel}
        activeChannelEphemeralDisplay={activeChannelEphemeralDisplay}
        activeChannelTitle={activeChannelTitle}
        actionsVariant={shouldCompactHeaderActions ? "compact" : "inline"}
        activeDmAvatarUrl={activeDmAvatarUrl}
        activeDmHeaderParticipants={activeDmHeaderParticipants}
        activeDmPresenceStatus={activeDmPresenceStatus}
        chromeWrapperRef={channelHeaderChromeRef}
        currentPubkey={currentPubkey}
        isAddBotOpen={isAddBotOpen}
        isJoining={joinChannelMutation.isPending}
        onAddBotOpenChange={setIsAddBotOpen}
        onJoinChannel={joinChannelMutation.mutateAsync}
        onManageChannel={handleManageChannel}
        onToggleMembers={handleToggleMembers}
        showHeaderContent={!isSinglePanelView}
        transparentChrome={activeChannel?.channelType !== "forum"}
      />
    ),
    [
      activeChannel,
      activeChannelEphemeralDisplay,
      activeChannelTitle,
      shouldCompactHeaderActions,
      activeDmAvatarUrl,
      activeDmHeaderParticipants,
      activeDmPresenceStatus,
      channelHeaderChromeRef,
      currentPubkey,
      isAddBotOpen,
      joinChannelMutation.isPending,
      joinChannelMutation.mutateAsync,
      handleManageChannel,
      handleToggleMembers,
      isSinglePanelView,
    ],
  );

  return (
    <AgentSessionProvider onOpenAgentSession={handleOpenAgentSession}>
      <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
        <WelcomeAgentCreateDialog
          guideName={welcomeGuideAgent?.name ?? "your welcome guide"}
          isSending={welcomeAgentCreate.isSending}
          onCreateInChat={() => void welcomeAgentCreate.createInChat()}
          onCreateManually={welcomeAgentCreate.createManually}
          onOpenChange={welcomeAgentCreate.setIsOpen}
          open={welcomeAgentCreate.isOpen}
          sendError={welcomeAgentCreate.error}
        />
        <div
          className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
          ref={channelContentRef}
        >
          {activeChannel ? (
            activeChannel.channelType === "forum" ? (
              <ForumChannelContent
                canResetPanelWidth={canResetThreadPanelWidth}
                channel={activeChannel}
                currentPubkey={currentPubkey}
                header={channelHeader}
                onClosePost={onCloseForumPost}
                onCloseProfilePanel={handleCloseProfilePanel}
                onOpenDm={handleOpenDm}
                onOpenProfilePanel={handleOpenProfilePanel}
                onPanelResizeStart={handleThreadPanelResizeStart}
                onProfilePanelTabChange={setProfilePanelTab}
                onProfilePanelViewChange={setProfilePanelView}
                onResetPanelWidth={handleThreadPanelWidthReset}
                onSelectPost={onSelectForumPost}
                panelWidthPx={threadPanelWidthPx}
                profilePanelPubkey={profilePanelPubkey}
                profilePanelTab={profilePanelTab}
                profilePanelView={profilePanelView}
                selectedPostId={selectedForumPostId}
                targetReplyId={targetForumReplyId}
              />
            ) : (
              <React.Suspense
                fallback={<ViewLoadingFallback includeHeader kind="channel" />}
              >
                <ChannelPane
                  activeChannel={activeChannel}
                  activityAgents={channelAgentSessionAgents}
                  agentPubkeys={agentPubkeys}
                  agentPubkeysPending={agentPubkeysPending}
                  agentSessionAgents={agentSessionAgents}
                  autoSendDraftKey={autoSendDraftKey}
                  onAutoSendComplete={clearAutoSend}
                  botTypingEntries={botTypingEntries}
                  channelFind={channelFind}
                  channelManagementOpen={channelManagementOpen}
                  currentPubkey={currentPubkey}
                  canResetThreadPanelWidth={canResetThreadPanelWidth}
                  fetchOlder={fetchOlder}
                  header={channelHeader}
                  hasOlderMessages={hasOlderMessages}
                  historyExhausted={historyExhausted}
                  onAddAgent={handleOpenAddBot}
                  onBrowseChannels={openBrowseChannels}
                  onCreateChannel={openCreateChannel}
                  onOpenMembers={handleOpenMembersSidebar}
                  isFetchingOlder={isFetchingOlder}
                  entranceMessageId={welcomeEntranceMessageId}
                  onEntranceMessageComplete={handleWelcomeEntranceComplete}
                  welcomeKickoffStage={welcomeKickoffStage}
                  welcomeKickoffSettingUp={welcomeKickoffSettingUp}
                  editTarget={
                    editTargetMessage
                      ? {
                          author: editTargetMessage.author,
                          body: editTargetMessage.body,
                          id: editTargetMessage.id,
                          imetaMedia: imetaMediaFromTags(
                            editTargetMessage.tags,
                          ),
                        }
                      : null
                  }
                  followThreadById={followThread}
                  unfollowThreadById={unfollowThread}
                  isFollowingThreadById={isFollowingThread}
                  isMessageUnreadById={isMessageUnread}
                  isFollowingThread={isNotifiedForEffectiveThread}
                  isSending={sendMessageMutation.isPending}
                  isSinglePanelView={isSinglePanelView}
                  isTimelineLoading={isTimelineLoading}
                  messages={timelineMessages}
                  threadSummaries={threadSummaries}
                  onCancelEdit={handleCancelEdit}
                  onCancelThreadReply={handleCancelThreadReply}
                  onChannelManagementDeleted={handleChannelManagementDeleted}
                  onFollowThread={
                    effectiveOpenThreadHeadId != null &&
                    !isNotifiedForEffectiveThread
                      ? () => followThread(effectiveOpenThreadHeadId)
                      : undefined
                  }
                  onUnfollowThread={
                    effectiveOpenThreadHeadId != null &&
                    isNotifiedForEffectiveThread
                      ? () => unfollowThread(effectiveOpenThreadHeadId)
                      : undefined
                  }
                  onCloseAgentSession={handleCloseAgentSession}
                  onBackFromAgentSession={
                    hasAgentSessionReturnTarget
                      ? handleBackFromAgentSession
                      : undefined
                  }
                  onCloseChannelManagement={handleCloseChannelManagement}
                  onCloseThread={handleCloseThread}
                  onDelete={
                    activeChannel?.archivedAt ? undefined : handleDelete
                  }
                  onEdit={activeChannel?.archivedAt ? undefined : handleEdit}
                  onEditSave={
                    activeChannel?.archivedAt ? undefined : handleEditSave
                  }
                  onMarkUnread={handleMessageMarkUnread}
                  onMarkRead={handleMessageMarkRead}
                  onExpandThreadReplies={handleExpandThreadReplies}
                  onOpenAgentSession={handleOpenAgentSession}
                  onOpenDm={handleOpenDm}
                  onOpenProfilePanel={handleOpenProfilePanel}
                  onResetThreadPanelWidth={handleThreadPanelWidthReset}
                  onCloseProfilePanel={handleCloseProfilePanel}
                  onOpenThread={handleOpenThreadAndCloseAgentSession}
                  onSelectThreadReplyTarget={handleSelectThreadReplyTarget}
                  onSendMessage={handleSendMessage}
                  onSendVideoReviewComment={effectiveSendVideoReviewComment}
                  onSendThreadReply={handleSendThreadReply}
                  onThreadScrollTargetResolved={
                    handleThreadScrollTargetResolved
                  }
                  onThreadPanelResizeStart={handleThreadPanelResizeStart}
                  onTargetReached={handleTargetReached}
                  onToggleReaction={effectiveToggleReaction}
                  openAgentSessionChannelId={openAgentSessionChannelId}
                  openAgentSessionPubkey={openAgentSessionPubkey}
                  openThreadHeadId={effectiveOpenThreadHeadId}
                  shouldShowThreadSkeleton={shouldShowThreadSkeleton}
                  onProfilePanelViewChange={setProfilePanelView}
                  onProfilePanelTabChange={setProfilePanelTab}
                  profilePanelPubkey={profilePanelPubkey}
                  profilePanelTab={profilePanelTab}
                  profilePanelView={profilePanelView}
                  personaLookup={personaLookup}
                  profiles={messageProfiles}
                  ownerProfiles={messageOwnerProfiles}
                  firstUnreadMessageId={firstUnreadMessageId}
                  unreadCount={unreadCount}
                  targetMessageId={mainTimelineTargetMessageId}
                  threadHeadMessage={displayedThreadHeadMessage}
                  threadMessages={displayedThreadMessages}
                  threadMessagesPending={threadRepliesQuery.isPending}
                  threadPanelWidthPx={threadPanelWidthPx}
                  threadTypingPubkeys={threadTypingPubkeys}
                  threadReplyTargetMessage={displayedThreadReplyTargetMessage}
                  threadScrollTargetId={threadScrollTargetId}
                  threadUnreadCounts={threadUnreadCounts}
                  threadReplyUnreadCounts={threadReplyUnreadCounts}
                  threadFirstUnreadReplyId={displayedThreadFirstUnreadReplyId}
                  isJoining={joinChannelMutation.isPending}
                  onJoinChannel={joinChannelMutation.mutateAsync}
                  typingPubkeys={humanTypingPubkeys}
                />
              </React.Suspense>
            )
          ) : (
            <ChannelScreenEmptyState />
          )}
        </div>

        <MembersSidebar
          channel={activeChannel}
          currentPubkey={currentPubkey}
          open={isMembersSidebarOpen}
          onOpenChange={setIsMembersSidebarOpen}
          onViewActivity={handleOpenAgentSession}
        />
      </ProfilePanelProvider>
    </AgentSessionProvider>
  );
}
