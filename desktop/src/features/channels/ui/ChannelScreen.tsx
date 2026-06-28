import * as React from "react";
import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useActiveChannelHeader } from "@/features/channels/useActiveChannelHeader";
import { useChannelPaneHandlers } from "@/features/channels/useChannelPaneHandlers";
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
import {
  ChannelPane,
  ForumView,
} from "@/features/channels/ui/ChannelScreenLazyViews";
import { MembersSidebar } from "@/features/channels/ui/MembersSidebar";
import {
  useManagedAgentsQuery,
  usePersonasQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import {
  mergeMessages,
  useChannelMessagesQuery,
  useChannelSubscription,
  useDeleteMessageMutation,
  useEditMessageMutation,
  useSendMessageMutation,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import {
  collectMessageAuthorPubkeys,
  collectMessageMentionPubkeys,
  formatTimelineMessages,
} from "@/features/messages/lib/formatTimelineMessages";
import { getThreadReference } from "@/features/messages/lib/threading";
import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import {
  resolveTimelineLoadingLatch,
  selectTimelineLoadingState,
} from "@/features/messages/lib/timelineLoadingState";
import { useFetchOlderMessages } from "@/features/messages/useFetchOlderMessages";
import { useLoadMissingAncestors } from "@/features/messages/useLoadMissingAncestors";
import { useChannelTyping } from "@/features/messages/useChannelTyping";
import type { TimelineMessage } from "@/features/messages/types";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import type { RespondToMode } from "@/shared/api/types";
import { useChannelFind } from "@/features/search/useChannelFind";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { AgentSessionProvider } from "@/shared/context/AgentSessionContext";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useMainInsetRef } from "@/shared/layout/MainInsetContext";
import { channelContentTopPaddingMeasurement } from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { useElementWidth } from "@/shared/hooks/use-mobile";
import {
  THREAD_PANEL_SINGLE_COLUMN_BREAKPOINT_PX,
  useThreadPanelWidth,
} from "@/shared/hooks/useThreadPanelWidth";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  mergeAgentNamesIntoProfiles,
  useChannelActivityTyping,
} from "./useChannelActivityTyping";
import { useChannelAgentSessions } from "./useChannelAgentSessions";
import { useChannelPanelHistoryState } from "./useChannelPanelHistoryState";
import { useChannelProfilePanel } from "./useChannelProfilePanel";
import { useChannelRouteTarget } from "./useChannelRouteTarget";
import { useChannelUnreadState } from "./useChannelUnreadState";
import type { ChannelScreenProps } from "./ChannelScreen.types";

const HEADER_ACTIONS_COMPACT_BREAKPOINT_PX = 760;

export function ChannelScreen({
  activeChannel,
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
    clearMessageRouteTarget,
    openAgentSessionPubkey,
    openThreadHeadId,
    profilePanelPubkey,
    profilePanelTab,
    profilePanelView,
    setChannelManagementOpen,
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
  // Thread panel state is URL-backed, but router navigation is intentionally
  // deferred out of the click handler. Keep a tiny optimistic override so the
  // auxiliary pane can open/close in the urgent render, then let the URL-backed
  // state hydrate the real thread contents when it catches up.
  const [optimisticOpenThreadHeadId, setOptimisticOpenThreadHeadId] =
    React.useState<string | null | undefined>(undefined);
  const clearOptimisticThreadOverride = React.useCallback(() => {
    setOptimisticOpenThreadHeadId(undefined);
  }, []);
  const mainInsetRef = useMainInsetRef();
  const currentPubkey = currentIdentity?.pubkey;
  const activeChannelId = activeChannel?.id ?? null;
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
  useChannelSubscription(activeChannel);
  const { fetchOlder, hasOlderMessages, isFetchingOlder } =
    useFetchOlderMessages(activeChannel);
  // Newest top-level message only: opening a channel should clear the timeline
  // without clearing unread thread replies.
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
  // No `lastMessageAt` fallback: it is reply-inclusive and would clear unread
  // thread/sidebar state before a real top-level position is known.
  const activeReadAt = latestActiveMessage
    ? new Date(latestActiveMessage.created_at * 1_000).toISOString()
    : null;
  React.useEffect(() => {
    if (!activeChannelId || activeChannel?.isMember === false) {
      return;
    }
    // Passive channel-open: advance the marker to the newest top-level message
    // only (NIP-RS Option 1). Opening a channel clears the main timeline while
    // leaving thread badges and Home inbox thread activity intact until each
    // thread itself is read.
    markChannelRead(activeChannelId, activeReadAt, { topLevelOnly: true });
  }, [activeChannel?.isMember, activeChannelId, activeReadAt, markChannelRead]);
  // Install the NIP-RS parent resolver: every `thread:<root>` or `msg:<id>`
  // context evaluated while this channel is active belongs to it (both are only
  // ever read for the active channel's timeline messages), so the parent is
  // always the active channel. Folding `msg:` to the channel — never to another
  // message — means reading an ancestor never covers a descendant (LP4 Issue 2
  // by construction); a channel-read still clears any message older than the
  // top-level channel frontier. Non-thread/non-message keys (channels) have no
  // parent → null, which degrades effective() to the own term. Cleared on
  // channel leave / unmount so a stale channel id never becomes the parent of
  // another channel's contexts.
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
  const resolvedMessages = React.useMemo(() => {
    const currentMessages = messagesQuery.data ?? [];
    if (!activeChannel || targetMessageEvents.length === 0) {
      return currentMessages;
    }
    return targetMessageEvents.reduce(mergeMessages, currentMessages);
  }, [activeChannel, messagesQuery.data, targetMessageEvents]);
  const messageAuthorPubkeys = React.useMemo(
    () => collectMessageAuthorPubkeys(resolvedMessages),
    [resolvedMessages],
  );
  const messageMentionPubkeys = React.useMemo(
    () => collectMessageMentionPubkeys(resolvedMessages),
    [resolvedMessages],
  );
  const latestMessageEvent = React.useMemo(
    () => resolvedMessages[resolvedMessages.length - 1] ?? null,
    [resolvedMessages],
  );
  const typingEntries = useChannelTyping(
    activeChannel,
    currentPubkey,
    latestMessageEvent,
  );
  const activeDmParticipantPubkeys = React.useMemo(
    () =>
      activeChannel?.channelType === "dm"
        ? activeChannel.participantPubkeys
        : [],
    [activeChannel],
  );
  const messageProfilePubkeys = React.useMemo(
    () => [
      ...new Set([
        ...messageAuthorPubkeys,
        ...messageMentionPubkeys,
        ...activeDmParticipantPubkeys,
        ...typingEntries.map((entry) => entry.pubkey),
      ]),
    ],
    [
      activeDmParticipantPubkeys,
      messageAuthorPubkeys,
      messageMentionPubkeys,
      typingEntries,
    ],
  );
  const messageProfilesQuery = useUsersBatchQuery(messageProfilePubkeys, {
    enabled: messageProfilePubkeys.length > 0,
  });
  const channelMembersQuery = useChannelMembersQuery(activeChannel?.id ?? null);
  const channelMembers = channelMembersQuery.data;
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data ?? [];
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgents = relayAgentsQuery.data ?? [];
  const agentPubkeys = React.useMemo(() => {
    const pubkeys = new Set<string>();
    for (const member of channelMembers ?? []) {
      if (member.role === "bot" || member.isAgent) {
        pubkeys.add(normalizePubkey(member.pubkey));
      }
    }
    for (const agent of managedAgents) {
      pubkeys.add(normalizePubkey(agent.pubkey));
    }
    for (const agent of relayAgents) {
      pubkeys.add(normalizePubkey(agent.pubkey));
    }
    return pubkeys;
  }, [channelMembers, managedAgents, relayAgents]);
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
  const observerBridgeAgents = React.useMemo(() => {
    if (
      !profilePanelPubkey ||
      !openAgentSessionPubkey ||
      normalizePubkey(profilePanelPubkey) !==
        normalizePubkey(openAgentSessionPubkey) ||
      managedAgents.some(
        (agent) =>
          normalizePubkey(agent.pubkey) === normalizePubkey(profilePanelPubkey),
      )
    ) {
      return managedAgents;
    }

    return [
      ...managedAgents,
      {
        pubkey: profilePanelPubkey,
        status: "deployed" as const,
      },
    ];
  }, [managedAgents, openAgentSessionPubkey, profilePanelPubkey]);
  useManagedAgentObserverBridge(observerBridgeAgents);
  const messageProfiles = React.useMemo(() => {
    const base =
      mergeCurrentProfileIntoLookup(
        messageProfilesQuery.data?.profiles,
        currentProfile,
      ) ?? {};
    return mergeAgentNamesIntoProfiles(base, managedAgents, relayAgents);
  }, [
    currentProfile,
    managedAgents,
    messageProfilesQuery.data?.profiles,
    relayAgents,
  ]);
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
      ),
    [
      activeChannel,
      channelMembers,
      currentProfile?.avatarUrl,
      currentPubkey,
      messageProfiles,
      personaLookup,
      respondToLookup,
      resolvedMessages,
    ],
  );
  const channelFind = useChannelFind({
    channelId: activeChannelId,
    messages: timelineMessages,
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
    threadMessages,
    threadReplyTargetMessage,
    threadReplyUnreadCounts,
    threadUnreadCounts,
    unreadCount,
  } = useChannelUnreadState({
    activeChannelId,
    timelineMessages,
    currentPubkey,
    openThreadHeadId,
    threadReplyTargetId,
    expandedThreadReplyIds,
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
  // The menu actions are typed (message) => void; the per-message read-state
  // handlers key off the message id (message + subtree). Adapt at the seam so
  // the handlers stay id-based and the menu stays message-based.
  const handleMessageMarkUnread = React.useCallback(
    (message: TimelineMessage) => handleMarkMessageUnread(message.id),
    [handleMarkMessageUnread],
  );
  const handleMessageMarkRead = React.useCallback(
    (message: TimelineMessage) => handleMarkMessageRead(message.id),
    [handleMarkMessageRead],
  );
  const handleSendVideoReviewComment = React.useCallback(
    async (
      message: { id: string },
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      parentEventId?: string,
    ) => {
      await sendMessageMutation.mutateAsync({
        content,
        mediaTags,
        mentionPubkeys,
        parentEventId: parentEventId ?? message.id,
      });
    },
    [sendMessageMutation],
  );
  const effectiveSendVideoReviewComment =
    activeChannel && !activeChannel.archivedAt && activeChannel.isMember
      ? handleSendVideoReviewComment
      : undefined;
  const {
    agentSessionAgents,
    channelAgentSessionAgents,
    closeAgentSession: handleCloseAgentSession,
    openAgentSession: handleOpenAgentSession,
    openThreadAndCloseAgentSession: handleOpenThreadAndCloseAgentSession,
  } = useChannelAgentSessions({
    activeChannel,
    activeChannelId,
    // The agent list comes from three queries; treat it as loaded only once
    // none of them are in their initial fetch, so a channel with genuinely
    // zero agents can still auto-close a stale agentSession param. A disabled
    // query (e.g. no active channel) reports isLoading=false, which is fine.
    agentsLoaded:
      !channelMembersQuery.isLoading &&
      !managedAgentsQuery.isLoading &&
      !relayAgentsQuery.isLoading,
    channelMembers,
    handleOpenThread,
    managedAgents: agentSessionCandidates,
    openAgentSessionPubkey,
    profilePanelPubkey,
    setChannelManagementOpen,
    setExpandedThreadReplyIds,
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
  // `data !== undefined` is not "loaded": the cache is seeded early by stale
  // placeholders and the live subscription. Wait for the history fetch to settle.
  // Latch loaded per channel so a later background refetch can't flip back to
  // the skeleton — that re-flip is the "skeleton bouncing up and down" on entry.
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
  // Panel identity (thread/profile/agent session) lives in the URL search
  // params, so channel changes and back/forward traversals carry it per
  // history entry — only the local ephemeral targets need resetting here.
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
  React.useEffect(() => {
    if (openThreadHeadId && !openThreadHeadMessage) {
      // While the timeline is still loading (e.g. a reload restoring the
      // thread param from the URL) the head simply hasn't arrived yet.
      if (isTimelineLoading) {
        return;
      }
      clearOptimisticThreadOverride();
      setOpenThreadHeadId(null, { replace: true });
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      return;
    }

    if (openThreadHeadMessage && !threadReplyTargetId) {
      setThreadReplyTargetId(openThreadHeadMessage.id);
      return;
    }

    if (threadReplyTargetId && !threadReplyTargetMessage) {
      setThreadReplyTargetId(openThreadHeadMessage?.id ?? null);
    }
    if (editTargetId && !editTargetMessage) {
      setEditTargetId(null);
    }
  }, [
    clearOptimisticThreadOverride,
    editTargetId,
    editTargetMessage,
    isTimelineLoading,
    openThreadHeadId,
    openThreadHeadMessage,
    setOpenThreadHeadId,
    threadReplyTargetId,
    threadReplyTargetMessage,
  ]);

  useLoadMissingAncestors(activeChannel, resolvedMessages);
  const hasAuxiliaryPanel = Boolean(
    effectiveOpenThreadHeadId ||
      openAgentSessionPubkey ||
      profilePanelPubkey ||
      channelManagementOpen,
  );
  const displayedThreadHeadMessage =
    openThreadHeadMessage?.id === effectiveOpenThreadHeadId
      ? openThreadHeadMessage
      : null;
  const displayedThreadMessages = displayedThreadHeadMessage
    ? threadMessages
    : [];
  const displayedThreadReplyTargetMessage = displayedThreadHeadMessage
    ? threadReplyTargetMessage
    : null;
  const displayedThreadFirstUnreadReplyId = displayedThreadHeadMessage
    ? threadFirstUnreadReplyId
    : null;
  const shouldShowThreadSkeleton = Boolean(
    effectiveOpenThreadHeadId && activeChannel && !displayedThreadHeadMessage,
  );
  const isNarrowPanelViewport =
    channelContentWidthPx > 0 &&
    channelContentWidthPx < THREAD_PANEL_SINGLE_COLUMN_BREAKPOINT_PX;
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

  const channelHeader = (
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
      onManageChannel={() => {
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
      }}
      onToggleMembers={() => setIsMembersSidebarOpen((prev) => !prev)}
      showHeaderContent={!isSinglePanelView}
      transparentChrome={activeChannel?.channelType !== "forum"}
    />
  );

  return (
    <AgentSessionProvider onOpenAgentSession={handleOpenAgentSession}>
      <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
        <div
          className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
          ref={channelContentRef}
        >
          {activeChannel ? (
            activeChannel.channelType === "forum" ? (
              <>
                {channelHeader}
                <React.Suspense fallback={<ViewLoadingFallback kind="forum" />}>
                  <ForumView
                    channel={activeChannel}
                    currentPubkey={currentPubkey}
                    onClosePost={onCloseForumPost}
                    onSelectPost={onSelectForumPost}
                    selectedPostId={selectedForumPostId}
                    targetReplyId={targetForumReplyId}
                  />
                </React.Suspense>
              </>
            ) : (
              <React.Suspense
                fallback={<ViewLoadingFallback includeHeader kind="channel" />}
              >
                <ChannelPane
                  activeChannel={activeChannel}
                  activityAgents={channelAgentSessionAgents}
                  agentPubkeys={agentPubkeys}
                  agentSessionAgents={agentSessionAgents}
                  botTypingEntries={botTypingEntries}
                  channelFind={channelFind}
                  channelManagementOpen={channelManagementOpen}
                  currentPubkey={currentPubkey}
                  canResetThreadPanelWidth={canResetThreadPanelWidth}
                  fetchOlder={fetchOlder}
                  header={channelHeader}
                  hasOlderMessages={hasOlderMessages}
                  onAddAgent={() => setIsAddBotOpen(true)}
                  onCreateChannel={openCreateChannel}
                  onOpenMembers={() => setIsMembersSidebarOpen(true)}
                  isFetchingOlder={isFetchingOlder}
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
                  onCancelEdit={handleCancelEdit}
                  onCancelThreadReply={handleCancelThreadReply}
                  onChannelManagementDeleted={() => {
                    setChannelManagementOpen(false);
                    void goHome({ replace: true });
                  }}
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
                  onCloseChannelManagement={() =>
                    setChannelManagementOpen(false)
                  }
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
                  firstUnreadMessageId={firstUnreadMessageId}
                  unreadCount={unreadCount}
                  targetMessageId={mainTimelineTargetMessageId}
                  threadHeadMessage={displayedThreadHeadMessage}
                  threadMessages={displayedThreadMessages}
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
