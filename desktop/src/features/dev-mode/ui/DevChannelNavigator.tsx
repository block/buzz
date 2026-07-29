import * as React from "react";

import {
  type ChannelGroup,
  toggleChannelPinned,
} from "@/features/dev-mode/lib/pinnedChannels";
import { useNavigatorWidth } from "@/features/dev-mode/lib/useNavigatorWidth";
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

function ChannelRow({
  channel,
  isHighlighted,
  isPinned,
  isUnread,
  onHighlight,
  onOpen,
}: {
  channel: Channel;
  isHighlighted: boolean;
  isPinned: boolean;
  isUnread: boolean;
  onHighlight: (channelId: string) => void;
  onOpen: (channelId: string) => void;
}) {
  const scrollHighlightedIntoView = React.useCallback(
    (node: HTMLDivElement | null) => {
      node?.scrollIntoView({ block: "nearest" });
    },
    [],
  );

  return (
    <div
      ref={isHighlighted ? scrollHighlightedIntoView : undefined}
      className={cn(
        "group relative flex items-baseline",
        isHighlighted
          ? "bg-primary/15 text-foreground"
          : isUnread
            ? "text-foreground hover:bg-muted/40"
            : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
      )}
    >
      <button
        className="flex min-w-0 flex-1 cursor-pointer items-baseline gap-2 rounded-none px-2 py-0.5 text-left text-sm"
        onClick={() =>
          isHighlighted ? onOpen(channel.id) : onHighlight(channel.id)
        }
        onDoubleClick={() => onOpen(channel.id)}
        type="button"
      >
        <span aria-hidden className="w-3 shrink-0 select-none">
          {isHighlighted ? "▸" : ""}
        </span>
        <span
          className={cn(
            "min-w-0 flex-1 truncate",
            isUnread ? "font-semibold" : "font-medium",
          )}
        >
          # {channel.name}
        </span>
        {isUnread ? (
          <span
            className="shrink-0 self-center text-3xs leading-none text-primary"
            data-testid="dev-mode-unread-dot"
            role="img"
            aria-label="unread"
          >
            ●
          </span>
        ) : null}
        <span className="shrink-0 text-xs text-muted-foreground/60">
          {formatRelativeTime(channel.lastMessageAt)}
        </span>
      </button>
      <button
        aria-label={
          isPinned ? `Unpin # ${channel.name}` : `Pin # ${channel.name}`
        }
        className={cn(
          "shrink-0 cursor-pointer px-1.5 py-0.5 text-xs text-muted-foreground/60 hover:text-foreground",
          !isPinned && "opacity-0 group-hover:opacity-100",
        )}
        onClick={() => toggleChannelPinned(channel.id)}
        type="button"
      >
        {isPinned ? "unpin" : "pin"}
      </button>
    </div>
  );
}

/**
 * Always-visible left channel list. The shell owns which channel is
 * highlighted (↑/↓) and what Enter/Escape do; this renders a pinned section
 * on top and all other chats beneath — both ordered by last activity, most
 * recent first — with unread indicators and per-row pin toggles.
 */
export function DevChannelNavigator({
  groups,
  unreadChannelIds,
  highlightedId,
  dimmed,
  onHighlight,
  onOpen,
}: {
  /** Render-ordered groups; within each, most recent activity renders first. */
  groups: ChannelGroup[];
  unreadChannelIds: ReadonlySet<string>;
  highlightedId: string | null;
  /** True while a channel is focused — the list stays visible but recedes. */
  dimmed: boolean;
  onHighlight: (channelId: string) => void;
  onOpen: (channelId: string) => void;
}) {
  const isEmpty = groups.every((group) => group.channels.length === 0);
  const { width, dragging, dividerProps } = useNavigatorWidth();

  return (
    <div className="flex shrink-0" style={{ width }}>
      <div
        className={cn(
          "flex min-w-0 flex-1 flex-col bg-background font-mono transition-opacity",
          dimmed && "opacity-45",
        )}
        data-testid="dev-mode-channel-navigator"
      >
        <div className="min-h-0 flex-1 overflow-y-auto px-1 py-1">
          {isEmpty ? (
            <div className="px-2 py-1 text-sm text-muted-foreground/60">
              no sessions yet
            </div>
          ) : null}
          {groups.map((group) => (
            <div key={group.pinned ? "pinned" : "chats"}>
              {group.pinned ? (
                <div className="select-none px-2 pb-0.5 pt-2 text-xs uppercase tracking-wide text-muted-foreground/50">
                  pinned
                </div>
              ) : groups.length > 1 ? (
                <div className="mt-2 border-t border-border/40 pt-1" />
              ) : null}
              {group.channels.map((channel) => (
                <ChannelRow
                  key={channel.id}
                  channel={channel}
                  isHighlighted={channel.id === highlightedId}
                  isPinned={group.pinned}
                  isUnread={unreadChannelIds.has(channel.id)}
                  onHighlight={onHighlight}
                  onOpen={onOpen}
                />
              ))}
            </div>
          ))}
        </div>
        <div className="shrink-0 truncate border-t border-border/60 px-3 py-1.5 text-xs text-muted-foreground/60">
          ↑↓: preview · enter: open · esc: back
        </div>
      </div>
      {/* biome-ignore lint/a11y/useSemanticElements: <hr> cannot host the drag/keyboard resize handlers of a movable separator */}
      <div
        className={cn(
          "w-1 shrink-0 cursor-col-resize bg-border/60 outline-none hover:bg-primary/60 focus-visible:bg-primary/60",
          dragging && "bg-primary",
        )}
        data-testid="dev-mode-navigator-resize"
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={Math.round(width)}
        tabIndex={0}
        {...dividerProps}
      />
    </div>
  );
}
