import { MessageCircle, MoveUpRight } from "lucide-react";

import type { ChatLinkCardProps } from "./types";
import { cn } from "@/shared/lib/cn";

export function ChatLinkCard({
  channels,
  href,
  interactive,
  link,
  onOpenChatLink,
}: ChatLinkCardProps) {
  const channel = channels.find((c) => c.id === link.chatId);
  const title = link.title || channel?.name || "Side conversation";
  const shortId = link.chatId.slice(0, 8);

  const content = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-background text-muted-foreground">
        <MessageCircle className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold leading-5 text-foreground">
          {title}
        </span>
        <span className="block truncate text-xs leading-4 text-muted-foreground">
          Open shared chat · {shortId}
        </span>
      </span>
      {interactive ? (
        <span className="relative z-20 flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors group-hover/chat-card:bg-muted group-hover/chat-card:text-foreground">
          <MoveUpRight className="h-3.5 w-3.5" />
        </span>
      ) : null}
    </>
  );

  if (!interactive) {
    return (
      <span
        className="my-1 flex w-full min-w-0 max-w-xl items-center gap-3 overflow-hidden rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5 text-left"
        data-chat-link-card=""
        title={href}
      >
        {content}
      </span>
    );
  }

  return (
    <button
      aria-label={`Open chat ${title}`}
      className={cn(
        "group/chat-card my-1 flex w-full min-w-0 max-w-xl cursor-pointer items-center gap-3 overflow-hidden rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5 text-left transition-colors",
        "hover:border-border hover:bg-muted/50 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
      )}
      data-chat-link-card=""
      onClick={() => onOpenChatLink(link)}
      title={href}
      type="button"
    >
      {content}
    </button>
  );
}
