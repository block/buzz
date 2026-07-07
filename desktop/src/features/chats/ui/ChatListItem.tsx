import { Archive, Pencil, Pin, PinOff } from "lucide-react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";

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
  canRename = true,
  chat,
  displayName,
  getChannelReadAt,
  isAgentRunning = false,
  isArchiving = false,
  isPinned = false,
  onArchiveChat,
  onRenameChat,
  onSelectChat,
  onTogglePin,
  selectedChatId,
  unreadChannelCounts,
  unreadChannelIds,
}: {
  /** Renaming writes owner metadata — disabled for shared chats. */
  canRename?: boolean;
  chat: Channel;
  /** Preferred label (chat metadata title); falls back to the channel name. */
  displayName?: string | null;
  getChannelReadAt: (channelId: string) => number | null;
  isAgentRunning?: boolean;
  isArchiving?: boolean;
  isPinned?: boolean;
  onArchiveChat?: (chatId: string) => void;
  onRenameChat?: (chatId: string) => void;
  onSelectChat: (chatId: string) => void;
  onTogglePin?: (chatId: string) => void;
  selectedChatId: string | null;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
}) {
  const name = displayName?.trim() || chat.name;
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

  const row = (
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
        <span className="min-w-0 flex-1 truncate font-medium">
          {/* Shimmer lives on an inner inline-block sized to the TEXT (the
              outer span is flex-1 = full row, and the highlight wash would
              otherwise sweep the whole row). The inner truncates itself so
              the overlay's ellipsis stays aligned with the visible text. */}
          <span
            className={cn(
              "max-w-full truncate align-bottom",
              isAgentRunning && "buzz-shimmer buzz-shimmer-accent inline-block",
            )}
            data-shimmer-text={isAgentRunning ? name : undefined}
          >
            {name}
          </span>
        </span>
        {isPinned ? (
          <Pin
            aria-hidden="true"
            className="h-3 w-3 shrink-0 text-muted-foreground/70"
          />
        ) : null}
        {hasUnread ? (
          <span className="shrink-0 rounded-full bg-primary/15 px-1.5 text-2xs font-semibold text-primary">
            {unreadCount > 0 ? Math.min(unreadCount, 99) : ""}
          </span>
        ) : null}
      </button>
      {onArchiveChat ? (
        // Zero-width until hover/focus so the title gets the full row; the
        // slot appears instantly to make room for the archive button.
        <div className="relative flex h-6 w-0 shrink-0 items-center justify-center overflow-hidden group-focus-within/chat-row:w-6 group-hover/chat-row:w-6">
          <Button
            aria-label={`Archive ${name}`}
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
        </div>
      ) : null}
    </div>
  );

  if (!onRenameChat && !onTogglePin && !onArchiveChat) {
    return row;
  }

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
      <ContextMenuContent className="w-44">
        {onRenameChat && canRename ? (
          <ContextMenuItem
            onSelect={() => {
              // Defer past the menu's close/focus-restore: opening a dialog
              // in the same tick wrestles Radix's focus management and can
              // freeze the webview (observed as an app "crash" on rename).
              window.setTimeout(() => onRenameChat(chat.id), 0);
            }}
          >
            <Pencil className="h-3.5 w-3.5" />
            Rename chat
          </ContextMenuItem>
        ) : null}
        {onTogglePin ? (
          <ContextMenuItem onSelect={() => onTogglePin(chat.id)}>
            {isPinned ? (
              <PinOff className="h-3.5 w-3.5" />
            ) : (
              <Pin className="h-3.5 w-3.5" />
            )}
            {isPinned ? "Unpin chat" : "Pin chat"}
          </ContextMenuItem>
        ) : null}
        {onArchiveChat ? (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem
              disabled={isArchiving}
              onSelect={() => onArchiveChat(chat.id)}
            >
              <Archive className="h-3.5 w-3.5" />
              Archive chat
            </ContextMenuItem>
          </>
        ) : null}
      </ContextMenuContent>
    </ContextMenu>
  );
}
