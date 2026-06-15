import type {
  TimelineThreadSummary,
  TimelineThreadSummaryParticipant,
} from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { formatThreadSummaryLastReplyTime } from "@/features/messages/lib/dateFormatters";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MESSAGE_TEXT_OFFSET_PX = 54;
const MESSAGE_BODY_OFFSET_PX = MESSAGE_TEXT_OFFSET_PX + 4;
const NESTED_REPLY_OFFSET_PX = 28;

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
        className="h-7 w-7 text-[10px]"
        displayName={participant.author}
        size="sm"
      />
    </div>
  );
}

export function MessageThreadSummaryRow({
  depth = 0,
  message,
  onOpenThread,
  showDepthGuides = true,
  summary,
}: {
  depth?: number;
  message: TimelineMessage;
  onOpenThread: (message: TimelineMessage) => void;
  showDepthGuides?: boolean;
  summary: TimelineThreadSummary;
}) {
  const visibleDepth = Math.min(Math.max(depth, 0), 6);
  const indentPx =
    visibleDepth > 0
      ? MESSAGE_TEXT_OFFSET_PX + (visibleDepth - 1) * NESTED_REPLY_OFFSET_PX
      : 0;
  const marginLeftPx = indentPx + MESSAGE_BODY_OFFSET_PX;
  const replyLabel = summary.replyCount === 1 ? "reply" : "replies";
  const summaryAriaLabel = summary.lastReplyAt
    ? `View thread with ${summary.replyCount} ${replyLabel}, last reply ${formatThreadSummaryLastReplyTime(summary.lastReplyAt)}`
    : `View thread with ${summary.replyCount} ${replyLabel}`;
  const depthGuideOffsets =
    visibleDepth === 0
      ? []
      : Array.from({ length: visibleDepth }, (_, index) =>
          index === 0
            ? MESSAGE_TEXT_OFFSET_PX / 2
            : MESSAGE_TEXT_OFFSET_PX +
              NESTED_REPLY_OFFSET_PX / 2 +
              (index - 1) * NESTED_REPLY_OFFSET_PX,
        );

  return (
    <div className="relative pb-1 pt-0.5">
      {showDepthGuides && depthGuideOffsets.length > 0 ? (
        <div
          aria-hidden
          className="pointer-events-none absolute left-0"
          style={{ bottom: "-4px", top: "-4px" }}
        >
          {depthGuideOffsets.map((offset, index) => (
            <div
              className="absolute bottom-0 top-0 border-l border-border/70"
              key={`${message.id}-summary-depth-guide-${offset}`}
              style={{
                left: `${offset}px`,
                opacity: index === depthGuideOffsets.length - 1 ? 0.9 : 0.55,
              }}
            />
          ))}
        </div>
      ) : null}

      <button
        aria-label={summaryAriaLabel}
        className="group relative isolate inline-flex h-8 w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-full text-left text-xs font-medium text-muted-foreground transition-[color,opacity] before:pointer-events-none before:absolute before:-bottom-0.5 before:-left-0.5 before:-right-2 before:-top-0.5 before:-z-10 before:rounded-full before:content-[''] before:transition-[background-color,box-shadow] hover:text-foreground hover:opacity-90 hover:before:bg-background/95 hover:before:ring-1 hover:before:ring-border/70 focus-visible:outline-hidden focus-visible:before:bg-background/95 focus-visible:before:ring-1 focus-visible:before:ring-ring"
        data-thread-head-id={message.id}
        data-testid="message-thread-summary"
        onClick={() => onOpenThread(message)}
        style={{ marginLeft: `${marginLeftPx}px` }}
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
