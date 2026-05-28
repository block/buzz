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
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import type { useChannelFind } from "@/features/search/useChannelFind";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
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
  isSending: boolean;
  isTimelineLoading: boolean;
  messages: TimelineMessage[];
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
  onOpenThread: (message: TimelineMessage) => void;
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
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  openThreadHeadId: string | null;
  openAgentSessionPubkey: string | null;
  profilePanelPubkey?: string | null;
  threadHeadMessage: TimelineMessage | null;
  threadMessages: MainTimelineEntry[];
  threadTypingPubkeys: string[];
  threadReplyTargetId: string | null;
  threadReplyTargetMessage: TimelineMessage | null;
  threadScrollTargetId: string | null;
  targetMessageId: string | null;
  typingPubkeys: string[];
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
  isJoining = false,
  isSending,
  isTimelineLoading,
  messages,
  onCancelEdit,
  onCancelThreadReply,
  onCloseAgentSession,
  onCloseProfilePanel,
  onCloseThread,
  onDelete,
  onEdit,
  onEditSave,
  onMarkUnread,
  onExpandThreadReplies,
  onJoinChannel,
  onOpenAgentSession,
  onOpenDm,
  onOpenThread,
  onSelectThreadReplyTarget,
  onSendMessage,
  onSendThreadReply,
  onThreadScrollTargetResolved,
  onTargetReached,
  onToggleReaction,
  personaLookup,
  profiles,
  openThreadHeadId,
  openAgentSessionPubkey,
  profilePanelPubkey,
  targetMessageId,
  threadHeadMessage,
  threadMessages,
  threadScrollTargetId,
  threadTypingPubkeys,
  threadReplyTargetId,
  threadReplyTargetMessage,
  typingPubkeys,
}: ChannelPaneProps) {
  const {
    canReset: canResetThreadPanelWidth,
    onResetWidth: handleThreadPanelWidthReset,
    onResizeStart: handleThreadPanelResizeStart,
    widthPx: threadPanelWidthPx,
  } = useThreadPanelWidth();

  const timelineScrollRef = React.useRef<HTMLDivElement>(null);
  const composerWrapperRef = React.useRef<HTMLDivElement>(null);
  useComposerHeightPadding(timelineScrollRef, composerWrapperRef);

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

  const isNonMemberView =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt;

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
          activeReplyTargetId={openThreadHeadId}
          scrollContainerRef={timelineScrollRef}
          currentPubkey={currentPubkey}
          fetchOlder={fetchOlder}
          hasOlderMessages={hasOlderMessages}
          isFetchingOlder={isFetchingOlder}
          personaLookup={personaLookup}
          profiles={profiles}
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
              <div className="h-7 bg-background px-4 pb-1 pt-0 sm:px-6 -mt-1">
                <div className="flex h-full w-full items-center gap-2">
                  {hasComposerBotActivity ? (
                    <div className="shrink-0">
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
                      variant="activity"
                    />
                  ) : null}
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {threadHeadMessage ? (
        <MessageThreadPanel
          channel={activeChannel}
          channelId={activeChannel?.id ?? null}
          channelName={activeChannel?.name ?? "channel"}
          currentPubkey={currentPubkey}
          disabled={isComposerDisabled}
          editTarget={threadEditTarget}
          isSending={isSending}
          onCancelEdit={onCancelEdit}
          onCancelReply={onCancelThreadReply}
          onClose={onCloseThread}
          onDelete={onDelete}
          onEdit={onEdit}
          onEditSave={onEditSave}
          onMarkUnread={onMarkUnread}
          onExpandReplies={onExpandThreadReplies}
          onSelectReplyTarget={onSelectThreadReplyTarget}
          onSend={onSendThreadReply}
          onScrollTargetResolved={onThreadScrollTargetResolved}
          onToggleReaction={onToggleReaction}
          profiles={profiles}
          replyTargetId={threadReplyTargetId}
          replyTargetMessage={threadReplyTargetMessage}
          scrollTargetId={threadScrollTargetId}
          canResetWidth={canResetThreadPanelWidth}
          onResetWidth={handleThreadPanelWidthReset}
          onResizeStart={handleThreadPanelResizeStart}
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
          onClose={onCloseAgentSession}
          onResetWidth={handleThreadPanelWidthReset}
          onResizeStart={handleThreadPanelResizeStart}
          widthPx={threadPanelWidthPx}
        />
      ) : profilePanelPubkey ? (
        <UserProfilePanel
          canResetWidth={canResetThreadPanelWidth}
          currentPubkey={currentPubkey}
          onClose={onCloseProfilePanel}
          onOpenDm={onOpenDm}
          onResetWidth={handleThreadPanelWidthReset}
          onResizeStart={handleThreadPanelResizeStart}
          pubkey={profilePanelPubkey}
          widthPx={threadPanelWidthPx}
        />
      ) : null}
    </div>
  );
});
