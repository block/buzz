import { ReceiptText } from "lucide-react";

import {
  formatCoarseUptime,
  type ThreadAttentionRow,
} from "@/features/channels/lib/threadAttention";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { useNow } from "@/shared/lib/useNow";

const MAX_BADGE_COUNT = 99;

/**
 * Header-level threads attention control: a ReceiptText trigger badged with
 * the channel's unread-reply total, opening one combined list of threads that
 * are unread or have an agent actively working. Selecting a row opens the
 * thread panel focused on the thread head.
 */
export function ChannelThreadsAttentionMenu({
  onSelectThread,
  rows,
  unreadCount,
}: {
  onSelectThread: (threadHeadId: string) => void;
  rows: readonly ThreadAttentionRow[];
  unreadCount: number;
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label={
            unreadCount > 0
              ? `Threads needing attention (${unreadCount} unread)`
              : "Threads needing attention"
          }
          className="relative"
          data-testid="channel-threads-attention-trigger"
          size="icon"
          type="button"
          variant="outline"
        >
          <ReceiptText />
          {unreadCount > 0 ? (
            <span
              className="absolute -right-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground tabular-nums"
              data-testid="channel-threads-attention-badge"
            >
              {unreadCount > MAX_BADGE_COUNT
                ? `${MAX_BADGE_COUNT}+`
                : unreadCount}
            </span>
          ) : null}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-80"
        data-testid="channel-threads-attention-menu"
      >
        <DropdownMenuLabel>Threads</DropdownMenuLabel>
        {rows.length === 0 ? (
          <div className="px-2 py-4 text-center text-sm text-muted-foreground">
            No unread or active threads
          </div>
        ) : (
          <ThreadAttentionMenuRows
            onSelectThread={onSelectThread}
            rows={rows}
          />
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * Rendered only while the menu is open, so the 1s uptime tick never runs for
 * a closed menu.
 */
function ThreadAttentionMenuRows({
  onSelectThread,
  rows,
}: {
  onSelectThread: (threadHeadId: string) => void;
  rows: readonly ThreadAttentionRow[];
}) {
  const hasActiveRow = rows.some((row) => row.activeSince !== null);
  const now = useNow(hasActiveRow ? 1_000 : 60_000);

  return (
    <>
      {rows.map((row) => (
        <DropdownMenuItem
          data-testid={`channel-threads-attention-item-${row.threadHeadId}`}
          key={row.threadHeadId}
          onSelect={() => onSelectThread(row.threadHeadId)}
        >
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <div className="flex items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-sm font-medium">
                {row.headAuthor ?? "Thread"}
              </span>
              {row.activeSince !== null ? (
                <span
                  className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-2xs font-medium leading-none tabular-nums text-primary motion-safe:animate-pulse"
                  data-testid="channel-threads-attention-uptime"
                >
                  {formatCoarseUptime(now - row.activeSince)}
                </span>
              ) : null}
              {row.unreadCount > 0 ? (
                <span
                  className="flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground tabular-nums"
                  data-testid="channel-threads-attention-unread"
                >
                  {row.unreadCount}
                </span>
              ) : null}
            </div>
            <span className="truncate text-xs text-muted-foreground">
              {row.headPreview ??
                `${row.replyCount} ${row.replyCount === 1 ? "reply" : "replies"}`}
            </span>
          </div>
        </DropdownMenuItem>
      ))}
    </>
  );
}
