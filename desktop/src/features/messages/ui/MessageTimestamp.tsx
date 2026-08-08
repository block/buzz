import {
  formatFullDateTime,
  formatMessageTimestamp,
  formatTimeWithoutDayPeriod,
} from "@/features/messages/lib/dateFormatters";
import { cn } from "@/shared/lib/cn";
import { useMinuteNow } from "@/shared/lib/useMinuteNow";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

const TIMESTAMP_TOOLTIP_DELAY_MS = 500;

export function MessageTimestamp({
  className,
  createdAt,
  hideDayPeriod = false,
  time,
}: {
  className?: string;
  createdAt: number;
  hideDayPeriod?: boolean;
  time: string;
}) {
  const now = useMinuteNow();
  const displayTime = hideDayPeriod
    ? formatTimeWithoutDayPeriod(time)
    : formatMessageTimestamp(createdAt, now / 1_000);

  return (
    <TooltipProvider
      delayDuration={TIMESTAMP_TOOLTIP_DELAY_MS}
      skipDelayDuration={0}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <p
            className={cn(
              "shrink-0 cursor-default whitespace-nowrap text-2xs font-normal leading-4 tabular-nums text-muted-foreground",
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
    </TooltipProvider>
  );
}
