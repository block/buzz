import { formatFullDateTime } from "@/features/messages/lib/dateFormatters";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function MessageTimestamp({
  createdAt,
  time,
}: {
  createdAt: number;
  time: string;
}) {
  return (
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
  );
}
