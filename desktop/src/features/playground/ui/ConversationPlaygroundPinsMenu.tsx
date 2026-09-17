import { Pin, X } from "lucide-react";
import * as React from "react";
import { useSyncExternalStore } from "react";

import {
  destroyLinkSidePanelIfPin,
  openLinkSidePanel,
} from "@/features/link-panel/lib/linkSidePanelStore";
import {
  conversationPlaygroundPinWebviewId,
  getConversationPlaygroundPinsMenuOpenRequest,
  listConversationPlaygroundPins,
  subscribeConversationPlaygroundPins,
  subscribeConversationPlaygroundPinsMenu,
  unpinPlaygroundFromConversation,
  type ConversationPlaygroundPin,
} from "@/features/playground/lib/conversationPins";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

function scopeKeyForConversation(input: {
  channelId: string;
  threadId?: string | null;
}): string {
  const threadId = input.threadId?.trim() ?? "";
  if (threadId) return `thread:${threadId}`;
  return `channel:${input.channelId}`;
}

export function ConversationPlaygroundPinsMenu({
  channelId,
  threadId,
}: {
  channelId: string;
  threadId?: string | null;
}) {
  const scopeKey = scopeKeyForConversation({ channelId, threadId });
  const pins = useSyncExternalStore(
    subscribeConversationPlaygroundPins,
    () => listConversationPlaygroundPins(scopeKey),
    () => listConversationPlaygroundPins(scopeKey),
  );
  const menuRequest = useSyncExternalStore(
    subscribeConversationPlaygroundPinsMenu,
    getConversationPlaygroundPinsMenuOpenRequest,
    getConversationPlaygroundPinsMenuOpenRequest,
  );
  const [open, setOpen] = React.useState(false);

  React.useEffect(() => {
    if (!menuRequest) return;
    if (menuRequest.scopeKey !== scopeKey) return;
    setOpen(true);
  }, [menuRequest, scopeKey]);

  function openPin(pin: ConversationPlaygroundPin) {
    openLinkSidePanel(pin.url, {
      title: pin.name,
      pinId: conversationPlaygroundPinWebviewId(pin.sid),
      keepAlive: true,
    });
    setOpen(false);
  }

  function unpin(pin: ConversationPlaygroundPin, event: React.MouseEvent) {
    event.preventDefault();
    event.stopPropagation();
    unpinPlaygroundFromConversation(scopeKey, pin.sid);
    destroyLinkSidePanelIfPin(conversationPlaygroundPinWebviewId(pin.sid));
  }

  return (
    <DropdownMenu onOpenChange={setOpen} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label={
                pins.length > 0
                  ? `Playground pins (${pins.length})`
                  : "Playground pins"
              }
              className="relative shrink-0"
              data-testid="conversation-playground-pins-menu"
              size="icon"
              type="button"
              variant="ghost"
            >
              <Pin />
              {pins.length > 0 ? (
                <span
                  className="absolute -right-1 -top-1 flex h-3.5 min-w-3.5 items-center justify-center rounded-full border border-border bg-background px-0.5 text-2xs font-bold text-muted-foreground"
                  data-testid="conversation-playground-pins-badge"
                >
                  {pins.length}
                </span>
              ) : null}
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>Playground pins</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-64">
        <DropdownMenuLabel>Playground pins</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {pins.length === 0 ? (
          <DropdownMenuItem disabled data-testid="conversation-playground-pins-empty">
            Pin a playground from a card
          </DropdownMenuItem>
        ) : (
          pins.map((pin) => (
            <DropdownMenuItem
              className="flex items-center gap-2"
              data-testid={`conversation-playground-pin-${pin.sid}`}
              key={pin.sid}
              onSelect={() => openPin(pin)}
            >
              <span className="min-w-0 flex-1 truncate">{pin.name}</span>
              <span
                aria-label={`Unpin ${pin.name}`}
                className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                data-testid={`conversation-playground-unpin-${pin.sid}`}
                onClick={(event) => unpin(pin, event)}
                onPointerDown={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                }}
                role="button"
              >
                <X className="h-3.5 w-3.5" />
              </span>
            </DropdownMenuItem>
          ))
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

