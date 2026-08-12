import * as React from "react";

import {
  threadReplyLength,
  THREAD_REPLY_LINE_WIDTH_REM,
} from "@/features/messages/lib/threadTreeLayout";
import type { TimelineMessage } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";

type DepthGuideAction = {
  active?: boolean;
  depth: number;
  label: string;
  message: TimelineMessage;
};

export function MessageDepthGuides({
  collapseDepthGuideActionsByDepth,
  depthGuideItems,
  guideBleedRem,
  highlightThreadLineDepths,
  messageId,
  onCollapse,
  onHoverChange,
}: {
  collapseDepthGuideActionsByDepth: ReadonlyMap<number, DepthGuideAction>;
  depthGuideItems: ReadonlyArray<{ depth: number; offset: number }>;
  guideBleedRem: number;
  highlightThreadLineDepths?: ReadonlyArray<number>;
  messageId: string;
  onCollapse: (
    event: React.MouseEvent<HTMLButtonElement>,
    message: TimelineMessage,
  ) => void;
  onHoverChange: (message: TimelineMessage, active: boolean) => void;
}) {
  return (
    <div
      aria-hidden={collapseDepthGuideActionsByDepth.size > 0 ? undefined : true}
      className={cn(
        "absolute left-0",
        collapseDepthGuideActionsByDepth.size === 0 && "pointer-events-none",
      )}
      style={{
        bottom: threadReplyLength(-guideBleedRem),
        top: threadReplyLength(-guideBleedRem),
      }}
    >
      {depthGuideItems.map(({ depth, offset }) => {
        const collapseAction = collapseDepthGuideActionsByDepth.get(depth);
        const isHighlighted =
          Boolean(collapseAction?.active) ||
          Boolean(highlightThreadLineDepths?.includes(depth));
        if (collapseAction) {
          return (
            <React.Fragment key={`${messageId}-depth-guide-${offset}`}>
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
                onBlur={() => onHoverChange(collapseAction.message, false)}
                onClick={(event) => onCollapse(event, collapseAction.message)}
                onFocus={() => onHoverChange(collapseAction.message, true)}
                onMouseEnter={() => onHoverChange(collapseAction.message, true)}
                onMouseLeave={() =>
                  onHoverChange(collapseAction.message, false)
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
            key={`${messageId}-depth-guide-${offset}`}
            style={{
              borderLeftWidth: threadReplyLength(THREAD_REPLY_LINE_WIDTH_REM),
              left: threadReplyLength(offset),
            }}
          />
        );
      })}
    </div>
  );
}
