import { Bot, Pin, X } from "lucide-react";
import * as React from "react";
import { useSyncExternalStore } from "react";

import {
  listBrowserAgentGrants,
  subscribeBrowserAgentGrant,
} from "@/features/browser-agent/lib/api";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";
import {
  closeLinkSidePanel,
  destroyLinkSidePanelIfPin,
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
import { findGrantForPlaygroundPin } from "@/features/playground/lib/pinAgentGrant";
import {
  addPlaygroundSession,
  hasPlaygroundSession,
  showPlaygroundSession,
} from "@/features/playground/lib/sessions";
import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
} from "@/features/playground/lib/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

function scopeKeyForConversation(input: {
  channelId: string;
  threadId?: string | null;
}): string {
  const threadId = input.threadId?.trim() ?? "";
  if (threadId) return `thread:${threadId}`;
  return `channel:${input.channelId}`;
}

function useBrowserAgentGrants(): BrowserAgentGrant[] {
  const [grants, setGrants] = React.useState<BrowserAgentGrant[]>([]);
  const refresh = React.useCallback(() => {
    void listBrowserAgentGrants()
      .then(setGrants)
      .catch(() => setGrants([]));
  }, []);
  React.useEffect(() => {
    refresh();
    const unlisten = subscribeBrowserAgentGrant(() => {
      refresh();
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [refresh]);
  return grants;
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
  const grants = useBrowserAgentGrants();
  const menuRequest = useSyncExternalStore(
    subscribeConversationPlaygroundPinsMenu,
    getConversationPlaygroundPinsMenuOpenRequest,
    getConversationPlaygroundPinsMenuOpenRequest,
  );
  const [open, setOpen] = React.useState(false);
  // Radix MenuItem selects on pointerup even when a nested control stopped
  // pointerdown. Mark unpin gestures so onSelect does not open the pin as a
  // chrome-less right-pane webview (Cloudflare Access soft-stuck UI).
  const suppressOpenFromUnpinRef = React.useRef(false);

  React.useEffect(() => {
    if (!menuRequest) return;
    if (menuRequest.scopeKey !== scopeKey) return;
    setOpen(true);
  }, [menuRequest, scopeKey]);

  function openPin(pin: ConversationPlaygroundPin) {
    // Conversation pins identify the same playground surface as the left-rail
    // session. Close any legacy URL pin-panel first, then reuse/add the
    // playground session in the RHS idle-auxiliary slide-out so Agent chrome
    // + Observe/Drive grants stay on playground-{sid} (not a grant-less pin
    // webview, not left dock, not windowed inset-0 cover).
    closeLinkSidePanel();
    if (hasPlaygroundSession(pin.sid)) {
      showPlaygroundSession(pin.sid, { preferSidePanel: true });
    } else {
      addPlaygroundSession(
        {
          hula: PLAYGROUND_HULA,
          v: PLAYGROUND_VERSION,
          name: pin.name,
          url: pin.url,
          sid: pin.sid,
          ...(pin.pin ? { pin: pin.pin } : {}),
          ...(pin.stack ? { stack: pin.stack } : {}),
          ...(pin.expires != null ? { expires: pin.expires } : {}),
        },
        { preferSidePanel: true },
      );
    }
    setOpen(false);
  }

  function handlePinSelect(pin: ConversationPlaygroundPin, event: Event) {
    if (suppressOpenFromUnpinRef.current) {
      suppressOpenFromUnpinRef.current = false;
      // Keep the menu open so the user can unpin more pins.
      event.preventDefault();
      return;
    }
    openPin(pin);
  }

  function unpin(pin: ConversationPlaygroundPin, event: React.SyntheticEvent) {
    event.preventDefault();
    event.stopPropagation();
    suppressOpenFromUnpinRef.current = true;
    unpinPlaygroundFromConversation(scopeKey, pin.sid);
    // Always tear down any open keep-alive surface for this pin so unpin
    // cannot leave a half-open native webview covering the channel.
    destroyLinkSidePanelIfPin(conversationPlaygroundPinWebviewId(pin.sid));
  }

  function suppressItemSelect(event: React.SyntheticEvent) {
    // pointerdown/up on X must not reach MenuItem — Radix selects on up.
    event.preventDefault();
    event.stopPropagation();
    suppressOpenFromUnpinRef.current = true;
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
          <DropdownMenuItem
            disabled
            data-testid="conversation-playground-pins-empty"
          >
            Pin a playground from a card
          </DropdownMenuItem>
        ) : (
          pins.map((pin) => {
            const grant = findGrantForPlaygroundPin(grants, pin.sid);
            const agentDriven = grant != null;
            const agentTitle =
              grant?.mode === "drive"
                ? "Agent Drive"
                : grant?.mode === "observe"
                  ? "Agent Observe"
                  : null;
            return (
              <DropdownMenuItem
                className="flex items-center gap-2"
                data-testid={`conversation-playground-pin-${pin.sid}`}
                key={pin.sid}
                onSelect={(event) => handlePinSelect(pin, event)}
              >
                {agentDriven ? (
                  <span
                    className={cn(
                      "inline-flex h-4 w-4 shrink-0 items-center justify-center rounded-sm border border-orange-500/50 text-orange-700 dark:text-orange-400",
                      grant?.mode === "drive" && "bg-orange-500/15",
                    )}
                    data-testid={`conversation-playground-pin-agent-${pin.sid}`}
                    title={agentTitle ?? "Agent attached"}
                  >
                    <Bot aria-hidden className="h-3 w-3" />
                    <span className="sr-only">
                      {agentTitle ?? "Agent attached"}
                    </span>
                  </span>
                ) : null}
                <span className="min-w-0 flex-1 truncate">{pin.name}</span>
                <button
                  aria-label={`Unpin ${pin.name}`}
                  className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                  data-testid={`conversation-playground-unpin-${pin.sid}`}
                  onClick={(event) => unpin(pin, event)}
                  onPointerDown={suppressItemSelect}
                  onPointerUp={suppressItemSelect}
                  type="button"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </DropdownMenuItem>
            );
          })
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
