import type {
  TimelineThreadSummary,
  TimelineThreadSummaryParticipant,
} from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MESSAGE_TEXT_OFFSET_PX = 54;
const NESTED_REPLY_OFFSET_PX = 28;

function formatRelativeTime(unixSeconds: number): string {
  const now = Date.now() / 1_000;
  const diff = now - unixSeconds;

  if (diff < 60) return "just now";
  if (diff < 3_600) return `${Math.floor(diff / 60)}m`;
  if (diff < 86_400) return `${Math.floor(diff / 3_600)}h`;
  if (diff < 604_800) return `${Math.floor(diff / 86_400)}d`;

  return new Date(unixSeconds * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function ParticipantAvatar({
  participant,
  index,
}: {
  participant: TimelineThreadSummaryParticipant;
  index: number;
}) {
  return (
    <div
      className={index > 0 ? "-ml-2" : ""}
      data-testid="message-thread-summary-participant"
      style={{ zIndex: 10 - index }}
    >
      <UserAvatar
        avatarUrl={participant.avatarUrl}
        className="rounded-full border-2 border-background"
        displayName={participant.author}
        size="xs"
      />
    </div>
  );
}

export function MessageThreadSummaryRow({
  depth = 0,
  message,
  onOpenThread,
  summary,
}: {
  depth?: number;
  message: TimelineMessage;
  onOpenThread: (message: TimelineMessage) => void;
  summary: TimelineThreadSummary;
}) {
  const visibleDepth = Math.min(Math.max(depth, 0), 6);
  const indentPx =
    visibleDepth > 0
      ? MESSAGE_TEXT_OFFSET_PX + (visibleDepth - 1) * NESTED_REPLY_OFFSET_PX
      : 0;
  const marginLeftPx = indentPx + MESSAGE_TEXT_OFFSET_PX;
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
    <div className="relative pb-1 pt-1">
      {depthGuideOffsets.length > 0 ? (
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
        className="group relative inline-flex w-fit max-w-full cursor-pointer items-center gap-1 rounded-full text-left text-xs font-medium text-muted-foreground transition-[color,opacity] before:pointer-events-none before:absolute before:-inset-y-0.5 before:-left-0.5 before:-right-1.5 before:rounded-full before:content-[''] before:transition-shadow hover:text-foreground hover:opacity-90 hover:before:ring-1 hover:before:ring-border/70 focus-visible:outline-hidden focus-visible:before:ring-1 focus-visible:before:ring-ring"
        data-thread-head-id={message.id}
        data-testid="message-thread-summary"
        onClick={() => onOpenThread(message)}
        style={{ marginLeft: `${marginLeftPx}px` }}
        type="button"
      >
        <div className="flex shrink-0 items-center">
          {summary.participants.map((participant, index) => (
            <ParticipantAvatar
              index={index}
              key={participant.id}
              participant={participant}
            />
          ))}
        </div>
        <div className="min-w-0">
          <div className="font-medium">
            <span className="transition-colors group-hover:text-foreground">
              {summary.replyCount}{" "}
              {summary.replyCount === 1 ? "reply" : "replies"}
            </span>
            {summary.lastReplyAt ? (
              <span className="ml-1 text-muted-foreground/70">
                last {formatRelativeTime(summary.lastReplyAt)}
              </span>
            ) : null}
          </div>
        </div>
      </button>
    </div>
  );
}
