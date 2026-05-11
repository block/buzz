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
import { useFetchOlderMessages } from "@/features/messages/useFetchOlderMessages";
import { useLoadMissingAncestors } from "@/features/messages/useLoadMissingAncestors";
import {
  type TypingIndicatorEntry,
  useChannelTyping,
} from "@/features/messages/useChannelTyping";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import type {
  Channel,
  Identity,
  Profile,
  RelayEvent,
} from "@/shared/api/types";
import { useChannelFind } from "@/features/search/useChannelFind";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { AgentSessionProvider } from "@/shared/context/AgentSessionContext";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useChannelAgentSessions } from "./useChannelAgentSessions";
import { useChannelProfilePanel } from "./useChannelProfilePanel";
type ChannelScreenProps = {
  activeChannel: Channel | null;
  currentIdentity?: Identity;
  currentProfile?: Profile;
  onCloseForumPost: () => void;
  onSelectForumPost: (postId: string) => void;
  selectedForumPostId: string | null;
  targetForumReplyId: string | null;
  targetMessageEvent: RelayEvent | null;
  targetMessageId: string | null;
};

export function ChannelScreen({
  activeChannel,
  currentIdentity,
  currentProfile,
  onCloseForumPost,
  onSelectForumPost,
  selectedForumPostId,
  targetForumReplyId,
  targetMessageEvent,
  targetMessageId,
}: ChannelScreenProps) {
  const { markChannelRead, openChannelManagement } = useAppShell();
  const [profilePanelPubkey, setProfilePanelPubkey] = React.useState<
    string | null
  >(null);
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  const [openThreadHeadId, setOpenThreadHeadId] = React.useState<string | null>(
    null,
  );
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
    if (!activeChannel || !targetMessageEvent) return currentMessages;
    return mergeMessages(currentMessages, targetMessageEvent);
  }, [activeChannel, messagesQuery.data, targetMessageEvent]);
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
  const threadTypingPubkeys = React.useMemo(
    () =>
      typingEntries
        .filter((entry) => entry.threadHeadId === openThreadHeadId)
        .map((entry) => entry.pubkey),
    [openThreadHeadId, typingEntries],
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
  const managedAgentsQuery = useManagedAgentsQuery();
  useManagedAgentObserverBridge(managedAgentsQuery.data ?? []);
  const { botTypingEntries, humanTypingPubkeys } = React.useMemo<{
    botTypingEntries: TypingIndicatorEntry[];
    humanTypingPubkeys: string[];
  }>(() => {
    const localAgentSet = new Set(
      (managedAgentsQuery.data ?? [])
        .filter((agent) => agent.backend.type === "local")
        .map((agent) => agent.pubkey.toLowerCase()),
    );
    const channelTypingEntries = typingEntries.filter(
      (entry) => entry.threadHeadId === null,
    );
    const agentTypingEntries = typingEntries.filter((entry) =>
      localAgentSet.has(entry.pubkey.toLowerCase()),
    );
    return {
      botTypingEntries: agentTypingEntries,
      humanTypingPubkeys: channelTypingEntries
        .filter((entry) => !localAgentSet.has(entry.pubkey.toLowerCase()))
        .map((entry) => entry.pubkey),
    };
  }, [managedAgentsQuery.data, typingEntries]);
  const messageProfiles = React.useMemo(() => {
    const base =
      mergeCurrentProfileIntoLookup(
        messageProfilesQuery.data?.profiles,
        currentProfile,
      ) ?? {};
    // Merge managed agent names so system messages resolve instantly
    // (without waiting for the relay profile batch query).
    const agents = managedAgentsQuery.data ?? [];
    const merged = { ...base };
    for (const agent of agents) {
      const key = agent.pubkey.toLowerCase();
      if (!merged[key]?.displayName) {
        merged[key] = {
          ...merged[key],
          displayName: agent.name,
          avatarUrl: null,
          nip05Handle: null,
        };
      }
    }
    return merged;
  }, [
    currentProfile,
    managedAgentsQuery.data,
    messageProfilesQuery.data?.profiles,
  ]);
  const channelMembersQuery = useChannelMembersQuery(activeChannel?.id ?? null);
  const channelMembers = channelMembersQuery.data;
  const personasQuery = usePersonasQuery();
  const personaLookup = React.useMemo(() => {
    const agents = managedAgentsQuery.data ?? [];
    const personas = personasQuery.data ?? [];
    const personaById = new Map(personas.map((p) => [p.id, p.displayName]));
    const lookup = new Map<string, string>();
    for (const agent of agents) {
      if (agent.personaId) {
        const personaName = personaById.get(agent.personaId);
        if (personaName) {
          lookup.set(agent.pubkey.toLowerCase(), personaName);
        }
      }
    }
    return lookup;
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
      ),
    [
      activeChannel,
      channelMembers,
      currentProfile?.avatarUrl,
      currentPubkey,
      messageProfiles,
      personaLookup,
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
    managedAgents: managedAgentsQuery.data ?? [],
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    targetMessageId,
    timelineMessages,
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

  return (
    <AgentSessionProvider onOpenAgentSession={handleOpenAgentSession}>
      <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
        <ChannelScreenHeader
          activeChannel={activeChannel}
          activeChannelEphemeralDisplay={activeChannelEphemeralDisplay}
          activeChannelTitle={activeChannelTitle}
          activeDmPresenceStatus={activeDmPresenceStatus}
          currentPubkey={currentPubkey}
          isJoining={joinChannelMutation.isPending}
          onJoinChannel={joinChannelMutation.mutateAsync}
          onManageChannel={openChannelManagement}
          onToggleMembers={() => setIsMembersSidebarOpen((prev) => !prev)}
        />

        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
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
                  fetchOlder={fetchOlder}
                  hasOlderMessages={hasOlderMessages}
                  isFetchingOlder={isFetchingOlder}
                  editTarget={
                    editTargetMessage
                      ? {
                          author: editTargetMessage.author,
                          body: editTargetMessage.body,
                          id: editTargetMessage.id,
                        }
                      : null
                  }
                  isSending={sendMessageMutation.isPending}
                  isTimelineLoading={isTimelineLoading}
                  messages={timelineMessages}
                  onCancelEdit={handleCancelEdit}
                  onCancelThreadReply={handleCancelThreadReply}
                  onCloseAgentSession={handleCloseAgentSession}
                  onCloseThread={handleCloseThread}
                  onDelete={
                    activeChannel?.archivedAt ? undefined : handleDelete
                  }
                  onEdit={activeChannel?.archivedAt ? undefined : handleEdit}
                  onEditSave={
                    activeChannel?.archivedAt ? undefined : handleEditSave
                  }
                  onExpandThreadReplies={handleExpandThreadReplies}
                  onOpenAgentSession={handleOpenAgentSession}
                  onOpenDm={handleOpenDm}
                  onCloseProfilePanel={handleCloseProfilePanel}
                  onOpenThread={handleOpenThreadAndCloseAgentSession}
                  onSelectThreadReplyTarget={handleSelectThreadReplyTarget}
                  onSendMessage={handleSendMessage}
                  onSendThreadReply={handleSendThreadReply}
                  onThreadScrollTargetResolved={
                    handleThreadScrollTargetResolved
                  }
                  onToggleReaction={effectiveToggleReaction}
                  openAgentSessionPubkey={openAgentSessionPubkey}
                  openThreadHeadId={openThreadHeadId}
                  profilePanelPubkey={profilePanelPubkey}
                  personaLookup={personaLookup}
                  profiles={messageProfiles}
                  targetMessageId={targetMessageId}
                  threadHeadMessage={openThreadHeadMessage}
                  threadMessages={threadMessages}
                  threadTypingPubkeys={threadTypingPubkeys}
                  threadReplyTargetId={threadReplyTargetId}
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
