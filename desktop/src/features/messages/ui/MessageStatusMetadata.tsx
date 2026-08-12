import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function MessageStatusMetadata({
  edited,
  pending,
}: {
  edited?: boolean;
  pending?: boolean;
}) {
  return (
    <>
      {pending ? (
        <p
          className="font-normal text-muted-foreground/70"
          data-testid="message-send-status"
        >
          Sending…
        </p>
      ) : null}
      {edited ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <p className="text-muted-foreground/70">(edited)</p>
          </TooltipTrigger>
          <TooltipContent>This message has been edited</TooltipContent>
        </Tooltip>
      ) : null}
    </>
  );
}
