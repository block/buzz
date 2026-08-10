import * as React from "react";
import { Hash, LogIn } from "lucide-react";
import { AnimatePresence } from "motion/react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { ComposerDockBackdrop } from "@/features/messages/ui/ComposerDockBackdrop";
import { ComposerUploadProgressOverlay } from "@/features/messages/ui/ComposerUploadProgressOverlay";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { ComposerTimeoutBanner } from "@/features/moderation/ui/ComposerTimeoutBanner";
import { useTimeoutState } from "@/features/moderation/lib/timeoutStore";
import { isModerationDm } from "@/features/moderation/lib/moderationDm";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { DropZoneOverlay } from "@/features/messages/ui/ComposerAttachments";
import { MessageThreadPanel } from "@/features/messages/ui/MessageThreadPanel";
import { MessageThreadPanelSkeleton } from "@/features/messages/ui/MessageThreadPanelSkeleton";
import {
  MessageTimeline,
  type MessageTimelineHandle,
} from "@/features/messages/ui/MessageTimeline";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import {
  getDmHuddleMemberPubkeys,
  hasOtherDmParticipant,
} from "@/features/channels/lib/dmHuddleMembers";
import { buildVideoReviewPresentationByMessageId } from "@/features/messages/lib/videoReviewContext";
import { useComposerHeightPadding } from "@/features/messages/ui/useComposerHeightPadding";
import { UserProfilePanel } from "@/features/profile/ui/UserProfilePanel";
import { ChannelFindBar } from "@/features/search/ui/ChannelFindBar";
import { AgentSessionThreadPanel } from "@/features/channels/ui/AgentSessionThreadPanel";
import { ChannelManagementAuxiliaryPanel } from "@/features/channels/ui/ChannelManagementAuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { ThreadViewModeToggle } from "@/features/channels/ui/ThreadViewModeToggle";
import { FocusThreadDrawer } from "@/features/channels/ui/FocusThreadDrawer";
import { THREAD_SURFACE_KEY } from "@/features/channels/lib/threadFocusLayout";
import { getThreadPanelLayout } from "@/features/channels/lib/threadPanelLayout";
import { useThreadViewMode } from "@/features/channels/lib/threadViewModePreference";
import { useThreadViewModeSwitch } from "@/features/channels/ui/useThreadViewModeSwitch";
import { useFocusDrawerPresence } from "@/features/channels/ui/useFocusDrawerPresence";
import { useChannelWorkingAgentPubkeys } from "@/features/agents/agentWorkingSignal";
import { useCardMintJobs } from "@/features/agents/cardMintStore";
import { BotActivityComposerAction } from "@/features/channels/ui/BotActivityBar";
import { ChannelComposerActivityAccessory } from "@/features/channels/ui/ChannelComposerActivityAccessory";
import {
  containsWelcomePersonaMention,
  WelcomeComposerGuidanceLayer,
} from "@/features/channels/ui/WelcomeComposerBanner";
import { useWelcomeComposerBanner } from "@/features/channels/ui/useWelcomeComposerBanner";
import { mentionsKnownAgent } from "@/features/channels/ui/ChannelPane.helpers";
import { HuddleStartingView, HuddleTranscriptIntro } from "@/features/huddle";
import { useChannelIntro } from "@/features/channels/ui/useChannelIntro";
import type { ChannelPaneProps } from "@/features/channels/ui/ChannelPane.types";
import * as agentSessionSelection from "@/features/channels/ui/agentSessionSelection";
import { usePrepareDmSendChannel } from "@/features/channels/ui/usePrepareDmSendChannel";
import { useChannelPaneMessages } from "@/features/channels/ui/useChannelPaneMessages";
import { Button } from "@/shared/ui/button";
import { useRenderScopedReactionHydration } from "@/features/messages/lib/useRenderScopedReactionHydration";
import type { TimelineMessage } from "@/features/messages/types";
import { isWelcomeExperienceChannel as isWelcomeExperience } from "@/features/onboarding/welcome";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
const HUDDLE_TRANSCRIPT_ROOT_STYLE = {
  "--buzz-channel-content-top-padding": "0rem",
  "--channel-top-chrome-height": "0.25rem",
} as React.CSSProperties;
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
  historyExhausted,
  isFetchingOlder,
  isHuddleTranscript = false,
  followThreadById,
  isFollowingThread,
  isFollowingThreadById,
  isMessageUnreadById,
  isJoining = false,
  isSinglePanelView = false,
  isSending,
  isTimelineLoading,
  entranceMessageId = null,
  onEntranceMessageComplete,
  welcomeKickoffStage = null,
  welcomeKickoffSettingUp = false,
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
  onBrowseChannels,
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
  ownerProfiles,
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
  threadAllMessages,
  threadHeadMessage,
  threadMessages,
  threadMessagesPending = false,
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
  const { goChannel } = useAppNavigation();
  const prepareDmSendChannel = usePrepareDmSendChannel(
    activeChannel,
    currentPubkey,
  );
  const mainComposerMedia = useMediaUpload({ deferUploadsUntilSend: true });
  const [isMainDeferredEditPending, setMainDeferredEditPending] =
    React.useState(false);
  const isNonMemberView =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt;
  const hasMainComposerOverlay = !isNonMemberView;
  const activeChannelId = activeChannel?.id ?? null;
  const activeChannelIdRef = React.useRef(activeChannelId);
  const channelPaneMountedRef = React.useRef(false);
  activeChannelIdRef.current = activeChannelId;
  React.useEffect(() => {
    channelPaneMountedRef.current = true;
    return () => {
      channelPaneMountedRef.current = false;
    };
  }, []);
  // Clear only the auto-send key so thread state survives deferred submission;
  // older wrappers fall back to goChannel to prevent back-navigation replay.
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
    activeChannel !== null && isWelcomeExperience(activeChannel);
  useComposerHeightPadding(
    timelineScrollRef,
    composerWrapperRef,
    `${activeChannelId}:${isSinglePanelView}:${hasMainComposerOverlay}`,
    "css-variable",
    () => messageTimelineRef.current?.settleAtBottom() ?? false,
  );
  const {
    bannerState: welcomeComposerBannerState,
    completeBanner: completeWelcomeComposerBanner,
    dismissBanner: handleDismissWelcomeBanner,
  } = useWelcomeComposerBanner(
    activeChannelId,
    isActiveWelcomeChannel,
    currentPubkey ?? null,
  );
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

  const timeoutState = useTimeoutState();

  // A moderation DM (1:1 with the relay identity) is read-only for the member;
  // only DMs pay for the NIP-11 `self` lookup. Fails open: no `relaySelf` →
  // ordinary DM, composer enabled.
  const relaySelfQuery = useRelaySelfQuery(activeChannel?.channelType === "dm");
  const isModerationDmChannel = isModerationDm(
    activeChannel ?? null,
    currentPubkey,
    relaySelfQuery.data,
  );

  const isComposerDisabled =
    !activeChannel?.isMember ||
    activeChannel.archivedAt !== null ||
    activeChannel.channelType === "forum" ||
    timeoutState.active ||
    isModerationDmChannel ||
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

      if (
        channelId &&
        channelId !== activeChannelId &&
        channelPaneMountedRef.current &&
        activeChannelIdRef.current === activeChannelId
      ) {
        await goChannel(channelId, { replace: true });
      }

      if (shouldCompleteWelcomeBanner) {
        completeWelcomeComposerBanner();
      }
    },
    [
      activeChannelId,
      completeWelcomeComposerBanner,
      goChannel,
      isActiveWelcomeChannel,
      knownAgentPubkeys,
      onSendMessage,
    ],
  );
  const canDropInMainColumn =
    hasMainComposerOverlay &&
    !isComposerDisabled &&
    !isMainDeferredEditPending &&
    !isSinglePanelView;
  const hasTypingActivity = typingPubkeys.length > 0;
  // Unified working set for the composer bar: observer-derived turns primary,
  // bot typing fallback (both folded together by agentWorkingSignal). This is
  // what makes the bar show for an agent whose observer stream is live but
  // whose typing signal never arrives — and vice versa.
  const composerWorkingBotPubkeys = useChannelWorkingAgentPubkeys(
    activeChannel?.id ?? null,
  );
  const hasComposerBotActivity = composerWorkingBotPubkeys.length > 0;
  const hasCardMintActivity = useCardMintJobs().length > 0;
  const hasComposerBottomActivity =
    hasComposerBotActivity || hasTypingActivity || hasCardMintActivity;
  const threadComposerBotTypingPubkeys = React.useMemo(() => {
    if (!openThreadHeadId) return [];
    return botTypingEntries
      .filter((entry) => entry.threadHeadId === openThreadHeadId)
      .map((entry) => entry.pubkey)
      .filter(
        (pubkey, index, all) =>
          all.findIndex(
            (candidate) => candidate.toLowerCase() === pubkey.toLowerCase(),
          ) === index,
      );
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

  const handleWelcomeAddAgent = React.useCallback(() => {
    onAddAgent?.({
      beforeSend: () =>
        messageTimelineRef.current?.scrollToBottomOnNextUpdate(),
    });
  }, [onAddAgent]);
  const standardChannelIntro = useChannelIntro({
    activeChannel,
    onAddAgent,
    onBrowseChannels,
    onCreateChannel,
    onOpenMembers,
    onWelcomeAddAgent: onAddAgent ? handleWelcomeAddAgent : undefined,
  });
  const channelIntro = isHuddleTranscript ? null : standardChannelIntro;
  const { mainTimelineEntries, visibleMessages } = useChannelPaneMessages({
    activeChannel,
    isHuddleTranscript,
    messages,
    profiles,
    threadSummaries,
  });
  useRenderScopedReactionHydration({
    activeChannel,
    mainTimelineEntries,
    threadHeadMessage,
    threadMessages,
  });
  const activeVideoReviewCommentSender = activeChannel?.archivedAt
    ? undefined
    : onSendVideoReviewComment;
  const threadVideoReviewPresentation = React.useMemo(() => {
    const messagesById = new Map(
      messages.map((message) => [message.id, message]),
    );
    if (threadHeadMessage) {
      messagesById.set(threadHeadMessage.id, threadHeadMessage);
    }
    for (const message of threadAllMessages) {
      messagesById.set(message.id, message);
    }

    return buildVideoReviewPresentationByMessageId({
      channelId: activeChannel?.id ?? null,
      channelName: activeChannel?.name,
      channelType: activeChannel?.channelType ?? null,
      isSendingVideoReviewComment: isSending,
      messages: [...messagesById.values()],
      onSendVideoReviewComment: activeVideoReviewCommentSender,
      onToggleReaction,
      profiles,
    });
  }, [
    activeChannel,
    activeVideoReviewCommentSender,
    isSending,
    messages,
    onToggleReaction,
    profiles,
    threadAllMessages,
    threadHeadMessage,
  ]);

  const isOverlay = useIsThreadPanelOverlay();
  const useSplitAuxiliaryPane = !isSinglePanelView && !isOverlay;
  const threadViewMode = useThreadViewMode();
  const useFocusThreadDrawer =
    threadViewMode === "focus" &&
    useSplitAuxiliaryPane &&
    (Boolean(threadHeadMessage) || shouldShowThreadSkeleton);
  const { channelIsCovered, markExitComplete } = useFocusDrawerPresence(
    useFocusThreadDrawer,
    onCloseThread,
  );
  const { changeThreadViewMode, layoutScrollTargetId, resolveScrollTarget } =
    useThreadViewModeSwitch({
      activeThreadHeadId: threadHeadMessage?.id ?? null,
      externalScrollTargetId: threadScrollTargetId,
      onExternalTargetResolved: onThreadScrollTargetResolved,
      onModeChange: markExitComplete,
    });
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
  const wrapAux = (
    panel: React.ReactNode,
    testId: string,
    options: { key?: string } = {},
  ) =>
    useSplitAuxiliaryPane ? (
      <RightAuxiliaryPane
        canResetWidth={canResetThreadPanelWidth}
        key={options.key ?? testId}
        onResetWidth={onResetThreadPanelWidth}
        onResizeStart={onThreadPanelResizeStart}
        testId={testId}
        widthPx={threadPanelWidthPx}
      >
        {panel}
      </RightAuxiliaryPane>
    ) : (
      <React.Fragment key={options.key ?? testId}>{panel}</React.Fragment>
    );
  const wrapThreadPanel = (panel: React.ReactNode) =>
    useFocusThreadDrawer ? (
      <FocusThreadDrawer
        channelName={activeChannel?.name ?? "channel"}
        key={THREAD_SURFACE_KEY}
        onClose={onCloseThread}
      >
        {panel}
      </FocusThreadDrawer>
    ) : (
      wrapAux(panel, "message-thread-panel", { key: THREAD_SURFACE_KEY })
    );
  const threadHeaderLeading = useSplitAuxiliaryPane ? (
    <ThreadViewModeToggle onChange={changeThreadViewMode} />
  ) : undefined;
  const threadLayoutProps = getThreadPanelLayout({
    headerLeading: threadHeaderLeading,
    isFocusDrawer: useFocusThreadDrawer,
    isSinglePanelView,
    useSplitAuxiliaryPane,
  });
  const timelineReplyHandler =
    activeChannel?.archivedAt || isHuddleTranscript ? undefined : onOpenThread;
  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden"
      style={isHuddleTranscript ? HUDDLE_TRANSCRIPT_ROOT_STYLE : undefined}
    >
      {!isSinglePanelView && !isHuddleTranscript ? (
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
          inert={channelIsCovered ? true : undefined}
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
          {isHuddleTranscript ? null : header}
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
            channelId={activeChannel?.id}
            channelIntro={channelIntro}
            directMessageIntro={directMessageIntro}
            scrollContainerRef={timelineScrollRef}
            currentPubkey={currentPubkey}
            fetchOlder={fetchOlder}
            followThreadById={followThreadById}
            hasComposerOverlay={hasMainComposerOverlay}
            hasOlderMessages={hasOlderMessages}
            historyExhausted={historyExhausted}
            hideDayDividers={isHuddleTranscript}
            alwaysShowMessageIdentity={isHuddleTranscript}
            hideAgentAccessBadges={isHuddleTranscript}
            pinnedIntro={
              isHuddleTranscript ? <HuddleTranscriptIntro /> : undefined
            }
            huddleMemberPubkeys={huddleMemberPubkeys}
            huddleMemberPubkeysPending={huddleMemberPubkeysPending}
            isFetchingOlder={isFetchingOlder}
            isFollowingThreadById={isFollowingThreadById}
            isMessageUnreadById={isMessageUnreadById}
            personaLookup={personaLookup}
            profiles={profiles}
            ownerProfiles={ownerProfiles}
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
            isLoading={isHuddleTranscript ? false : isTimelineLoading}
            entranceMessageId={entranceMessageId}
            onEntranceMessageComplete={onEntranceMessageComplete}
            mainEntries={mainTimelineEntries}
            threadSummaries={threadSummaries}
            messages={visibleMessages}
            firstUnreadMessageId={firstUnreadMessageId}
            unreadCount={unreadCount}
            onDelete={onDelete}
            onEdit={onEdit}
            onMarkUnread={onMarkUnread}
            onMarkRead={onMarkRead}
            onReply={timelineReplyHandler}
            onOpenThread={isHuddleTranscript ? undefined : onOpenThread}
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
            splitThreadPanelOpen={
              useSplitAuxiliaryPane &&
              !useFocusThreadDrawer &&
              Boolean(openThreadHeadId)
            }
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
              className="pointer-events-none absolute inset-x-0 bottom-0 z-40 isolate before:absolute before:inset-x-0 before:bottom-0 before:-z-10 before:h-24 before:bg-gradient-to-b before:from-transparent before:to-background before:content-[''] after:absolute after:inset-x-0 after:bottom-0 after:-z-10 after:h-12 after:bg-background after:content-['']"
              data-testid="channel-composer-overlay"
              ref={composerWrapperRef}
            >
              <ComposerUploadProgressOverlay />
              <div
                className={cn(
                  "composer-dock composer-overlay-corner-masks relative pointer-events-auto",
                  hasComposerBottomActivity && "composer-dock--with-activity",
                )}
              >
                {isActiveWelcomeChannel && !timeoutState.active ? (
                  <WelcomeComposerGuidanceLayer
                    onDismiss={handleDismissWelcomeBanner}
                    settingUp={welcomeKickoffSettingUp}
                    state={welcomeComposerBannerState}
                  >
                    {welcomeKickoffStage}
                  </WelcomeComposerGuidanceLayer>
                ) : null}
                {timeoutState.active ? (
                  <ComposerTimeoutBanner
                    expiresAtMs={timeoutState.expiresAtMs}
                  />
                ) : null}
                <ComposerDockBackdrop gutterClassName="inset-x-5" />
                <MessageComposer
                  channelId={activeChannel?.id ?? null}
                  channelName={activeChannel?.name ?? "channel"}
                  channelType={activeChannel?.channelType ?? null}
                  containerClassName="px-5 pb-0"
                  layoutMode="dock"
                  disabled={isComposerDisabled}
                  editTarget={mainEditTarget}
                  autoSubmitDraftKey={autoSendDraftKey}
                  onAutoSubmitComplete={handleAutoSubmitComplete}
                  isSending={isSending}
                  mediaController={mainComposerMedia}
                  onDeferredEditPendingChange={setMainDeferredEditPending}
                  onCancelEdit={onCancelEdit}
                  onEditLastOwnMessage={handleEditLastOwnMainMessage}
                  onEditSave={onEditSave}
                  onPrepareSendChannel={
                    activeChannel?.channelType === "dm"
                      ? prepareDmSendChannel
                      : undefined
                  }
                  onSend={handleSendMessage}
                  profiles={profiles}
                  showBackgroundUploadProgress={false}
                  placeholder={
                    timeoutState.active
                      ? "You're timed out by community moderators."
                      : isModerationDmChannel
                        ? "This channel is read-only."
                        : activeChannel?.archivedAt
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
                {/* The activity accessory is anchored in the dock's reserved
                    bottom rail, so fading it cannot change the observed
                    overlay height or move the conversation. Its natural
                    content height remains responsive. */}
                <ChannelComposerActivityAccessory
                  agents={activityAgents}
                  channel={activeChannel}
                  currentPubkey={currentPubkey}
                  onOpenAgentSession={onOpenAgentSession}
                  openAgentSessionPubkey={openAgentSessionPubkey}
                  profiles={profiles}
                  typingPubkeys={typingPubkeys}
                  visible={hasComposerBottomActivity}
                  workingBotPubkeys={composerWorkingBotPubkeys}
                />
              </div>
            </div>
          )}
          {canDropInMainColumn && mainComposerMedia.isDragOver ? (
            <DropZoneOverlay className="z-50 rounded-2xl bg-primary/20 backdrop-blur-sm" />
          ) : null}
        </section>
      ) : null}

      {/*
       * `AnimatePresence` keeps the focus thread drawer mounted through its exit
       * animation — without it the drawer's own existence condition
       * (`useFocusThreadDrawer`, which is derived from `threadHeadMessage`) goes
       * false on the same frame as the close, and there is nothing left to
       * animate. It can hold the real thread through the exit rather than a
       * frozen snapshot because the panel is fully prop-driven.
       */}
      <AnimatePresence onExitComplete={markExitComplete}>
        {channelManagementOpen && activeChannel ? (
          <ChannelManagementAuxiliaryPanel
            activeChannel={activeChannel}
            canResetThreadPanelWidth={canResetThreadPanelWidth}
            currentPubkey={currentPubkey}
            isSinglePanelView={isSinglePanelView}
            key="channel-management-panel"
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
                channel={activeChannel}
                channelId={activeChannel?.id ?? null}
                channelName={activeChannel?.name ?? "channel"}
                currentPubkey={currentPubkey}
                disabled={isComposerDisabled}
                editTarget={threadEditTarget}
                firstUnreadReplyId={threadFirstUnreadReplyId}
                huddleMemberPubkeys={huddleMemberPubkeys}
                huddleMemberPubkeysPending={huddleMemberPubkeysPending}
                isHuddleTranscript={isHuddleTranscript}
                isFollowingThread={isFollowingThread}
                isMessageUnreadById={isMessageUnreadById}
                isSending={isSending}
                {...threadLayoutProps}
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
                onScrollTargetResolved={() => resolveScrollTarget()}
                onScrollTargetSettled={resolveScrollTarget}
                onToggleReaction={onToggleReaction}
                onUnfollowThread={onUnfollowThread}
                profiles={profiles}
                replyTargetMessage={threadReplyTargetMessage}
                scrollTargetHighlights={!layoutScrollTargetId}
                scrollTargetId={layoutScrollTargetId ?? threadScrollTargetId}
                threadHead={threadHeadMessage}
                videoReviewPresentation={threadVideoReviewPresentation}
                widthPx={threadPanelWidthPx}
                threadReplies={threadMessages}
                threadRepliesPending={threadMessagesPending}
                threadUnreadCount={threadUnreadCounts?.get(
                  threadHeadMessage.id,
                )}
                threadReplyUnreadCounts={threadReplyUnreadCounts}
                threadTypingPubkeys={threadTypingPubkeys}
                activityAccessoryVisible={hasThreadComposerBotActivity}
                activityAccessoryContent={
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
            return wrapThreadPanel(panel);
          })()
        ) : shouldShowThreadSkeleton ? (
          (() => {
            if (isHuddleTranscript) {
              return wrapThreadPanel(<HuddleStartingView />);
            }
            const panel = (
              <MessageThreadPanelSkeleton
                {...threadLayoutProps}
                onClose={onCloseThread}
                widthPx={threadPanelWidthPx}
              />
            );
            return wrapThreadPanel(panel);
          })()
        ) : activeChannel && selectedAgent ? (
          (() => {
            // When the panel was opened from a different channel than the
            // currently active one, re-scope it to the active channel so
            // that both the content/header AND channel-backed actions (e.g.
            // Stop current turn) operate on the same channel object.
            const effectiveAgentSessionChannelId =
              openAgentSessionChannelId &&
              activeChannel.id !== openAgentSessionChannelId
                ? activeChannelId
                : openAgentSessionChannelId;
            const panel = (
              <AgentSessionThreadPanel
                agent={selectedAgent}
                canInterruptTurn={selectedAgent.canInterruptTurn}
                channel={
                  effectiveAgentSessionChannelId
                    ? effectiveAgentSessionChannelId === activeChannel.id
                      ? activeChannel
                      : null
                    : agentSessionSelection.isAgentInActivityList({
                          activityAgents,
                          selectedAgent,
                        })
                      ? activeChannel
                      : null
                }
                channelId={effectiveAgentSessionChannelId}
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
                callerChannelId={activeChannelId}
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
      </AnimatePresence>
    </div>
  );
});
