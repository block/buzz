import { AppWindow, Columns2 } from "lucide-react";
import { toast } from "sonner";

import {
  openPopoutWindow,
  popoutErrorMessage,
} from "@/features/popout/lib/popoutWindow";
import { listConversationPlaygroundPins } from "@/features/playground/lib/conversationPins";
import type { PlaygroundCard } from "@/features/playground/lib/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

function pinAsCard(session: {
  sid: string;
  name: string;
  url: string;
  pin?: string;
  stack?: string;
  expires?: string;
}): PlaygroundCard {
  return {
    hula: "playground",
    v: 1,
    name: session.name,
    url: session.url,
    sid: session.sid,
    ...(session.pin ? { pin: session.pin } : {}),
    ...(session.stack ? { stack: session.stack } : {}),
    ...(session.expires != null ? { expires: session.expires } : {}),
  };
}

function playgroundPinsScopeKey(
  channelId: string,
  threadId?: string | null,
): string {
  const trimmedThread = threadId?.trim() ?? "";
  if (trimmedThread) return `thread:${trimmedThread}`;
  return `channel:${channelId}`;
}

export function ConversationPopoutMenu({
  channelId,
  threadId,
}: {
  channelId: string;
  threadId?: string | null;
}) {
  const playgrounds = listConversationPlaygroundPins(
    playgroundPinsScopeKey(channelId, threadId),
  ).map(pinAsCard);

  async function openPlain() {
    try {
      await openPopoutWindow({
        kind: "thread",
        title: threadId ? "Thread" : "Channel",
        seed: threadId ? `${channelId}-${threadId}` : channelId,
        channelId,
        ...(threadId ? { threadId } : {}),
      });
    } catch (error) {
      toast.error(popoutErrorMessage(error, "Could not open window."));
    }
  }

  async function openSplit(playground: PlaygroundCard) {
    try {
      await openPopoutWindow({
        kind: "split",
        title: playground.name,
        seed: `${playground.sid}-${channelId}-${threadId ?? "channel"}`,
        channelId,
        ...(threadId ? { threadId } : {}),
        playground,
      });
    } catch (error) {
      toast.error(popoutErrorMessage(error, "Could not open split window."));
    }
  }

  return (
    <DropdownMenu>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Open in a new window"
              className="shrink-0"
              data-testid="conversation-popout-menu"
              size="icon"
              type="button"
              variant="ghost"
            >
              <AppWindow />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>Open in a new window</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuItem
          data-testid="conversation-popout-plain"
          onSelect={() => void openPlain()}
        >
          <AppWindow />
          Open in a new window
        </DropdownMenuItem>
        {playgrounds.length > 0 ? (
          <DropdownMenuSub>
            <DropdownMenuSubTrigger data-testid="conversation-popout-split">
              <Columns2 />
              Open with anchored playground
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent className="w-56">
              {playgrounds.map((playground) => (
                <DropdownMenuItem
                  data-testid={`conversation-popout-split-${playground.sid}`}
                  key={playground.sid}
                  onSelect={() => void openSplit(playground)}
                >
                  {playground.name}
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        ) : (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled>
              Pin a playground to split with one
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
