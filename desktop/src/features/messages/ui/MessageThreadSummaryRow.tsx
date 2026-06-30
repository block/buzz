import * as React from "react";

import type {
  TimelineThreadSummary,
  TimelineThreadSummaryParticipant,
} from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { ThreadDepthGuideAction } from "@/features/messages/ui/MessageRow";
import { formatThreadSummaryLastReplyTime } from "@/features/messages/lib/dateFormatters";
import {
  getThreadReplyAvatarCenterRem,
  getThreadReplyIndentRem,
  threadReplyLength,
  THREAD_REPLY_BODY_OFFSET_REM,
  THREAD_REPLY_LINE_WIDTH_REM,
} from "@/features/messages/lib/threadTreeLayout";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function ParticipantAvatar({
  participant,
  index,
  participantCount,
}: {
  participant: TimelineThreadSummaryParticipant;
  index: number;
  participantCount: number;
}) {
  return (
    <div
      className={index > 0 ? "-ml-2" : ""}
      data-testid="message-thread-summary-participant"
      style={{
        zIndex: index + 1,
        ...(index < participantCount - 1 && {
          mask: "radial-gradient(circle 16px at calc(100% + 4px) 50%, transparent 99%, #fff 100%)",
          WebkitMask:
            "radial-gradient(circle 16px at calc(100% + 4px) 50%, transparent 99%, #fff 100%)",
        }),
      }}
    >
      <UserAvatar
        avatarUrl={participant.avatarUrl}
        className="h-7 w-7 text-2xs"
        displayName={participant.author}
        size="sm"
      />
    </div>
  );
}

export function MessageThreadSummaryRow({
  collapseDepthGuideActions,
  depth = 0,
  depthGuideDepths,
  highlightThreadLineDepths,
  message,
  onCollapseDepthGuide,
  onCollapseDepthGuideHoverChange,
  onOpenThread,
  showDepthGuides = true,
  summary,
  summaryIndentOffsetRem = 0,
  unreadCount,
}: {
  collapseDepthGuideActions?: ReadonlyArray<ThreadDepthGuideAction>;
  depth?: number;
  depthGuideDepths?: ReadonlyArray<number>;
  highlightThreadLineDepths?: ReadonlyArray<number>;
  message: TimelineMessage;
  onCollapseDepthGuide?: (message: TimelineMessage) => void;
  onCollapseDepthGuideHoverChange?: (
    message: TimelineMessage,
    hovered: boolean,
  ) => void;
  onOpenThread?: (message: TimelineMessage) => void;
  showDepthGuides?: boolean;
  summary: TimelineThreadSummary;
  summaryIndentOffsetRem?: number;
  unreadCount?: number;
}) {
  const indentRem = getThreadReplyIndentRem(depth);
  const marginLeftRem =
    indentRem + THREAD_REPLY_BODY_OFFSET_REM + summaryIndentOffsetRem;
  const replyLabel = summary.replyCount === 1 ? "reply" : "replies";
  const summaryAriaLabel = summary.lastReplyAt
    ? `View thread with ${summary.replyCount} ${replyLabel}, last reply ${formatThreadSummaryLastReplyTime(summary.lastReplyAt)}`
    : `View thread with ${summary.replyCount} ${replyLabel}`;
  const guideDepths = depthGuideDepths
    ? [...depthGuideDepths]
    : Array.from({ length: Math.max(0, depth - 1) }, (_, index) => index + 1);
  const depthGuideItems = guideDepths.map((guideDepth) => ({
    depth: guideDepth,
    offset: getThreadReplyAvatarCenterRem(guideDepth),
  }));
  const collapseDepthGuideActionsByDepth = new Map(
    collapseDepthGuideActions?.map((action) => [action.depth, action]) ?? [],
  );

  return (
    <div className="relative pb-1 pt-0.5">
      {showDepthGuides && depthGuideItems.length > 0 ? (
        <div
          aria-hidden={
            collapseDepthGuideActionsByDepth.size > 0 ? undefined : true
          }
          className={cn(
            "absolute left-0",
            collapseDepthGuideActionsByDepth.size === 0 &&
              "pointer-events-none",
          )}
          style={{ bottom: "-0.25rem", top: "-0.25rem" }}
        >
          {depthGuideItems.map(({ depth: guideDepth, offset }) => {
            const collapseAction =
              collapseDepthGuideActionsByDepth.get(guideDepth);
            const isHighlighted =
              Boolean(collapseAction?.active) ||
              Boolean(highlightThreadLineDepths?.includes(guideDepth));
            if (collapseAction) {
              return (
                <React.Fragment
                  key={`${message.id}-summary-depth-guide-${offset}`}
                >
                  <div
                    aria-hidden
                    className={cn(
                      "pointer-events-none absolute bottom-0 top-0 border-l transition-[border-color]",
                      isHighlighted ? "border-primary" : "border-border/45",
                    )}
                    style={{
                      borderLeftWidth: threadReplyLength(
                        THREAD_REPLY_LINE_WIDTH_REM,
                      ),
                      left: threadReplyLength(offset),
                    }}
                  />
                  <button
                    aria-label={collapseAction.label}
                    className="absolute bottom-0 top-0 z-20 w-5 -translate-x-1/2 cursor-pointer rounded-full focus-visible:outline-hidden"
                    data-thread-head-id={collapseAction.message.id}
                    data-testid="thread-collapse-guide"
                    onBlur={() =>
                      onCollapseDepthGuideHoverChange?.(
                        collapseAction.message,
                        false,
                      )
                    }
                    onClick={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      onCollapseDepthGuide?.(collapseAction.message);
                    }}
                    onFocus={() =>
                      onCollapseDepthGuideHoverChange?.(
                        collapseAction.message,
                        true,
                      )
                    }
                    onMouseEnter={() =>
                      onCollapseDepthGuideHoverChange?.(
                        collapseAction.message,
                        true,
                      )
                    }
                    onMouseLeave={() =>
                      onCollapseDepthGuideHoverChange?.(
                        collapseAction.message,
                        false,
                      )
                    }
                    style={{ left: threadReplyLength(offset) }}
                    type="button"
                  />
                </React.Fragment>
              );
            }

            return (
              <div
                aria-hidden
                className={cn(
                  "pointer-events-none absolute bottom-0 top-0 border-l transition-[border-color]",
                  isHighlighted ? "border-primary" : "border-border/45",
                )}
                key={`${message.id}-summary-depth-guide-${offset}`}
                style={{
                  borderLeftWidth: threadReplyLength(
                    THREAD_REPLY_LINE_WIDTH_REM,
                  ),
                  left: threadReplyLength(offset),
                }}
              />
            );
          })}
        </div>
      ) : null}

      <button
        aria-label={summaryAriaLabel}
        className="group relative isolate inline-flex h-8 w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-full text-left text-xs font-medium text-muted-foreground transition-[color,opacity] before:pointer-events-none before:absolute before:-bottom-0.5 before:-left-0.5 before:-right-2 before:-top-0.5 before:-z-10 before:rounded-full before:content-[''] before:transition-[background-color,box-shadow] hover:text-foreground hover:opacity-90 hover:before:bg-background/95 hover:before:ring-1 hover:before:ring-border/70 focus-visible:outline-hidden focus-visible:before:bg-background/95 focus-visible:before:ring-1 focus-visible:before:ring-ring"
        data-thread-head-id={message.id}
        data-testid="message-thread-summary"
        onClick={() => onOpenThread?.(message)}
        style={{ marginLeft: threadReplyLength(marginLeftRem) }}
        type="button"
      >
        <div className="ml-0.5 flex shrink-0 items-center">
          {summary.participants.map((participant, index) => (
            <ParticipantAvatar
              index={index}
              key={participant.id}
              participant={participant}
              participantCount={summary.participants.length}
            />
          ))}
        </div>
        <div className="min-w-0">
          <div>
            <span className="font-medium transition-colors group-hover:text-foreground">
              {summary.replyCount} {replyLabel}
            </span>
            {unreadCount != null && unreadCount > 0 ? (
              <span className="ml-1" data-testid="thread-unread-badge">
                ({unreadCount} new)
              </span>
            ) : null}
            {summary.lastReplyAt ? (
              <>
                <span className="mx-1 font-normal text-muted-foreground/50">
                  ·
                </span>
                <span className="inline-grid font-normal text-muted-foreground/70">
                  <span
                    className="col-start-1 row-start-1 transition-opacity group-hover:opacity-0 group-focus-visible:opacity-0"
                    data-testid="message-thread-summary-last-reply"
                  >
                    last reply{" "}
                    {formatThreadSummaryLastReplyTime(summary.lastReplyAt)}
                  </span>
                  <span
                    className="col-start-1 row-start-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
                    data-testid="message-thread-summary-hover-action"
                  >
                    View thread
                  </span>
                </span>
              </>
            ) : null}
          </div>
        </div>
      </button>
    </div>
  );
}
