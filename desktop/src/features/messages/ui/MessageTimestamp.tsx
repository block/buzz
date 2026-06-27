import { formatFullDateTime } from "@/features/messages/lib/dateFormatters";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

const TIMESTAMP_TOOLTIP_DELAY_MS = 500;

export function MessageTimestamp({
  createdAt,
  time,
}: {
  createdAt: number;
  time: string;
}) {
  return (
    <TooltipProvider
      delayDuration={TIMESTAMP_TOOLTIP_DELAY_MS}
      skipDelayDuration={0}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <p className="shrink-0 cursor-default whitespace-nowrap text-xs font-normal leading-4 tabular-nums text-muted-foreground/55">
            {time}
          </p>
        </TooltipTrigger>
        <TooltipContent side="top">
          {formatFullDateTime(createdAt)}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
