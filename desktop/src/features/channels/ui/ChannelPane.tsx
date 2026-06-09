import * as React from "react";
import { Hash, LogIn } from "lucide-react";

import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { MessageThreadPanel } from "@/features/messages/ui/MessageThreadPanel";
import { MessageTimeline } from "@/features/messages/ui/MessageTimeline";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { useComposerHeightPadding } from "@/features/messages/ui/useComposerHeightPadding";
import { TypingIndicatorRow } from "@/features/messages/ui/TypingIndicatorRow";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
import { UserProfilePanel } from "@/features/profile/ui/UserProfilePanel";
import { ChannelFindBar } from "@/features/search/ui/ChannelFindBar";
import { AgentSessionThreadPanel } from "@/features/channels/ui/AgentSessionThreadPanel";
import {
  BotActivityComposerAction,
  type BotActivityAgent,
} from "@/features/channels/ui/BotActivityBar";
import type { ChannelAgentSessionAgent } from "@/features/channels/ui/useChannelAgentSessions";
import { Button } from "@/shared/ui/button";
import type { useChannelFind } from "@/features/search/useChannelFind";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";

type ChannelPaneProps = {
  activeChannel: Channel | null;
  activityAgents?: BotActivityAgent[];
  agentSessionAgents: ChannelAgentSessionAgent[];
  botTypingEntries: TypingIndicatorEntry[];
  channelFind: ReturnType<typeof useChannelFind>;
  currentPubkey?: string;
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  fetchOlder?: () => Promise<void>;
  hasOlderMessages?: boolean;
  isFetchingOlder?: boolean;
  isJoining?: boolean;
  isSinglePanelView?: boolean;
  isSending: boolean;
  isTimelineLoading: boolean;
  messages: TimelineMessage[];
  canResetThreadPanelWidth: boolean;
  onCancelEdit?: () => void;
  onCancelThreadReply: () => void;
  onCloseAgentSession: () => void;
  onCloseProfilePanel: () => void;
  onCloseThread: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onMarkUnread?: (message: TimelineMessage) => void;
  onExpandThreadReplies: (message: TimelineMessage) => void;
  onJoinChannel?: () => Promise<void>;
  onOpenAgentSession: (pubkey: string) => void;
  onOpenDm?: (pubkeys: string[]) => void;
  onOpenProfilePanel: (pubkey: string) => void;
  onOpenThread: (message: TimelineMessage) => void;
  onResetThreadPanelWidth: () => void;
  onSelectThreadReplyTarget: (message: TimelineMessage) => void;
  onSendMessage: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
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
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  openThreadHeadId: string | null;
  openAgentSessionPubkey: string | null;
  profilePanelPubkey?: string | null;
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
  threadPanelWidthPx: number;
  threadTypingPubkeys: string[];
  threadReplyTargetMessage: TimelineMessage | null;
  threadScrollTargetId: string | null;
  targetMessageId: string | null;
  typingPubkeys: string[];
  isFollowingThread?: boolean;
  onFollowThread?: () => void;
  onUnfollowThread?: () => void;
  followThreadById?: (rootId: string) => void;
  unfollowThreadById?: (rootId: string) => void;
  isFollowingThreadById?: (rootId: string) => boolean;
};

export const ChannelPane = React.memo(function ChannelPane({
  activeChannel,
  agentSessionAgents,
  activityAgents = agentSessionAgents,
  botTypingEntries,
  channelFind,
  currentPubkey,
  editTarget = null,
  fetchOlder,
  hasOlderMessages,
  isFetchingOlder,
  followThreadById,
  isFollowingThread,
  isFollowingThreadById,
  isJoining = false,
  isSinglePanelView = false,
  isSending,
  isTimelineLoading,
  messages,
  canResetThreadPanelWidth,
  onCancelEdit,
  onCancelThreadReply,
  onCloseAgentSession,
  onCloseProfilePanel,
  onCloseThread,
  onDelete,
  onEdit,
  onEditSave,
  onFollowThread,
  onMarkUnread,
  onExpandThreadReplies,
  onJoinChannel,
  onOpenAgentSession,
  onOpenDm,
  onOpenProfilePanel,
  onOpenThread,
  onResetThreadPanelWidth,
  onSelectThreadReplyTarget,
  onSendMessage,
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
  openAgentSessionPubkey,
  profilePanelPubkey,
  targetMessageId,
  threadHeadMessage,
  threadMessages,
  threadPanelWidthPx,
  threadScrollTargetId,
  threadTypingPubkeys,
  threadReplyTargetMessage,
  typingPubkeys,
}: ChannelPaneProps) {
  const timelineScrollRef = React.useRef<HTMLDivElement>(null);
  const composerWrapperRef = React.useRef<HTMLDivElement>(null);
  const isNonMemberView =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt;
  const hasMainComposerOverlay = !isNonMemberView;
  useComposerHeightPadding(
    timelineScrollRef,
    composerWrapperRef,
    `${isSinglePanelView}:${hasMainComposerOverlay}`,
  );

  // Scope the edit target to the correct composer: if the message being edited
  // lives inside the open thread (thread head or a reply), show the editing UI
  // only in the thread panel; otherwise show it in the main channel composer.
  const isEditInThread =
    editTarget != null &&
    threadHeadMessage != null &&
    (editTarget.id === threadHeadMessage.id ||
      threadMessages.some((entry) => entry.message.id === editTarget.id));
  const mainEditTarget = editTarget && !isEditInThread ? editTarget : null;
  const threadEditTarget = editTarget && isEditInThread ? editTarget : null;

  // ↑-to-edit resolvers. Find the most recent message authored by the current
  // user in the relevant scope and enter edit mode via `onEdit`. Editability
  // mirrors the action bar's gate (`message.pubkey === currentPubkey`); we
  // also skip optimistic `pending` messages, which have no persisted event id
  // to target. Both scopes are passed in chronological (oldest→newest) order,
  // so we select by newest `createdAt` and break ties toward the later array
  // position (`>=`) — `createdAt` is second-granularity, so a reply sent in
  // the same second as the message before it must still win. Returns true when
  // a target was found so MessageComposer can swallow the ArrowUp.
  const findLastOwnEditable = React.useCallback(
    (candidates: TimelineMessage[]): TimelineMessage | null => {
      if (!onEdit || !currentPubkey) return null;
      let best: TimelineMessage | null = null;
      for (const message of candidates) {
        if (message.pubkey !== currentPubkey || message.pending) continue;
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
    // Thread scope = the open thread head plus its replies, in chronological
    // order. The head is oldest, so append it first.
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
  const directMessageIntro = React.useMemo(() => {
    if (activeChannel?.channelType !== "dm") {
      return null;
    }

    const participants = activeChannel.participantPubkeys.map(
      (pubkey, index) => ({
        fallbackName: activeChannel.participants[index] ?? null,
        pubkey,
      }),
    );
    const otherParticipants = currentPubkey
      ? participants.filter(
          (participant) =>
            participant.pubkey.toLowerCase() !== currentPubkey.toLowerCase(),
        )
      : participants;
    const [participant] =
      otherParticipants.length > 0 ? otherParticipants : participants;

    if (!participant) {
      return null;
    }

    const profile = profiles?.[participant.pubkey.toLowerCase()] ?? null;
    const displayName = resolveUserLabel({
      currentPubkey,
      fallbackName: participant.fallbackName,
      profiles,
      pubkey: participant.pubkey,
    });

    return {
      avatarUrl: profile?.avatarUrl ?? null,
      displayName,
    };
  }, [activeChannel, currentPubkey, profiles]);

  const selectedAgent = React.useMemo(
    () =>
      openAgentSessionPubkey
        ? (agentSessionAgents.find(
            (agent) => agent.pubkey === openAgentSessionPubkey,
          ) ?? null)
        : null,
    [agentSessionAgents, openAgentSessionPubkey],
  );
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
      {!isSinglePanelView ? (
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          {channelFind.isOpen ? (
            <ChannelFindBar
              matchCount={channelFind.matchCount}
              matchIndex={channelFind.activeIndex}
              onClose={channelFind.close}
              onNext={channelFind.goToNext}
              onPrevious={channelFind.goToPrevious}
              onQueryChange={channelFind.setQuery}
              query={channelFind.query}
            />
          ) : null}
          <MessageTimeline
            channelId={activeChannel?.id}
            directMessageIntro={directMessageIntro}
            scrollContainerRef={timelineScrollRef}
            currentPubkey={currentPubkey}
            fetchOlder={fetchOlder}
            followThreadById={followThreadById}
            hasComposerOverlay={hasMainComposerOverlay}
            hasOlderMessages={hasOlderMessages}
            isFetchingOlder={isFetchingOlder}
            isFollowingThreadById={isFollowingThreadById}
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
            messages={messages}
            onDelete={onDelete}
            onEdit={onEdit}
            onMarkUnread={onMarkUnread}
            onReply={activeChannel?.archivedAt ? undefined : onOpenThread}
            onTargetReached={onTargetReached}
            onToggleReaction={onToggleReaction}
            searchActiveMessageId={channelFind.activeMatch?.messageId ?? null}
            searchMatchingMessageIds={channelFind.matchingMessageIds}
            searchQuery={channelFind.query}
            targetMessageId={targetMessageId}
          />
          {isNonMemberView ? (
            <div
              data-testid="join-banner"
              className="flex items-center gap-3 border-t border-border/80 bg-card/50 px-4 py-3"
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
                <LogIn className="mr-1.5 h-3.5 w-3.5" />
                {isJoining ? "Joining..." : "Join to participate"}
              </Button>
            </div>
          ) : (
            <div
              className="pointer-events-none absolute inset-x-0 bottom-0 z-10"
              ref={composerWrapperRef}
            >
              <div className="pointer-events-auto">
                <MessageComposer
                  channelId={activeChannel?.id ?? null}
                  channelName={activeChannel?.name ?? "channel"}
                  disabled={isComposerDisabled}
                  editTarget={mainEditTarget}
                  isSending={isSending}
                  onCancelEdit={onCancelEdit}
                  onEditLastOwnMessage={handleEditLastOwnMainMessage}
                  onEditSave={onEditSave}
                  onSend={onSendMessage}
                  profiles={profiles}
                  placeholder={
                    activeChannel?.archivedAt
                      ? "Archived channels are read-only."
                      : activeChannel?.channelType === "forum"
                        ? "Forum posting is not wired in this pass."
                        : activeChannel
                          ? `Message #${activeChannel.name}`
                          : "Select a channel"
                  }
                  showTopBorder={false}
                />
                <div className="h-7 overflow-visible bg-background px-4 pb-1 pt-0 sm:px-6">
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
                        className="min-w-0 flex-1 px-0 py-0"
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
        </div>
      ) : null}

      {threadHeadMessage ? (
        <MessageThreadPanel
          channel={activeChannel}
          channelId={activeChannel?.id ?? null}
          channelName={activeChannel?.name ?? "channel"}
          currentPubkey={currentPubkey}
          disabled={isComposerDisabled}
          editTarget={threadEditTarget}
          isFollowingThread={isFollowingThread}
          isSending={isSending}
          isSinglePanelView={isSinglePanelView}
          onCancelEdit={onCancelEdit}
          onCancelReply={onCancelThreadReply}
          onClose={onCloseThread}
          onDelete={onDelete}
          onEdit={onEdit}
          onEditLastOwnMessage={handleEditLastOwnThreadMessage}
          onEditSave={onEditSave}
          onFollowThread={onFollowThread}
          onMarkUnread={onMarkUnread}
          onExpandReplies={onExpandThreadReplies}
          onSelectReplyTarget={onSelectThreadReplyTarget}
          onSend={onSendThreadReply}
          onScrollTargetResolved={onThreadScrollTargetResolved}
          onToggleReaction={onToggleReaction}
          onUnfollowThread={onUnfollowThread}
          profiles={profiles}
          replyTargetMessage={threadReplyTargetMessage}
          scrollTargetId={threadScrollTargetId}
          canResetWidth={canResetThreadPanelWidth}
          onResetWidth={onResetThreadPanelWidth}
          onResizeStart={onThreadPanelResizeStart}
          threadHead={threadHeadMessage}
          widthPx={threadPanelWidthPx}
          threadReplies={threadMessages}
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
      ) : activeChannel && selectedAgent ? (
        <AgentSessionThreadPanel
          agent={selectedAgent}
          canResetWidth={canResetThreadPanelWidth}
          canInterruptTurn={selectedAgent.canInterruptTurn}
          channel={activeChannel}
          isWorking={botTypingEntries.some(
            (entry) =>
              entry.pubkey.toLowerCase() === selectedAgent.pubkey.toLowerCase(),
          )}
          isSinglePanelView={isSinglePanelView}
          profiles={profiles}
          onBackToProfile={() => onOpenProfilePanel(selectedAgent.pubkey)}
          onClose={onCloseAgentSession}
          onResetWidth={onResetThreadPanelWidth}
          onResizeStart={onThreadPanelResizeStart}
          widthPx={threadPanelWidthPx}
        />
      ) : profilePanelPubkey ? (
        <UserProfilePanel
          canResetWidth={canResetThreadPanelWidth}
          currentPubkey={currentPubkey}
          isSinglePanelView={isSinglePanelView}
          onClose={onCloseProfilePanel}
          onOpenDm={onOpenDm}
          onResetWidth={onResetThreadPanelWidth}
          onResizeStart={onThreadPanelResizeStart}
          pubkey={profilePanelPubkey}
          splitPaneClamp
          widthPx={threadPanelWidthPx}
        />
      ) : null}
    </div>
  );
});
