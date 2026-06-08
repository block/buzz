import * as React from "react";
import { useAppShell } from "@/app/AppShellContext";
import { useActiveChannelHeader } from "@/features/channels/useActiveChannelHeader";
import { useChannelPaneHandlers } from "@/features/channels/useChannelPaneHandlers";
import {
  useChannelMembersQuery,
  useJoinChannelMutation,
} from "@/features/channels/hooks";
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
  formatTimelineMessages,
} from "@/features/messages/lib/formatTimelineMessages";
import { buildThreadPanelData } from "@/features/messages/lib/threadPanel";
import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { useFetchOlderMessages } from "@/features/messages/useFetchOlderMessages";
import { useLoadMissingAncestors } from "@/features/messages/useLoadMissingAncestors";
import { useChannelTyping } from "@/features/messages/useChannelTyping";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import type { RespondToMode } from "@/shared/api/types";
import { useChannelFind } from "@/features/search/useChannelFind";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { AgentSessionProvider } from "@/shared/context/AgentSessionContext";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import {
  useElementWidthBreakpoint,
  useIsThreadPanelOverlay,
} from "@/shared/hooks/use-mobile";
import {
  THREAD_PANEL_MIN_WIDTH_PX,
  THREAD_PANEL_SINGLE_COLUMN_BREAKPOINT_PX,
  useThreadPanelWidth,
} from "@/shared/hooks/useThreadPanelWidth";
import {
  mergeAgentNamesIntoProfiles,
  useChannelActivityTyping,
} from "./useChannelActivityTyping";
import { useChannelAgentSessions } from "./useChannelAgentSessions";
import { useChannelProfilePanel } from "./useChannelProfilePanel";
import { useChannelRouteTarget } from "./useChannelRouteTarget";
import type { ChannelScreenProps } from "./ChannelScreen.types";
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
  const {
    markChannelRead,
    markChannelUnread,
    openChannelManagement,
    followThread,
    unfollowThread,
    isFollowingThread,
    isNotifiedForThread,
  } = useAppShell();
  const [profilePanelPubkey, setProfilePanelPubkey] = React.useState<
    string | null
  >(null);
  const {
    canReset: canResetThreadPanelWidth,
    onResetWidth: handleThreadPanelWidthReset,
    onResizeStart: handleThreadPanelResizeStart,
    widthPx: threadPanelWidthPx,
  } = useThreadPanelWidth();
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  const isThreadPanelOverlay = useIsThreadPanelOverlay();
  const [channelContentRef, isNarrowPanelViewport] =
    useElementWidthBreakpoint<HTMLDivElement>(
      THREAD_PANEL_SINGLE_COLUMN_BREAKPOINT_PX,
    );
  const [openThreadHeadId, setOpenThreadHeadId] = React.useState<string | null>(
    null,
  );
  const isNotifiedForCurrentThread =
    openThreadHeadId != null ? isNotifiedForThread(openThreadHeadId) : false;
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
  const currentPubkey = currentIdentity?.pubkey;
  const activeChannelId = activeChannel?.id ?? null;
  const messagesQuery = useChannelMessagesQuery(activeChannel);
  useChannelSubscription(activeChannel);
  const { fetchOlder, hasOlderMessages, isFetchingOlder } =
    useFetchOlderMessages(activeChannel);
  const latestActiveMessage =
    messagesQuery.data?.[messagesQuery.data.length - 1] ?? null;
  const activeReadAt = latestActiveMessage
    ? new Date(latestActiveMessage.created_at * 1_000).toISOString()
    : (activeChannel?.lastMessageAt ?? null);
  React.useEffect(() => {
    if (!activeChannelId || activeChannel?.isMember === false) {
      return;
    }
    markChannelRead(activeChannelId, activeReadAt);
  }, [activeChannel?.isMember, activeChannelId, activeReadAt, markChannelRead]);
  const {
    activeChannelTitle,
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
  const latestMessageEvent = React.useMemo(
    () => resolvedMessages[resolvedMessages.length - 1] ?? null,
    [resolvedMessages],
  );
  const typingEntries = useChannelTyping(
    activeChannel,
    currentPubkey,
    latestMessageEvent,
  );
  const messageProfilePubkeys = React.useMemo(
    () => [
      ...new Set([
        ...messageAuthorPubkeys,
        ...typingEntries.map((entry) => entry.pubkey),
      ]),
    ],
    [messageAuthorPubkeys, typingEntries],
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
  const {
    botTypingEntries,
    channelAgentSessionAgents: activeChannelAgentSessionAgents,
    humanTypingPubkeys,
    threadTypingPubkeys,
  } = useChannelActivityTyping({
    activeChannel,
    activeChannelId,
    channelMembers,
    managedAgents,
    openThreadHeadId,
    relayAgents,
    typingEntries,
  });
  useManagedAgentObserverBridge(activeChannelAgentSessionAgents);
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
  const directReplyIdsByParentId = React.useMemo(() => {
    const map = new Map<string, string[]>();
    for (const message of timelineMessages) {
      if (!message.parentId) continue;
      const currentReplies = map.get(message.parentId) ?? [];
      currentReplies.push(message.id);
      map.set(message.parentId, currentReplies);
    }
    return map;
  }, [timelineMessages]);
  const getFirstReplyIdForMessage = React.useCallback(
    (messageId: string) => directReplyIdsByParentId.get(messageId)?.[0] ?? null,
    [directReplyIdsByParentId],
  );
  const getReplyDescendantIdsForMessage = React.useCallback(
    (messageId: string) => {
      const descendantIds: string[] = [];
      const pendingIds = [...(directReplyIdsByParentId.get(messageId) ?? [])];
      while (pendingIds.length > 0) {
        const currentId = pendingIds.pop();
        if (!currentId) continue;
        descendantIds.push(currentId);
        pendingIds.push(...(directReplyIdsByParentId.get(currentId) ?? []));
      }
      return descendantIds;
    },
    [directReplyIdsByParentId],
  );
  const threadPanelData = React.useMemo(
    () =>
      buildThreadPanelData(
        timelineMessages,
        openThreadHeadId,
        threadReplyTargetId,
        expandedThreadReplyIds,
      ),
    [
      expandedThreadReplyIds,
      openThreadHeadId,
      threadReplyTargetId,
      timelineMessages,
    ],
  );
  const openThreadHeadMessage = threadPanelData.threadHead;
  const threadMessages = threadPanelData.visibleReplies;
  const threadReplyTargetMessage = threadPanelData.replyTargetMessage;
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
    openThreadHeadId,
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
  const handleMarkUnread = React.useCallback(() => {
    if (!activeChannelId) return;
    markChannelUnread(activeChannelId);
  }, [activeChannelId, markChannelUnread]);
  const {
    channelAgentSessionAgents,
    closeAgentSession: handleCloseAgentSession,
    openAgentSession: handleOpenAgentSession,
    openAgentSessionPubkey,
    openThreadAndCloseAgentSession: handleOpenThreadAndCloseAgentSession,
  } = useChannelAgentSessions({
    activeChannel,
    activeChannelId,
    channelMembers,
    handleOpenThread,
    managedAgents: activeChannelAgentSessionAgents,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  });
  const { handleOpenProfilePanel, handleCloseProfilePanel, handleOpenDm } =
    useChannelProfilePanel({
      closeAgentSession: handleCloseAgentSession,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    });
  const isTimelineLoading =
    activeChannel !== null &&
    activeChannel.channelType !== "forum" &&
    (messagesQuery.isPending ||
      (messagesQuery.isFetching && resolvedMessages.length === 0));
  const resetComposerTargets = React.useCallback(
    (_channelId: string | null) => {
      setOpenThreadHeadId(null);
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      handleCloseAgentSession();
      setEditTargetId(null);
      setProfilePanelPubkey(null);
    },
    [handleCloseAgentSession],
  );
  const handleThreadScrollTargetResolved = React.useCallback(() => {
    setThreadScrollTargetId(null);
  }, []);
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
      setOpenThreadHeadId(null);
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
    editTargetId,
    editTargetMessage,
    openThreadHeadId,
    openThreadHeadMessage,
    threadReplyTargetId,
    threadReplyTargetMessage,
  ]);

  useLoadMissingAncestors(activeChannel, resolvedMessages);
  const isSinglePanelView =
    isNarrowPanelViewport &&
    activeChannel?.channelType !== "forum" &&
    Boolean(
      openThreadHeadMessage || openAgentSessionPubkey || profilePanelPubkey,
    );
  const hasSplitRightPanel =
    !isSinglePanelView &&
    !isThreadPanelOverlay &&
    Boolean(
      openThreadHeadMessage || openAgentSessionPubkey || profilePanelPubkey,
    );
  const headerActionsRightInset = hasSplitRightPanel
    ? `min(${threadPanelWidthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`
    : undefined;

  return (
    <AgentSessionProvider onOpenAgentSession={handleOpenAgentSession}>
      <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
        <ChannelScreenHeader
          activeChannel={activeChannel}
          activeChannelEphemeralDisplay={activeChannelEphemeralDisplay}
          activeChannelTitle={activeChannelTitle}
          actionsRightInset={headerActionsRightInset}
          activeDmPresenceStatus={activeDmPresenceStatus}
          currentPubkey={currentPubkey}
          isJoining={joinChannelMutation.isPending}
          onJoinChannel={joinChannelMutation.mutateAsync}
          onManageChannel={openChannelManagement}
          onToggleMembers={() => setIsMembersSidebarOpen((prev) => !prev)}
          showHeaderContent={!isSinglePanelView}
        />

        <div
          className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
          ref={channelContentRef}
        >
          {activeChannel ? (
            activeChannel.channelType === "forum" ? (
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
            ) : (
              <React.Suspense fallback={<ViewLoadingFallback kind="channel" />}>
                <ChannelPane
                  activeChannel={activeChannel}
                  agentSessionAgents={channelAgentSessionAgents}
                  botTypingEntries={botTypingEntries}
                  channelFind={channelFind}
                  currentPubkey={currentPubkey}
                  canResetThreadPanelWidth={canResetThreadPanelWidth}
                  fetchOlder={fetchOlder}
                  hasOlderMessages={hasOlderMessages}
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
                  isFollowingThread={isNotifiedForCurrentThread}
                  isSending={sendMessageMutation.isPending}
                  isSinglePanelView={isSinglePanelView}
                  isTimelineLoading={isTimelineLoading}
                  messages={timelineMessages}
                  onCancelEdit={handleCancelEdit}
                  onCancelThreadReply={handleCancelThreadReply}
                  onFollowThread={
                    openThreadHeadId != null && !isNotifiedForCurrentThread
                      ? () => followThread(openThreadHeadId)
                      : undefined
                  }
                  onUnfollowThread={
                    openThreadHeadId != null && isNotifiedForCurrentThread
                      ? () => unfollowThread(openThreadHeadId)
                      : undefined
                  }
                  onCloseAgentSession={handleCloseAgentSession}
                  onCloseThread={handleCloseThread}
                  onDelete={
                    activeChannel?.archivedAt ? undefined : handleDelete
                  }
                  onEdit={activeChannel?.archivedAt ? undefined : handleEdit}
                  onEditSave={
                    activeChannel?.archivedAt ? undefined : handleEditSave
                  }
                  onMarkUnread={handleMarkUnread}
                  onExpandThreadReplies={handleExpandThreadReplies}
                  onOpenAgentSession={handleOpenAgentSession}
                  onOpenDm={handleOpenDm}
                  onResetThreadPanelWidth={handleThreadPanelWidthReset}
                  onCloseProfilePanel={handleCloseProfilePanel}
                  onOpenThread={handleOpenThreadAndCloseAgentSession}
                  onSelectThreadReplyTarget={handleSelectThreadReplyTarget}
                  onSendMessage={handleSendMessage}
                  onSendThreadReply={handleSendThreadReply}
                  onThreadScrollTargetResolved={
                    handleThreadScrollTargetResolved
                  }
                  onThreadPanelResizeStart={handleThreadPanelResizeStart}
                  onToggleReaction={effectiveToggleReaction}
                  openAgentSessionPubkey={openAgentSessionPubkey}
                  openThreadHeadId={openThreadHeadId}
                  profilePanelPubkey={profilePanelPubkey}
                  personaLookup={personaLookup}
                  profiles={messageProfiles}
                  targetMessageId={mainTimelineTargetMessageId}
                  threadHeadMessage={openThreadHeadMessage}
                  threadMessages={threadMessages}
                  threadPanelWidthPx={threadPanelWidthPx}
                  threadTypingPubkeys={threadTypingPubkeys}
                  threadReplyTargetMessage={threadReplyTargetMessage}
                  threadScrollTargetId={threadScrollTargetId}
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
