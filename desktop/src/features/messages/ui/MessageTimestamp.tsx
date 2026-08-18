import {
  formatFullDateTime,
  formatTime,
  formatTimeWithoutDayPeriod,
} from "@/features/messages/lib/dateFormatters";
import { cn } from "@/shared/lib/cn";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/**
 * The timestamp beside a message author, and the clock that fades in over the
 * avatar gutter on continuation rows.
 *
 * Both labels are derived from `createdAt` here rather than taken as a
 * pre-formatted string, so the wording is recomputed on each render instead of
 * being frozen at the time the message list was formatted. Note this does not
 * make it live: `MessageRow` is memoized, so a row already on screen when the
 * clock passes midnight keeps saying "Today" until something re-renders it. The
 * day divider above it has the same property, and both correct themselves on the
 * next message, scroll, or navigation.
 *
 * The two modes carry different information on purpose:
 *
 * - Header (default) — the full relative label ("Yesterday at 9:05 AM"). The day
 *   divider above the group says which day it is, but a divider scrolls out of
 *   view while its messages stay on screen, so a bare clock time on a row from
 *   last week has nothing to anchor it.
 * - `hideDayPeriod` — clock only, minus the AM/PM marker. This renders in a
 *   36px-wide gutter where the avatar would be, so it has room for "9:05" and
 *   nothing more.
 */
export function MessageTimestamp({
  className,
  createdAt,
  hideDayPeriod = false,
}: {
  className?: string;
  createdAt: number;
  hideDayPeriod?: boolean;
}) {
  const displayTime = hideDayPeriod
    ? formatTimeWithoutDayPeriod(formatTime(createdAt))
    : formatItemTimestamp(createdAt, { withTime: true });

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <p
          className={cn(
            "shrink-0 cursor-default whitespace-nowrap text-xs font-normal leading-4 tabular-nums text-muted-foreground/55",
            className,
          )}
          data-testid="message-timestamp"
        >
          {displayTime}
        </p>
      </TooltipTrigger>
      <TooltipContent side="top">
        {formatFullDateTime(createdAt)}
      </TooltipContent>
    </Tooltip>
  );
}
