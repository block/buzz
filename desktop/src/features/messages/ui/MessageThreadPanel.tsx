import * as React from "react";
import { ArrowDown, ArrowLeft, X } from "lucide-react";

import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { MessageComposer } from "./MessageComposer";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { TypingIndicatorRow } from "./TypingIndicatorRow";
import { useComposerHeightPadding } from "./useComposerHeightPadding";
import { useTimelineScrollManager } from "./useTimelineScrollManager";

type MessageThreadPanelProps = {
  canResetWidth: boolean;
  channel: Channel | null;
  channelId: string | null;
  channelName: string;
  currentPubkey?: string;
  disabled?: boolean;
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  isSending: boolean;
  isSinglePanelView?: boolean;
  onCancelEdit?: () => void;
  onCancelReply: () => void;
  onClose: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditLastOwnMessage?: () => boolean;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onMarkUnread?: (message: TimelineMessage) => void;
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
  replyTargetMessage: TimelineMessage | null;
  scrollTargetId: string | null;
  threadHead: TimelineMessage | null;
  threadReplies: MainTimelineEntry[];
  threadTypingPubkeys: string[];
  toolbarExtraActions?: React.ReactNode;
  widthPx: number;
  isFollowingThread?: boolean;
  onFollowThread?: () => void;
  onUnfollowThread?: () => void;
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
  isSinglePanelView = false,
  isFollowingThread,
  onCancelEdit,
  onCancelReply,
  onClose,
  onDelete,
  onEdit,
  onEditLastOwnMessage,
  onEditSave,
  onFollowThread,
  onMarkUnread,
  onExpandReplies,
  onResetWidth,
  onResizeStart,
  onScrollTargetResolved,
  onSelectReplyTarget,
  onSend,
  onToggleReaction,
  onUnfollowThread,
  profiles,
  replyTargetMessage,
  scrollTargetId,
  threadHead,
  threadReplies,
  threadTypingPubkeys,
  toolbarExtraActions,
  widthPx,
}: MessageThreadPanelProps) {
  const threadBodyRef = React.useRef<HTMLDivElement>(null);
  const threadComposerWrapperRef = React.useRef<HTMLDivElement>(null);
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const usesChannelSplitChrome = !isOverlay && !isSinglePanelView;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);
  useComposerHeightPadding(
    threadBodyRef,
    threadComposerWrapperRef,
    isSinglePanelView,
  );

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
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="message-thread-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`,
        }}
      >
        {!isOverlay && !isSinglePanelView && (
          <button
            aria-label="Resize thread panel"
            className="peer/thread-resize group/thread-resize absolute inset-y-0 left-0 z-40 w-3 -translate-x-1/2 cursor-col-resize"
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
            <span className="absolute bottom-0 left-1/2 top-10 w-px -translate-x-1/2 bg-transparent transition-colors group-hover/thread-resize:bg-border/80 group-focus-visible/thread-resize:bg-border/80" />
          </button>
        )}

        {!isOverlay ? (
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-x-0 top-0 z-40 bg-transparent after:absolute after:bottom-0 after:-left-px after:top-10 after:w-px after:bg-border/45 after:transition-colors peer-hover/thread-resize:after:bg-border/80 peer-focus-visible/thread-resize:after:bg-border/80",
              usesChannelSplitChrome ? "h-[92px]" : "h-[76px]",
            )}
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[76px] min-h-[76px] shrink-0 gap-[10px] bg-background/80 pb-[3px] pl-[16px] pr-[8px] pt-[43px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 sm:pr-[12px] dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55`
              : isOverlay
                ? "relative z-50 min-h-[44px] shrink-0 gap-3 bg-background/80 px-3 py-[6px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
                : "absolute inset-x-0 top-[48px] z-50 h-[32px] gap-[10px] py-0 pl-[16px] pr-[8px] sm:pr-[12px]",
          )}
          data-tauri-drag-region
        >
          <div
            className={cn(
              "flex min-w-0 items-center",
              isSinglePanelView ? "gap-[4px]" : "gap-1.5",
            )}
          >
            {isSinglePanelView ? (
              <div className="relative h-[14px] w-[14px] shrink-0">
                <Button
                  aria-label="Back to conversation"
                  className="absolute left-1/2 top-1/2 h-6 w-6 -translate-x-1/2 -translate-y-1/2 text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                  data-testid="message-thread-back"
                  onClick={onClose}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <ArrowLeft className="h-3.5 w-3.5" />
                </Button>
              </div>
            ) : null}
            <h2 className="translate-y-px text-base font-semibold leading-6 tracking-tight">
              Thread
            </h2>
          </div>
          <Button
            aria-label="Close thread"
            className="ml-auto h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
            data-testid="message-thread-close"
            onClick={onClose}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X className="size-5" />
          </Button>
        </div>

        <div
          className={cn(
            "min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain pb-24 [overflow-anchor:none]",
            usesChannelSplitChrome ? "pt-[92px]" : isOverlay ? "" : "pt-[76px]",
          )}
          data-testid="message-thread-body"
          onScroll={syncScrollState}
          ref={threadBodyRef}
        >
          <div ref={contentRef}>
            <div className="px-3 pb-1 pt-0" data-testid="message-thread-head">
              <div className="rounded-2xl">
                <MessageRow
                  channelId={channelId}
                  isFollowingThread={isFollowingThread}
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
                  onFollowThread={
                    onFollowThread ? (_msg) => onFollowThread() : undefined
                  }
                  onMarkUnread={onMarkUnread}
                  onToggleReaction={onToggleReaction}
                  onUnfollowThread={
                    onUnfollowThread ? (_msg) => onUnfollowThread() : undefined
                  }
                  profiles={profiles}
                />
              </div>
            </div>

            <div
              className="px-3 pb-3 pt-1"
              data-testid="message-thread-replies"
            >
              {threadReplies.length > 0 ? (
                <div className="space-y-2.5">
                  {threadReplies.map((entry) => {
                    return (
                      <div
                        className={cn(
                          "flex flex-col gap-1",
                          entry.summary &&
                            "group/message -mx-1 rounded-2xl px-1 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
                        )}
                        key={entry.message.id}
                      >
                        <MessageRow
                          channelId={channelId}
                          hoverBackground={!entry.summary}
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
                          onMarkUnread={onMarkUnread}
                          onReply={onSelectReplyTarget}
                          onToggleReaction={onToggleReaction}
                          profiles={profiles}
                        />
                        {entry.summary ? (
                          <MessageThreadSummaryRow
                            depth={entry.message.depth}
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
              className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-[11px] font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-3.5"
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

        <div
          className="pointer-events-none absolute inset-x-0 bottom-0 z-10"
          ref={threadComposerWrapperRef}
        >
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
              onEditLastOwnMessage={onEditLastOwnMessage}
              onEditSave={onEditSave}
              onSend={onSend}
              placeholder={`Reply in thread to ${threadHead.author}`}
              profiles={profiles}
              replyTarget={composerReplyTarget}
              typingParentEventId={threadHead.id}
              typingRootEventId={threadHead.rootId}
            />
            <div className="h-7 bg-background px-4 pb-1 pt-0 sm:px-6 -mt-1">
              <div className="mx-auto flex h-full w-full max-w-4xl items-center gap-2">
                {toolbarExtraActions ? (
                  <div className="shrink-0">{toolbarExtraActions}</div>
                ) : null}
                {threadTypingPubkeys.length > 0 ? (
                  <TypingIndicatorRow
                    channel={channel}
                    className="min-w-0 flex-1 px-0 py-0"
                    currentPubkey={currentPubkey}
                    profiles={profiles}
                    typingPubkeys={threadTypingPubkeys}
                    variant="activity"
                  />
                ) : null}
              </div>
            </div>
          </div>
        </div>
      </aside>
    </>
  );
}
