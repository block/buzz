import * as React from "react";
import { ArrowDown, X } from "lucide-react";

import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { MessageComposer } from "./MessageComposer";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { TypingIndicatorRow } from "./TypingIndicatorRow";
import { useTimelineScrollManager } from "./useTimelineScrollManager";

type MessageThreadPanelProps = {
  canResetWidth: boolean;
  channel: Channel | null;
  channelId: string | null;
  channelName: string;
  currentPubkey?: string;
  disabled?: boolean;
  editTarget?: { author: string; body: string; id: string } | null;
  isSending: boolean;
  onCancelEdit?: () => void;
  onCancelReply: () => void;
  onClose: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditSave?: (content: string) => Promise<void>;
  onExpandReplies: (message: TimelineMessage) => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onScrollTargetResolved: () => void;
  onSelectReplyTarget: (message: TimelineMessage) => void;
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  replyTargetId: string | null;
  replyTargetMessage: TimelineMessage | null;
  scrollTargetId: string | null;
  threadHead: TimelineMessage | null;
  threadReplies: MainTimelineEntry[];
  threadTypingPubkeys: string[];
  toolbarExtraActions?: React.ReactNode;
  widthPx: number;
};

function canManageMessage(
  message: TimelineMessage,
  currentPubkey: string | undefined,
): boolean {
  return Boolean(
    currentPubkey &&
      message.pubkey &&
      currentPubkey.toLowerCase() === message.pubkey.toLowerCase(),
  );
}

export function MessageThreadPanel({
  canResetWidth,
  channel,
  channelId,
  channelName,
  currentPubkey,
  disabled = false,
  editTarget,
  isSending,
  onCancelEdit,
  onCancelReply,
  onClose,
  onDelete,
  onEdit,
  onEditSave,
  onExpandReplies,
  onResetWidth,
  onResizeStart,
  onScrollTargetResolved,
  onSelectReplyTarget,
  onSend,
  onToggleReaction,
  profiles,
  replyTargetId,
  replyTargetMessage,
  scrollTargetId,
  threadHead,
  threadReplies,
  threadTypingPubkeys,
  toolbarExtraActions,
  widthPx,
}: MessageThreadPanelProps) {
  const threadBodyRef = React.useRef<HTMLDivElement>(null);
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay);

  const threadHeadId = threadHead?.id ?? null;

  const composerReplyTarget =
    replyTargetMessage && threadHead && replyTargetMessage.id !== threadHead.id
      ? {
          author: replyTargetMessage.author,
          body: replyTargetMessage.body,
          id: replyTargetMessage.id,
        }
      : null;

  const threadMessages = React.useMemo(
    () => threadReplies.map((entry) => entry.message),
    [threadReplies],
  );

  const {
    bottomAnchorRef,
    contentRef,
    isAtBottom,
    newMessageCount,
    scrollToBottom,
    syncScrollState,
  } = useTimelineScrollManager({
    channelId: threadHeadId,
    isLoading: false,
    messages: threadMessages,
    onTargetReached: onScrollTargetResolved,
    scrollContainerRef: threadBodyRef,
    targetMessageId: scrollTargetId,
  });

  if (!threadHead) {
    return null;
  }

  return (
    <>
      {isOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
          !isOverlay && "pt-11",
          isOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="message-thread-panel"
        style={{ width: `${widthPx}px` }}
      >
        {!isOverlay && (
          <button
            aria-label="Resize thread panel"
            className="group absolute inset-y-0 left-0 z-20 w-3 -translate-x-1/2 cursor-col-resize"
            data-testid="message-thread-resize-handle"
            onDoubleClick={canResetWidth ? onResetWidth : undefined}
            onPointerDown={onResizeStart}
            title={
              canResetWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-transparent transition-colors group-hover:bg-border/80" />
          </button>
        )}

        <div className="flex items-center gap-3 px-4 py-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold tracking-tight">Thread</h2>
          </div>
          <Button
            aria-label="Close thread"
            data-testid="message-thread-close"
            onClick={onClose}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div
          className="min-h-0 flex-1 overflow-y-auto pb-24"
          data-testid="message-thread-body"
          onScroll={syncScrollState}
          ref={threadBodyRef}
        >
          <div className="pb-10" ref={contentRef}>
            <div className="px-3 pb-1 pt-0" data-testid="message-thread-head">
              <div className="rounded-2xl">
                <MessageRow
                  activeReplyTargetId={replyTargetId}
                  layoutVariant="thread-reply"
                  message={threadHead}
                  onDelete={
                    onDelete && canManageMessage(threadHead, currentPubkey)
                      ? onDelete
                      : undefined
                  }
                  onEdit={
                    onEdit && canManageMessage(threadHead, currentPubkey)
                      ? onEdit
                      : undefined
                  }
                  onToggleReaction={onToggleReaction}
                  profiles={profiles}
                />
              </div>
            </div>

            <div
              className="px-3 pb-3 pt-1"
              data-testid="message-thread-replies"
            >
              {threadReplies.length > 0 ? (
                <div className="space-y-2">
                  {threadReplies.map((entry) => {
                    return (
                      <div key={entry.message.id}>
                        <MessageRow
                          activeReplyTargetId={replyTargetId}
                          layoutVariant="thread-reply"
                          message={entry.message}
                          onDelete={
                            onDelete &&
                            canManageMessage(entry.message, currentPubkey)
                              ? onDelete
                              : undefined
                          }
                          onEdit={
                            onEdit &&
                            canManageMessage(entry.message, currentPubkey)
                              ? onEdit
                              : undefined
                          }
                          onReply={onSelectReplyTarget}
                          onToggleReaction={onToggleReaction}
                          profiles={profiles}
                        />
                        {entry.summary ? (
                          <MessageThreadSummaryRow
                            depth={entry.message.depth}
                            layoutVariant="thread-reply"
                            message={entry.message}
                            onOpenThread={onExpandReplies}
                            summary={entry.summary}
                          />
                        ) : null}
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="rounded-2xl border border-dashed border-border/70 bg-card/40 px-4 py-6 text-center">
                  <p className="text-sm font-medium text-foreground/80">
                    No replies in this branch yet
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Reply in the thread to continue this branch.
                  </p>
                </div>
              )}
              <div aria-hidden className="h-px" ref={bottomAnchorRef} />
            </div>
          </div>
        </div>

        {!isAtBottom ? (
          <div className="pointer-events-none absolute inset-x-0 bottom-36 z-20 flex justify-center px-4">
            <Button
              className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-[11px] font-medium text-muted-foreground shadow-sm backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-3.5"
              data-testid="thread-scroll-to-latest"
              onClick={() => scrollToBottom("smooth")}
              size="sm"
              type="button"
              variant="outline"
            >
              <ArrowDown aria-hidden />
              {newMessageCount > 0
                ? `${newMessageCount} new message${newMessageCount === 1 ? "" : "s"}`
                : "Jump to latest"}
            </Button>
          </div>
        ) : null}

        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
          <div className="pointer-events-auto">
            <MessageComposer
              channelId={channelId}
              channelName={channelName}
              disabled={disabled || isSending || !channelId}
              draftKey={`thread:${threadHead.id}`}
              editTarget={editTarget}
              isSending={isSending}
              onCancelEdit={onCancelEdit}
              onCancelReply={composerReplyTarget ? onCancelReply : undefined}
              onEditSave={onEditSave}
              onSend={onSend}
              placeholder={`Reply in thread to ${threadHead.author}`}
              replyTarget={composerReplyTarget}
              toolbarExtraActions={toolbarExtraActions}
              typingParentEventId={threadHead.id}
              typingRootEventId={threadHead.rootId}
            />
            <div className="h-6 bg-background">
              {threadTypingPubkeys.length > 0 ? (
                <TypingIndicatorRow
                  channel={channel}
                  className="px-4 pb-1 pt-0 sm:px-6"
                  currentPubkey={currentPubkey}
                  profiles={profiles}
                  typingPubkeys={threadTypingPubkeys}
                />
              ) : null}
            </div>
          </div>
        </div>
      </aside>
    </>
  );
}
