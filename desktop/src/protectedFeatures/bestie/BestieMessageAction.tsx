import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { BestiePopover, BestieTriggerVisual } from "./BestiePopover";
import { upsertBestieTrackedThreadForScope } from "./bestieThreadStore";
import { useBestie } from "./useBestie";

export function BestieMessageAction({
  channelId,
  message,
}: {
  channelId?: string | null;
  message: TimelineMessage;
}) {
  const bestie = useBestie();
  const [open, setOpen] = React.useState(false);

  // Enroll the thread whenever Ask Assistant opens on a message (start or anytime).
  React.useEffect(() => {
    if (!open || !channelId || !bestie.assignedAgent || !bestie.ownerPubkey) {
      return;
    }
    const relayUrl = bestie.relayUrl;
    if (!relayUrl) return;
    const rootEventId = message.rootId ?? message.id;
    upsertBestieTrackedThreadForScope(
      {
        agentPubkey: normalizePubkey(bestie.assignedAgent.pubkey),
        ownerPubkey: normalizePubkey(bestie.ownerPubkey),
        relayUrl,
      },
      {
        authorName: message.author ?? null,
        channelId,
        preview: message.body ?? "",
        rootEventId,
        source: "ask",
      },
    );
  }, [
    bestie.assignedAgent,
    bestie.ownerPubkey,
    bestie.relayUrl,
    channelId,
    message.author,
    message.body,
    message.id,
    message.rootId,
    open,
  ]);

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label="Ask Assistant about this message"
              className="h-8 w-8 rounded-full p-0"
              data-testid={`bestie-message-${message.id}`}
              size="sm"
              type="button"
              variant="ghost"
            >
              <BestieTriggerVisual agent={bestie.assignedAgent} compact />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Ask Assistant</TooltipContent>
      </Tooltip>
      <PopoverContent align="end" className="w-80" side="top" sideOffset={10}>
        <BestiePopover
          contextChannelId={channelId}
          contextMessage={message}
          onRequestClose={() => setOpen(false)}
        />
      </PopoverContent>
    </Popover>
  );
}
