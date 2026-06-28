import * as React from "react";
import { Bot, Hash, LogIn, Plus, Sparkles, UserPlus } from "lucide-react";
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
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import {
  buildVideoReviewCommentsByRootId,
  buildVideoReviewContextForMessage,
} from "@/features/messages/lib/videoReviewContext";
import { useComposerHeightPadding } from "@/features/messages/ui/useComposerHeightPadding";
import { TypingIndicatorRow } from "@/features/messages/ui/TypingIndicatorRow";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
import {
  type ProfilePanelTab,
  type ProfilePanelView,
  UserProfilePanel,
} from "@/features/profile/ui/UserProfilePanel";
import { ChannelFindBar } from "@/features/search/ui/ChannelFindBar";
import { AgentSessionThreadPanel } from "@/features/channels/ui/AgentSessionThreadPanel";
import { ChannelManagementAuxiliaryPanel } from "@/features/channels/ui/ChannelManagementAuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import {
  BotActivityComposerAction,
  type BotActivityAgent,
} from "@/features/channels/ui/BotActivityBar";
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
import * as agentSessionSelection from "@/features/channels/ui/agentSessionSelection";
import type { ChannelAgentSessionAgent } from "@/features/channels/ui/useChannelAgentSessions";
import { Button } from "@/shared/ui/button";
import type { useChannelFind } from "@/features/search/useChannelFind";
import {
  buildMainTimelineEntries,
  type MainTimelineEntry,
} from "@/features/messages/lib/threadPanel";
import { useRenderScopedReactionHydration } from "@/features/messages/lib/useRenderScopedReactionHydration";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { isWelcomeChannel } from "@/features/onboarding/welcome";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import type { Channel } from "@/shared/api/types";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
type ChannelPaneProps = {
  activeChannel: Channel | null;
  activityAgents?: BotActivityAgent[];
  agentPubkeys?: ReadonlySet<string>;
  agentSessionAgents: ChannelAgentSessionAgent[];
  botTypingEntries: TypingIndicatorEntry[];
  channelFind: ReturnType<typeof useChannelFind>;
  channelManagementOpen?: boolean;
  currentPubkey?: string;
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  fetchOlder?: () => Promise<void>;
  header?: React.ReactNode;
  hasOlderMessages?: boolean;
  isFetchingOlder?: boolean;
  isJoining?: boolean;
  isSinglePanelView?: boolean;
  isSending: boolean;
  isTimelineLoading: boolean;
  messages: TimelineMessage[];
  firstUnreadMessageId?: string | null;
  unreadCount?: number;
  canResetThreadPanelWidth: boolean;
  onCancelEdit?: () => void;
  onCancelThreadReply: () => void;
  onCloseAgentSession: () => void;
  onCloseChannelManagement?: () => void;
  onChannelManagementDeleted?: () => void;
  onCloseProfilePanel: () => void;
  onAddAgent?: () => void;
  onCreateChannel?: () => void;
  onCloseThread: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onExpandThreadReplies: (message: TimelineMessage) => void;
  onJoinChannel?: () => Promise<void>;
  onOpenAgentSession: (pubkey: string) => void;
  onOpenDm?: (pubkeys: string[]) => Promise<void> | void;
  onOpenMembers?: () => void;
  onOpenProfilePanel: (pubkey: string) => void;
  onOpenThread: (message: TimelineMessage) => void;
  onResetThreadPanelWidth: () => void;
  onSelectThreadReplyTarget: (message: TimelineMessage) => void;
  onSendMessage: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  onSendThreadReply: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  onTargetReached?: (messageId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  onThreadScrollTargetResolved: () => void;
  onThreadPanelResizeStart: (
    event: React.PointerEvent<HTMLButtonElement>,
  ) => void;
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  openThreadHeadId: string | null;
  shouldShowThreadSkeleton: boolean;
  openAgentSessionPubkey: string | null;
  onProfilePanelViewChange: (
    view: ProfilePanelView,
    options?: { replace?: boolean },
  ) => void;
  onProfilePanelTabChange: (
    tab: ProfilePanelTab,
    options?: { replace?: boolean },
  ) => void;
  profilePanelPubkey?: string | null;
  profilePanelTab: ProfilePanelTab;
  profilePanelView: ProfilePanelView;
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
  threadPanelWidthPx: number;
  threadTypingPubkeys: string[];
  threadReplyTargetMessage: TimelineMessage | null;
  threadScrollTargetId: string | null;
  threadUnreadCounts?: ReadonlyMap<string, number>;
  threadReplyUnreadCounts?: ReadonlyMap<string, number>;
  threadFirstUnreadReplyId?: string | null;
  targetMessageId: string | null;
  typingPubkeys: string[];
  isFollowingThread?: boolean;
  onFollowThread?: () => void;
  onUnfollowThread?: () => void;
  followThreadById?: (rootId: string) => void;
  unfollowThreadById?: (rootId: string) => void;
  isFollowingThreadById?: (rootId: string) => boolean;
  isMessageUnreadById?: (messageId: string) => boolean;
};
export const ChannelPane = React.memo(function ChannelPane({
  activeChannel,
  agentPubkeys,
  agentSessionAgents,
  activityAgents = agentSessionAgents,
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
  firstUnreadMessageId = null,
  unreadCount = 0,
  canResetThreadPanelWidth,
  onCancelEdit,
  onCancelThreadReply,
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
  const mainComposerMedia = useMediaUpload();
  const isNonMemberView =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt;
  const hasMainComposerOverlay = !isNonMemberView;
  const activeChannelId = activeChannel?.id ?? null;
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
    ) => {
      const shouldCompleteWelcomeBanner =
        isActiveWelcomeChannel &&
        (containsWelcomePersonaMention(content) ||
          mentionsKnownAgent(mentionPubkeys, knownAgentPubkeys));

      messageTimelineRef.current?.scrollToBottomOnNextUpdate();
      await onSendMessage(content, mentionPubkeys, mediaTags);

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
  const composerBotTypingPubkeys = React.useMemo(() => {
    const pubkeys: string[] = [];
    for (const entry of botTypingEntries) {
      if (entry.threadHeadId !== null) {
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
  }, [botTypingEntries]);
  const hasComposerBotActivity = composerBotTypingPubkeys.length > 0;
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
    () => buildMainTimelineEntries(visibleMessages),
    [visibleMessages],
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
              className="pointer-events-none absolute inset-x-0 bottom-0 z-10"
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
                <div className="h-7 overflow-visible bg-background px-5 pb-1 pt-0">
                  <div className="flex h-full w-full items-center gap-2 overflow-visible">
                    {hasComposerBotActivity ? (
                      <div className="shrink-0 overflow-visible">
                        <BotActivityComposerAction
                          agents={activityAgents}
                          channelId={activeChannel?.id ?? null}
                          onOpenAgentSession={onOpenAgentSession}
                          openAgentSessionPubkey={openAgentSessionPubkey}
                          profiles={profiles}
                          typingBotPubkeys={composerBotTypingPubkeys}
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
              isFollowingThread={isFollowingThread}
              isMessageUnreadById={isMessageUnreadById}
              isSending={isSending}
              isSinglePanelView={
                useSplitAuxiliaryPane ? false : isSinglePanelView
              }
              layout={useSplitAuxiliaryPane ? "split" : "standalone"}
              transparentChrome={useSplitAuxiliaryPane}
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
                    typingBotPubkeys={threadComposerBotTypingPubkeys}
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
                agentSessionSelection.isAgentInActivityList({
                  activityAgents,
                  selectedAgent,
                })
                  ? activeChannel
                  : null
              }
              isWorking={botTypingEntries.some(
                (entry) =>
                  entry.pubkey.toLowerCase() ===
                  selectedAgent.pubkey.toLowerCase(),
              )}
              isSinglePanelView={
                useSplitAuxiliaryPane ? false : isSinglePanelView
              }
              layout={useSplitAuxiliaryPane ? "split" : "standalone"}
              transparentChrome={useSplitAuxiliaryPane}
              profiles={profiles}
              onBackToProfile={() => onOpenProfilePanel(selectedAgent.pubkey)}
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
