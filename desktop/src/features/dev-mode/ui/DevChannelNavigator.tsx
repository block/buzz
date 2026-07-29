import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

function formatRelativeTime(iso: string | null) {
  if (!iso) return "";
  const deltaSeconds = Math.max(
    0,
    Math.floor((Date.now() - new Date(iso).getTime()) / 1_000),
  );
  if (deltaSeconds < 60) return "now";
  if (deltaSeconds < 3_600) return `${Math.floor(deltaSeconds / 60)}m`;
  if (deltaSeconds < 86_400) return `${Math.floor(deltaSeconds / 3_600)}h`;
  return `${Math.floor(deltaSeconds / 86_400)}d`;
}

/**
 * Left slide-out channel list. The shell owns which channel is highlighted
 * (↑/↓) and what Enter/Escape do; this renders the full session list —
 * hundreds of channels scroll within the panel — and supports mouse
 * highlight/open.
 */
export function DevChannelNavigator({
  channels,
  highlightedId,
  onHighlight,
  onOpen,
}: {
  /** Ascending by recency — the newest channel renders last, nearest the composer. */
  channels: Channel[];
  highlightedId: string | null;
  onHighlight: (channelId: string) => void;
  onOpen: (channelId: string) => void;
}) {
  const scrollHighlightedIntoView = React.useCallback(
    (node: HTMLButtonElement | null) => {
      node?.scrollIntoView({ block: "nearest" });
    },
    [],
  );

  return (
    <div
      className="flex w-80 shrink-0 animate-in slide-in-from-left flex-col border-r border-border/60 bg-background font-mono duration-150"
      data-testid="dev-mode-channel-navigator"
    >
      <div className="shrink-0 border-b border-border/60 px-3 py-1.5 text-xs text-muted-foreground">
        channels · {channels.length}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-1 py-1">
        {channels.length === 0 ? (
          <div className="px-2 py-1 text-sm text-muted-foreground/60">
            no sessions yet
          </div>
        ) : null}
        {channels.map((channel) => {
          const isHighlighted = channel.id === highlightedId;
          return (
            <button
              key={channel.id}
              ref={isHighlighted ? scrollHighlightedIntoView : undefined}
              className={cn(
                "flex w-full cursor-pointer items-baseline gap-2 rounded-none px-2 py-0.5 text-left text-sm",
                isHighlighted
                  ? "bg-primary/15 text-foreground"
                  : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
              )}
              onClick={() =>
                isHighlighted ? onOpen(channel.id) : onHighlight(channel.id)
              }
              onDoubleClick={() => onOpen(channel.id)}
              type="button"
            >
              <span aria-hidden className="w-3 shrink-0 select-none">
                {isHighlighted ? "▸" : ""}
              </span>
              <span className="min-w-0 flex-1 truncate font-medium">
                # {channel.name}
              </span>
              <span className="shrink-0 text-xs text-muted-foreground/60">
                {formatRelativeTime(channel.lastMessageAt)}
              </span>
            </button>
          );
        })}
      </div>
      <div className="shrink-0 border-t border-border/60 px-3 py-1.5 text-xs text-muted-foreground/60">
        ↑↓: preview · enter: open · esc: back
      </div>
    </div>
  );
}
