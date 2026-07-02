import { Archive } from "lucide-react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";

export function ChatListHeader() {
  return (
    <div
      className="pointer-events-auto relative z-30 shrink-0 cursor-default select-none border-b border-border/35 bg-transparent px-5 py-2"
      data-tauri-drag-region
    >
      <div className="h-9" />
    </div>
  );
}

export function ChatListItem({
  chat,
  getChannelReadAt,
  isAgentRunning = false,
  isArchiving = false,
  onArchiveChat,
  onSelectChat,
  selectedChatId,
  unreadChannelCounts,
  unreadChannelIds,
}: {
  chat: Channel;
  getChannelReadAt: (channelId: string) => number | null;
  isAgentRunning?: boolean;
  isArchiving?: boolean;
  onArchiveChat?: (chatId: string) => void;
  onSelectChat: (chatId: string) => void;
  selectedChatId: string | null;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
}) {
  const isUnread = unreadChannelIds.has(chat.id);
  const unreadCount = unreadChannelCounts.get(chat.id) ?? 0;
  const readAt = getChannelReadAt(chat.id);
  const lastMessageAt = chat.lastMessageAt
    ? Math.floor(Date.parse(chat.lastMessageAt) / 1_000)
    : null;
  const hasUnread =
    isUnread ||
    (readAt !== null && lastMessageAt !== null && lastMessageAt > readAt);

  const isSelected = selectedChatId === chat.id;

  return (
    <div
      className={cn(
        "group/chat-row flex h-8 w-full min-w-0 items-center gap-1 rounded-md px-1 text-sm transition-colors",
        isSelected
          ? "bg-secondary text-secondary-foreground"
          : "text-foreground hover:bg-muted",
      )}
    >
      <button
        className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left"
        onClick={() => onSelectChat(chat.id)}
        type="button"
      >
        <span className="min-w-0 flex-1 truncate font-medium">{chat.name}</span>
        {hasUnread ? (
          <span className="shrink-0 rounded-full bg-primary/15 px-1.5 text-2xs font-semibold text-primary">
            {unreadCount > 0 ? Math.min(unreadCount, 99) : ""}
          </span>
        ) : null}
      </button>
      {isAgentRunning || onArchiveChat ? (
        <div className="relative flex h-6 w-6 shrink-0 items-center justify-center">
          {isAgentRunning ? (
            <Spinner
              aria-label={`Agent is running in ${chat.name}`}
              className={cn(
                "h-3.5 w-3.5 border-2 transition-opacity",
                isSelected
                  ? "text-secondary-foreground/70"
                  : "text-muted-foreground",
                onArchiveChat &&
                  "group-focus-within/chat-row:opacity-0 group-hover/chat-row:opacity-0",
              )}
            />
          ) : null}
          {onArchiveChat ? (
            <Button
              aria-label={`Archive ${chat.name}`}
              className={cn(
                "absolute inset-0 h-6 w-6 bg-transparent text-muted-foreground opacity-0 shadow-none transition-[background-color,color,opacity] hover:bg-muted hover:text-foreground focus-visible:bg-muted focus-visible:text-foreground focus-visible:opacity-100 group-focus-within/chat-row:opacity-100 group-hover/chat-row:opacity-100",
                isSelected
                  ? "hover:bg-secondary-foreground/10 focus-visible:bg-secondary-foreground/10"
                  : "hover:bg-muted focus-visible:bg-muted",
              )}
              disabled={isArchiving}
              onClick={() => onArchiveChat(chat.id)}
              size="icon-xs"
              title="Archive chat"
              type="button"
              variant="ghost"
            >
              <Archive className="h-3.5 w-3.5" />
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
