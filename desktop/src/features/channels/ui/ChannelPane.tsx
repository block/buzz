import * as React from "react";
import { Bot, Hash, LogIn, Plus, Sparkles, UserPlus } from "lucide-react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { DropZoneOverlay } from "@/features/messages/ui/ComposerAttachments";
import {
  MessageThreadPanel,
  MessageThreadPanelSkeleton,
} from "@/features/messages/ui/MessageThreadPanel";
import {
  MessageTimeline,
  type MessageTimelineHandle,
} from "@/features/messages/ui/MessageTimeline";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import {
  getDmHuddleMemberPubkeys,
  hasOtherDmParticipant,
} from "@/features/channels/lib/dmHuddleMembers";
import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewContextForMessage,
} from "@/features/messages/lib/videoReviewContext";
import { useComposerHeightPadding } from "@/features/messages/ui/useComposerHeightPadding";
import { TypingIndicatorRow } from "@/features/messages/ui/TypingIndicatorRow";
import { UserProfilePanel } from "@/features/profile/ui/UserProfilePanel";
import { ChannelFindBar } from "@/features/search/ui/ChannelFindBar";
import { AgentSessionThreadPanel } from "@/features/channels/ui/AgentSessionThreadPanel";
import { ChannelManagementAuxiliaryPanel } from "@/features/channels/ui/ChannelManagementAuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { useChannelWorkingAgentPubkeys } from "@/features/agents/agentWorkingSignal";
import { BotActivityComposerAction } from "@/features/channels/ui/BotActivityBar";
import {
  containsWelcomePersonaMention,
  WelcomeComposerBanner,
  WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS,
  WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS,
  WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS,
  WELCOME_PERSONA_ROTATION_MS,
  type WelcomeComposerBannerState,
} from "@/features/channels/ui/WelcomeComposerBanner";
import {
  getChannelIntroDescription,
  getChannelIntroKind,
  isWelcomeSetupSystemMessage,
  mentionsKnownAgent,
} from "@/features/channels/ui/ChannelPane.helpers";
import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";
import * as agentSessionSelection from "@/features/channels/ui/agentSessionSelection";
import { Button } from "@/shared/ui/button";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import { useRenderScopedReactionHydration } from "@/features/messages/lib/useRenderScopedReactionHydration";
import type { TimelineMessage } from "@/features/messages/types";
import { isWelcomeChannel } from "@/features/onboarding/welcome";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
export const ChannelPane = React.memo(function ChannelPane({
  activeChannel,
  agentPubkeys,
  agentPubkeysPending = false,
  agentSessionAgents,
  activityAgents = agentSessionAgents,
  autoSendDraftKey = null,
  onAutoSendComplete = null,
  botTypingEntries,
  channelFind,
  channelManagementOpen = false,
  currentPubkey,
  editTarget = null,
  fetchOlder,
  header,
  hasOlderMessages,
  isFetchingOlder,
  followThreadById,
  isFollowingThread,
  isFollowingThreadById,
  isMessageUnreadById,
  isJoining = false,
  isSinglePanelView = false,
  isSending,
  isTimelineLoading,
  messages,
  threadSummaries,
  firstUnreadMessageId = null,
  unreadCount = 0,
  canResetThreadPanelWidth,
  onCancelEdit,
  onCancelThreadReply,
  onBackFromAgentSession,
  onCloseAgentSession,
  onCloseChannelManagement,
  onChannelManagementDeleted,
  onCloseProfilePanel,
  onAddAgent,
  onCreateChannel,
  onCloseThread,
  onDelete,
  onEdit,
  onEditSave,
  onFollowThread,
  onMarkUnread,
  onMarkRead,
  onExpandThreadReplies,
  onJoinChannel,
  onOpenAgentSession,
  onOpenDm,
  onOpenMembers,
  onOpenProfilePanel,
  onOpenThread,
  onResetThreadPanelWidth,
  onSelectThreadReplyTarget,
  onSendMessage,
  onSendVideoReviewComment,
  onSendThreadReply,
  onThreadScrollTargetResolved,
  onThreadPanelResizeStart,
  onTargetReached,
  onToggleReaction,
  onUnfollowThread,
  unfollowThreadById,
  personaLookup,
  profiles,
  openThreadHeadId,
  shouldShowThreadSkeleton,
  openAgentSessionChannelId,
  openAgentSessionPubkey,
  onProfilePanelViewChange,
  onProfilePanelTabChange,
  profilePanelPubkey,
  profilePanelTab,
  profilePanelView,
  targetMessageId,
  threadHeadMessage,
  threadMessages,
  threadPanelWidthPx,
  threadScrollTargetId,
  threadTypingPubkeys,
  threadReplyTargetMessage,
  threadUnreadCounts,
  threadReplyUnreadCounts,
  threadFirstUnreadReplyId,
  typingPubkeys,
}: ChannelPaneProps) {
  const timelineScrollRef = React.useRef<HTMLDivElement>(null);
  const messageTimelineRef = React.useRef<MessageTimelineHandle>(null);
  const composerWrapperRef = React.useRef<HTMLDivElement>(null);
  const completedWelcomeBannerChannelIdsRef = React.useRef(new Set<string>());
  const welcomeComposerDismissTimerRef = React.useRef<number | null>(null);
  const welcomeComposerHideTimerRef = React.useRef<number | null>(null);
  const [welcomeComposerBannerState, setWelcomeComposerBannerState] =
    React.useState<WelcomeComposerBannerState>("prompt");
  const { goChannel } = useAppNavigation();
  const mainComposerMedia = useMediaUpload();
  const isNonMemberView =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt;
  const hasMainComposerOverlay = !isNonMemberView;
  const activeChannelId = activeChannel?.id ?? null;
  // Clear the ?autoSend search param once the auto-submit fires so
  // back-navigation cannot re-trigger the send.
  // When `onAutoSendComplete` is provided it does a surgical single-key clear
  // that preserves `?thread` and all other panel search state (required for
  // the thread-draft send path so the thread panel does not unmount before the
  // deferred setTimeout(0) submit fires). The goChannel fallback is kept for
  // callers that do not supply the prop (e.g. isolated tests / older wrappers).
  const handleAutoSubmitComplete = React.useCallback(() => {
    if (onAutoSendComplete) {
      onAutoSendComplete();
    } else if (activeChannelId) {
      void goChannel(activeChannelId, { replace: true });
    }
  }, [activeChannelId, goChannel, onAutoSendComplete]);
  const huddleMemberPubkeys = React.useMemo(
    () => getDmHuddleMemberPubkeys(activeChannel, agentPubkeys, currentPubkey),
    [activeChannel, agentPubkeys, currentPubkey],
  );
  const huddleMemberPubkeysPending =
    agentPubkeysPending && hasOtherDmParticipant(activeChannel, currentPubkey);
  const isActiveWelcomeChannel =
    activeChannel !== null && isWelcomeChannel(activeChannel);
  useComposerHeightPadding(
    timelineScrollRef,
    composerWrapperRef,
    `${activeChannelId}:${isSinglePanelView}:${hasMainComposerOverlay}`,
  );
  const clearWelcomeComposerDismissTimer = React.useCallback(() => {
    if (welcomeComposerDismissTimerRef.current !== null) {
      window.clearTimeout(welcomeComposerDismissTimerRef.current);
      welcomeComposerDismissTimerRef.current = null;
    }
    if (welcomeComposerHideTimerRef.current !== null) {
      window.clearTimeout(welcomeComposerHideTimerRef.current);
      welcomeComposerHideTimerRef.current = null;
    }
  }, []);

  React.useEffect(
    () => () => clearWelcomeComposerDismissTimer(),
    [clearWelcomeComposerDismissTimer],
  );

  React.useEffect(() => {
    clearWelcomeComposerDismissTimer();

    if (
      activeChannelId &&
      isActiveWelcomeChannel &&
      completedWelcomeBannerChannelIdsRef.current.has(activeChannelId)
    ) {
      setWelcomeComposerBannerState("hidden");
      return;
    }

    setWelcomeComposerBannerState("prompt");
  }, [
    activeChannelId,
    clearWelcomeComposerDismissTimer,
    isActiveWelcomeChannel,
  ]);

  const isEditInThread =
    editTarget != null &&
    threadHeadMessage != null &&
    (editTarget.id === threadHeadMessage.id ||
      threadMessages.some((entry) => entry.message.id === editTarget.id));
  const mainEditTarget = editTarget && !isEditInThread ? editTarget : null;
  const threadEditTarget = editTarget && isEditInThread ? editTarget : null;

  const findLastOwnEditable = React.useCallback(
    (candidates: TimelineMessage[]): TimelineMessage | null => {
      if (!onEdit || !currentPubkey) return null;
      let best: TimelineMessage | null = null;
      for (const message of candidates) {
        if (
          message.kind === KIND_SYSTEM_MESSAGE ||
          message.pubkey !== currentPubkey ||
          message.pending
        ) {
          continue;
        }
        if (!best || message.createdAt >= best.createdAt) {
          best = message;
        }
      }
      return best;
    },
    [onEdit, currentPubkey],
  );

  const handleEditLastOwnMainMessage = React.useCallback((): boolean => {
    const target = findLastOwnEditable(messages);
    if (!target || !onEdit) return false;
    onEdit(target);
    return true;
  }, [findLastOwnEditable, messages, onEdit]);

  const handleEditLastOwnThreadMessage = React.useCallback((): boolean => {
    if (!onEdit) return false;
    const scope: TimelineMessage[] = [];
    if (threadHeadMessage) scope.push(threadHeadMessage);
    for (const entry of threadMessages) scope.push(entry.message);
    const target = findLastOwnEditable(scope);
    if (!target) return false;
    onEdit(target);
    return true;
  }, [findLastOwnEditable, onEdit, threadHeadMessage, threadMessages]);

  const isComposerDisabled =
    !activeChannel?.isMember ||
    activeChannel.archivedAt !== null ||
    activeChannel.channelType === "forum" ||
    isSending;
  const knownAgentPubkeys = React.useMemo(() => {
    const pubkeys = new Set<string>();

    for (const pubkey of agentPubkeys ?? []) {
      pubkeys.add(pubkey.toLowerCase());
    }
    for (const agent of agentSessionAgents) {
      pubkeys.add(agent.pubkey.toLowerCase());
    }
    for (const agent of activityAgents) {
      pubkeys.add(agent.pubkey.toLowerCase());
    }

    return pubkeys;
  }, [activityAgents, agentPubkeys, agentSessionAgents]);
  const completeWelcomeComposerBanner = React.useCallback(() => {
    if (!activeChannelId || !isActiveWelcomeChannel) {
      return;
    }

    clearWelcomeComposerDismissTimer();
    completedWelcomeBannerChannelIdsRef.current.add(activeChannelId);
    setWelcomeComposerBannerState("complete");
    welcomeComposerDismissTimerRef.current = window.setTimeout(() => {
      setWelcomeComposerBannerState("dismissing");
      welcomeComposerDismissTimerRef.current = null;
      welcomeComposerHideTimerRef.current = window.setTimeout(
        () => {
          setWelcomeComposerBannerState("hidden");
          welcomeComposerHideTimerRef.current = null;
        },
        WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS * 1000 +
          WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS,
      );
    }, WELCOME_PERSONA_ROTATION_MS + WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS);
  }, [
    activeChannelId,
    clearWelcomeComposerDismissTimer,
    isActiveWelcomeChannel,
  ]);
  const handleSendMessage = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      channelId?: string | null,
    ) => {
      const shouldCompleteWelcomeBanner =
        isActiveWelcomeChannel &&
        (containsWelcomePersonaMention(content) ||
          mentionsKnownAgent(mentionPubkeys, knownAgentPubkeys));

      messageTimelineRef.current?.scrollToBottomOnNextUpdate();
      await onSendMessage(content, mentionPubkeys, mediaTags, channelId);

      if (shouldCompleteWelcomeBanner) {
        completeWelcomeComposerBanner();
      }
    },
    [
      completeWelcomeComposerBanner,
      isActiveWelcomeChannel,
      knownAgentPubkeys,
      onSendMessage,
    ],
  );
  const canDropInMainColumn =
    hasMainComposerOverlay && !isComposerDisabled && !isSinglePanelView;
  const hasTypingActivity = typingPubkeys.length > 0;
  // Unified working set for the composer bar: observer-derived turns primary,
  // bot typing fallback (both folded together by agentWorkingSignal). This is
  // what makes the bar show for an agent whose observer stream is live but
  // whose typing signal never arrives — and vice versa.
  const composerWorkingBotPubkeys = useChannelWorkingAgentPubkeys(
    activeChannel?.id ?? null,
  );
  const hasComposerBotActivity = composerWorkingBotPubkeys.length > 0;
  const threadComposerBotTypingPubkeys = React.useMemo(() => {
    if (!openThreadHeadId) {
      return [];
    }

    const pubkeys: string[] = [];
    for (const entry of botTypingEntries) {
      if (entry.threadHeadId !== openThreadHeadId) {
        continue;
      }

      if (
        !pubkeys.some(
          (pubkey) => pubkey.toLowerCase() === entry.pubkey.toLowerCase(),
        )
      ) {
        pubkeys.push(entry.pubkey);
      }
    }
    return pubkeys;
  }, [botTypingEntries, openThreadHeadId]);
  const hasThreadComposerBotActivity =
    threadComposerBotTypingPubkeys.length > 0;
  const directMessageIntro = React.useMemo(
    () =>
      buildDirectMessageIntro({
        channel: activeChannel,
        currentPubkey,
        profiles,
      }),
    [activeChannel, currentPubkey, profiles],
  );

  const channelIntro = React.useMemo(() => {
    if (!activeChannel || activeChannel.channelType === "dm") {
      return null;
    }

    const actions = [];
    if (isWelcomeChannel(activeChannel)) {
      if (onCreateChannel) {
        actions.push({
          icon: <Plus aria-hidden className="h-6 w-6" />,
          label: "Create a channel",
          onClick: onCreateChannel,
          testId: "welcome-intro-action-create-channel",
        });
      }

      if (onAddAgent) {
        actions.push({
          icon: <Bot aria-hidden className="h-6 w-6" />,
          label: "Create a custom agent",
          onClick: onAddAgent,
          testId: "welcome-intro-action-create-agent",
        });
      }

      return {
        actions,
        channelKindLabel: "private welcome channel",
        channelName: activeChannel.name,
        description: null,
        icon: <Sparkles aria-hidden className="h-7 w-7" />,
      };
    }

    if (!activeChannel.archivedAt && activeChannel.isMember) {
      if (onAddAgent) {
        actions.push({
          description: "Add an agent here.",
          icon: <Bot aria-hidden className="h-6 w-6" />,
          label: "Create agent",
          onClick: onAddAgent,
          testId: "channel-intro-action-create-agent",
        });
      }

      if (onOpenMembers) {
        actions.push({
          description: "Invite members.",
          icon: <UserPlus aria-hidden className="h-6 w-6" />,
          label: "Add people",
          onClick: onOpenMembers,
          testId: "channel-intro-action-add-people",
        });
      }
    }

    return {
      actions,
      channelKindLabel: getChannelIntroKind(activeChannel),
      channelName: activeChannel.name,
      description: getChannelIntroDescription(activeChannel),
    };
  }, [activeChannel, onAddAgent, onCreateChannel, onOpenMembers]);

  const visibleMessages = React.useMemo(() => {
    if (!isWelcomeChannel(activeChannel)) {
      return messages;
    }

    return messages.filter((message) => !isWelcomeSetupSystemMessage(message));
  }, [activeChannel, messages]);
  const mainTimelineEntries = React.useMemo(
    () =>
      buildMainTimelineEntries(
        visibleMessages,
        new Set(),
        threadSummaries,
        profiles,
      ),
    [profiles, threadSummaries, visibleMessages],
  );
  useRenderScopedReactionHydration({
    activeChannel,
    mainTimelineEntries,
    threadHeadMessage,
    threadMessages,
  });
  const videoReviewCommentsByRootId = React.useMemo(
    () => buildVideoReviewCommentsByRootId(messages),
    [messages],
  );
  const activeVideoReviewCommentSender = activeChannel?.archivedAt
    ? undefined
    : onSendVideoReviewComment;
  const threadHeadVideoReviewContext = React.useMemo(() => {
    if (!threadHeadMessage) {
      return undefined;
    }

    return buildVideoReviewContextForMessage({
      channelId: activeChannel?.id ?? null,
      channelName: activeChannel?.name,
      channelType: activeChannel?.channelType ?? null,
      comments: videoReviewCommentsByRootId.get(threadHeadMessage.id) ?? [],
      isSendingVideoReviewComment: isSending,
      message: threadHeadMessage,
      onSendVideoReviewComment: activeVideoReviewCommentSender,
      onToggleReaction,
      profiles,
    });
  }, [
    activeChannel,
    activeVideoReviewCommentSender,
    isSending,
    onToggleReaction,
    profiles,
    threadHeadMessage,
    videoReviewCommentsByRootId,
  ]);

  const isOverlay = useIsThreadPanelOverlay();
  const useSplitAuxiliaryPane = !isSinglePanelView && !isOverlay;
  const selectedAgent = React.useMemo(
    () =>
      agentSessionSelection.resolveSelectedAgentSession({
        agentSessionAgents,
        openAgentSessionPubkey,
        profilePanelPubkey,
        profiles,
      }),
    [agentSessionAgents, openAgentSessionPubkey, profilePanelPubkey, profiles],
  );
  const hasSplitAuxiliaryPane =
    useSplitAuxiliaryPane &&
    (channelManagementOpen ||
      Boolean(threadHeadMessage) ||
      shouldShowThreadSkeleton ||
      Boolean(activeChannel && selectedAgent) ||
      Boolean(profilePanelPubkey));
  const wrapAux = (panel: React.ReactNode, testId: string) =>
    useSplitAuxiliaryPane ? (
      <RightAuxiliaryPane
        canResetWidth={canResetThreadPanelWidth}
        onResetWidth={onResetThreadPanelWidth}
        onResizeStart={onThreadPanelResizeStart}
        testId={testId}
        widthPx={threadPanelWidthPx}
      >
        {panel}
      </RightAuxiliaryPane>
    ) : (
      panel
    );
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
      {!isSinglePanelView ? (
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute inset-x-0 top-0 z-30 bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
            channelChrome.headerHeight,
          )}
          data-testid="channel-shared-header-backdrop"
        />
      ) : null}

      {!isSinglePanelView ? (
        <section
          aria-label="Channel messages and composer"
          className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
          data-testid="channel-drop-zone"
          onDragEnter={
            canDropInMainColumn ? mainComposerMedia.handleDragEnter : undefined
          }
          onDragLeave={
            canDropInMainColumn ? mainComposerMedia.handleDragLeave : undefined
          }
          onDragOver={
            canDropInMainColumn ? mainComposerMedia.handleDragOver : undefined
          }
          onDrop={
            canDropInMainColumn
              ? (event) => {
                  void mainComposerMedia.handleDrop(event);
                }
              : undefined
          }
        >
          {header}
          {channelFind.isOpen ? (
            <div className={cn("absolute inset-x-0 z-40", channelChrome.top)}>
              <ChannelFindBar
                matchCount={channelFind.matchCount}
                matchIndex={channelFind.activeIndex}
                onClose={channelFind.close}
                onNext={channelFind.goToNext}
                onPrevious={channelFind.goToPrevious}
                onQueryChange={channelFind.setQuery}
                query={channelFind.query}
              />
            </div>
          ) : null}
          <MessageTimeline
            ref={messageTimelineRef}
            agentPubkeys={agentPubkeys}
            channelId={activeChannel?.id}
            channelIntro={channelIntro}
            directMessageIntro={directMessageIntro}
            scrollContainerRef={timelineScrollRef}
            currentPubkey={currentPubkey}
            fetchOlder={fetchOlder}
            followThreadById={followThreadById}
            hasComposerOverlay={hasMainComposerOverlay}
            hasOlderMessages={hasOlderMessages}
            huddleMemberPubkeys={huddleMemberPubkeys}
            huddleMemberPubkeysPending={huddleMemberPubkeysPending}
            isFetchingOlder={isFetchingOlder}
            isFollowingThreadById={isFollowingThreadById}
            isMessageUnreadById={isMessageUnreadById}
            personaLookup={personaLookup}
            profiles={profiles}
            unfollowThreadById={unfollowThreadById}
            emptyDescription={
              activeChannel?.channelType === "forum"
                ? "Select a stream or DM to load real message history in this first integration pass."
                : "Messages and sub-replies will appear here once the relay has history for this channel."
            }
            emptyTitle={
              activeChannel
                ? activeChannel.channelType === "forum"
                  ? "Forum channels are next"
                  : "No messages yet"
                : "No channel selected"
            }
            isLoading={isTimelineLoading}
            mainEntries={mainTimelineEntries}
            threadSummaries={threadSummaries}
            messages={visibleMessages}
            firstUnreadMessageId={firstUnreadMessageId}
            unreadCount={unreadCount}
            onDelete={onDelete}
            onEdit={onEdit}
            onMarkUnread={onMarkUnread}
            onMarkRead={onMarkRead}
            onReply={activeChannel?.archivedAt ? undefined : onOpenThread}
            channelName={activeChannel?.name}
            channelType={activeChannel?.channelType ?? null}
            isSendingVideoReviewComment={isSending}
            onSendVideoReviewComment={
              activeChannel?.archivedAt ? undefined : onSendVideoReviewComment
            }
            onTargetReached={onTargetReached}
            onToggleReaction={onToggleReaction}
            searchActiveMessageId={channelFind.activeMatch?.messageId ?? null}
            searchMatchingMessageIds={channelFind.matchingMessageIds}
            searchQuery={channelFind.query}
            targetMessageId={targetMessageId}
            threadUnreadCounts={threadUnreadCounts}
          />
          {isNonMemberView ? (
            <div
              data-testid="join-banner"
              className="flex items-center gap-3 border-t border-border/80 bg-card/50 px-5 py-3"
            >
              <div className="flex min-w-0 flex-1 items-center gap-2 text-sm text-muted-foreground">
                <Hash className="h-4 w-4 shrink-0" />
                <span className="truncate">
                  Viewing{" "}
                  <span className="font-medium text-foreground">
                    #{activeChannel?.name}
                  </span>
                </span>
              </div>
              <Button
                disabled={isJoining}
                onClick={() => {
                  void onJoinChannel?.();
                }}
                size="sm"
                variant="default"
              >
                <LogIn className="mr-1.5 h-4 w-4" />
                {isJoining ? "Joining..." : "Join to participate"}
              </Button>
            </div>
          ) : (
            <div
              className="pointer-events-none absolute inset-x-0 bottom-0 z-40"
              data-testid="channel-composer-overlay"
              ref={composerWrapperRef}
            >
              <div className="pointer-events-auto">
                {isActiveWelcomeChannel ? (
                  <WelcomeComposerBanner state={welcomeComposerBannerState} />
                ) : null}
                <MessageComposer
                  channelId={activeChannel?.id ?? null}
                  channelName={activeChannel?.name ?? "channel"}
                  channelType={activeChannel?.channelType ?? null}
                  containerClassName="px-5"
                  disabled={isComposerDisabled}
                  editTarget={mainEditTarget}
                  autoSubmitDraftKey={autoSendDraftKey}
                  onAutoSubmitComplete={handleAutoSubmitComplete}
                  isSending={isSending}
                  mediaController={mainComposerMedia}
                  onCancelEdit={onCancelEdit}
                  onEditLastOwnMessage={handleEditLastOwnMainMessage}
                  onEditSave={onEditSave}
                  onSend={handleSendMessage}
                  profiles={profiles}
                  placeholder={
                    activeChannel?.archivedAt
                      ? "Archived channels are read-only."
                      : activeChannel?.channelType === "forum"
                        ? "Forum posting is not wired in this pass."
                        : activeChannel
                          ? activeChannel.channelType === "dm" &&
                            directMessageIntro
                            ? `Message ${directMessageIntro.displayName}`
                            : `Message #${activeChannel.name}`
                          : "Select a channel"
                  }
                  showTopBorder={false}
                />
                <div className="min-h-8 overflow-visible bg-background px-5 pb-1.5 pt-0">
                  <div className="flex h-full w-full items-center gap-2 overflow-visible">
                    {hasComposerBotActivity ? (
                      <div className="flex min-w-0 flex-1 overflow-visible">
                        <BotActivityComposerAction
                          agents={activityAgents}
                          channelId={activeChannel?.id ?? null}
                          onOpenAgentSession={onOpenAgentSession}
                          openAgentSessionPubkey={openAgentSessionPubkey}
                          profiles={profiles}
                          workingBotPubkeys={composerWorkingBotPubkeys}
                          variant="inline"
                        />
                      </div>
                    ) : null}
                    {hasTypingActivity ? (
                      <TypingIndicatorRow
                        channel={activeChannel}
                        className="min-w-0 flex-1 py-0 pl-[calc(0.75rem+1px)] pr-0 sm:pl-[calc(1rem+1px)]"
                        currentPubkey={currentPubkey}
                        profiles={profiles}
                        typingPubkeys={typingPubkeys}
                      />
                    ) : null}
                  </div>
                </div>
              </div>
            </div>
          )}
          {canDropInMainColumn && mainComposerMedia.isDragOver ? (
            <DropZoneOverlay className="z-30 rounded-none" />
          ) : null}
        </section>
      ) : null}

      {channelManagementOpen && activeChannel ? (
        <ChannelManagementAuxiliaryPanel
          activeChannel={activeChannel}
          canResetThreadPanelWidth={canResetThreadPanelWidth}
          currentPubkey={currentPubkey}
          isSinglePanelView={isSinglePanelView}
          onChannelManagementDeleted={onChannelManagementDeleted}
          onCloseChannelManagement={onCloseChannelManagement}
          onResetThreadPanelWidth={onResetThreadPanelWidth}
          onThreadPanelResizeStart={onThreadPanelResizeStart}
          threadPanelWidthPx={threadPanelWidthPx}
          useSplitAuxiliaryPane={useSplitAuxiliaryPane}
          transparentChrome={hasSplitAuxiliaryPane}
        />
      ) : threadHeadMessage ? (
        (() => {
          const panel = (
            <MessageThreadPanel
              agentPubkeys={agentPubkeys}
              channel={activeChannel}
              channelId={activeChannel?.id ?? null}
              channelName={activeChannel?.name ?? "channel"}
              currentPubkey={currentPubkey}
              disabled={isComposerDisabled}
              editTarget={threadEditTarget}
              firstUnreadReplyId={threadFirstUnreadReplyId}
              huddleMemberPubkeys={huddleMemberPubkeys}
              huddleMemberPubkeysPending={huddleMemberPubkeysPending}
              isFollowingThread={isFollowingThread}
              isMessageUnreadById={isMessageUnreadById}
              isSending={isSending}
              isSinglePanelView={
                useSplitAuxiliaryPane ? false : isSinglePanelView
              }
              layout={useSplitAuxiliaryPane ? "split" : "standalone"}
              transparentChrome={useSplitAuxiliaryPane}
              autoSendDraftKey={autoSendDraftKey}
              onAutoSubmitComplete={handleAutoSubmitComplete}
              onCancelEdit={onCancelEdit}
              onCancelReply={onCancelThreadReply}
              onClose={onCloseThread}
              onDelete={onDelete}
              onEdit={onEdit}
              onEditLastOwnMessage={handleEditLastOwnThreadMessage}
              onEditSave={onEditSave}
              onFollowThread={onFollowThread}
              onMarkUnread={onMarkUnread}
              onMarkRead={onMarkRead}
              onExpandReplies={onExpandThreadReplies}
              onSelectReplyTarget={onSelectThreadReplyTarget}
              onSend={onSendThreadReply}
              onScrollTargetResolved={onThreadScrollTargetResolved}
              onToggleReaction={onToggleReaction}
              onUnfollowThread={onUnfollowThread}
              profiles={profiles}
              replyTargetMessage={threadReplyTargetMessage}
              scrollTargetId={threadScrollTargetId}
              threadHead={threadHeadMessage}
              threadHeadVideoReviewContext={threadHeadVideoReviewContext}
              widthPx={threadPanelWidthPx}
              threadReplies={threadMessages}
              threadUnreadCount={threadUnreadCounts?.get(threadHeadMessage.id)}
              threadReplyUnreadCounts={threadReplyUnreadCounts}
              threadTypingPubkeys={threadTypingPubkeys}
              toolbarExtraActions={
                hasThreadComposerBotActivity ? (
                  <BotActivityComposerAction
                    agents={activityAgents}
                    channelId={activeChannel?.id ?? null}
                    onOpenAgentSession={onOpenAgentSession}
                    openAgentSessionPubkey={openAgentSessionPubkey}
                    profiles={profiles}
                    workingBotPubkeys={threadComposerBotTypingPubkeys}
                    variant="inline"
                  />
                ) : null
              }
            />
          );
          return wrapAux(panel, "message-thread-panel");
        })()
      ) : shouldShowThreadSkeleton ? (
        (() => {
          const panel = (
            <MessageThreadPanelSkeleton
              isSinglePanelView={
                useSplitAuxiliaryPane ? false : isSinglePanelView
              }
              layout={useSplitAuxiliaryPane ? "split" : "standalone"}
              transparentChrome={useSplitAuxiliaryPane}
              onClose={onCloseThread}
              widthPx={threadPanelWidthPx}
            />
          );
          return wrapAux(panel, "message-thread-panel");
        })()
      ) : activeChannel && selectedAgent ? (
        (() => {
          const panel = (
            <AgentSessionThreadPanel
              agent={selectedAgent}
              canInterruptTurn={selectedAgent.canInterruptTurn}
              channel={
                openAgentSessionChannelId
                  ? activeChannel?.id === openAgentSessionChannelId
                    ? activeChannel
                    : null
                  : agentSessionSelection.isAgentInActivityList({
                        activityAgents,
                        selectedAgent,
                      })
                    ? activeChannel
                    : null
              }
              channelId={openAgentSessionChannelId}
              isSinglePanelView={
                useSplitAuxiliaryPane ? false : isSinglePanelView
              }
              layout={useSplitAuxiliaryPane ? "split" : "standalone"}
              transparentChrome={useSplitAuxiliaryPane}
              profiles={profiles}
              onBack={onBackFromAgentSession}
              onClose={onCloseAgentSession}
              widthPx={threadPanelWidthPx}
            />
          );
          return wrapAux(panel, "agent-session-thread-panel");
        })()
      ) : profilePanelPubkey ? (
        (() => {
          const panel = (
            <UserProfilePanel
              currentPubkey={currentPubkey}
              isSinglePanelView={
                useSplitAuxiliaryPane ? false : isSinglePanelView
              }
              layout={useSplitAuxiliaryPane ? "split" : "standalone"}
              transparentChrome={useSplitAuxiliaryPane}
              onClose={onCloseProfilePanel}
              onOpenDm={onOpenDm}
              onOpenProfile={onOpenProfilePanel}
              onTabChange={onProfilePanelTabChange}
              onViewChange={onProfilePanelViewChange}
              pubkey={profilePanelPubkey}
              splitPaneClamp
              tab={profilePanelTab}
              view={profilePanelView}
              widthPx={threadPanelWidthPx}
            />
          );
          return wrapAux(panel, "user-profile-panel");
        })()
      ) : null}
    </div>
  );
});
