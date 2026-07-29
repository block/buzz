import * as React from "react";

import {
  assignChannelCategory,
  type ChannelGroup,
} from "@/features/dev-mode/lib/channelCategories";
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

/**
 * Inline category picker for one channel row. Local (device-only) grouping —
 * see channelCategories.ts.
 */
function CategoryMenu({
  channelId,
  categories,
  current,
  onClose,
}: {
  channelId: string;
  categories: string[];
  current: string | null;
  onClose: () => void;
}) {
  const [draft, setDraft] = React.useState("");

  const pick = (category: string | null) => {
    assignChannelCategory(channelId, category);
    onClose();
  };

  return (
    <div
      className="absolute right-1 top-full z-30 w-48 border border-border bg-background py-1 shadow-md"
      data-testid="dev-mode-category-menu"
    >
      {categories.map((category) => (
        <button
          key={category}
          className={cn(
            "block w-full cursor-pointer px-2 py-0.5 text-left text-xs hover:bg-muted/60",
            category === current ? "text-foreground" : "text-muted-foreground",
          )}
          onClick={() => pick(category)}
          type="button"
        >
          {category === current ? "✓ " : ""}
          {category}
        </button>
      ))}
      {current !== null ? (
        <button
          className="block w-full cursor-pointer px-2 py-0.5 text-left text-xs text-muted-foreground hover:bg-muted/60"
          onClick={() => pick(null)}
          type="button"
        >
          remove from category
        </button>
      ) : null}
      <form
        className="px-2 py-1"
        onSubmit={(event) => {
          event.preventDefault();
          const name = draft.trim();
          if (name) pick(name);
        }}
      >
        <input
          className="w-full border border-border/60 bg-transparent px-1 py-0.5 text-xs outline-none placeholder:text-muted-foreground/50"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              onClose();
            }
          }}
          placeholder="new category…"
          value={draft}
        />
      </form>
    </div>
  );
}

function ChannelRow({
  channel,
  isHighlighted,
  categories,
  currentCategory,
  onHighlight,
  onOpen,
}: {
  channel: Channel;
  isHighlighted: boolean;
  categories: string[];
  currentCategory: string | null;
  onHighlight: (channelId: string) => void;
  onOpen: (channelId: string) => void;
}) {
  const [menuOpen, setMenuOpen] = React.useState(false);

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
        <span className="min-w-0 flex-1 truncate font-medium">
          # {channel.name}
        </span>
        <span className="shrink-0 text-xs text-muted-foreground/60">
          {formatRelativeTime(channel.lastMessageAt)}
        </span>
      </button>
      <button
        aria-label={`Set category for # ${channel.name}`}
        className={cn(
          "shrink-0 cursor-pointer px-1.5 py-0.5 text-xs text-muted-foreground/60 hover:text-foreground",
          !menuOpen && "opacity-0 group-hover:opacity-100",
        )}
        onClick={() => setMenuOpen((open) => !open)}
        type="button"
      >
        ⌄
      </button>
      {menuOpen ? (
        <CategoryMenu
          categories={categories}
          channelId={channel.id}
          current={currentCategory}
          onClose={() => setMenuOpen(false)}
        />
      ) : null}
    </div>
  );
}

/**
 * Always-visible left channel list. The shell owns which channel is
 * highlighted (↑/↓) and what Enter/Escape do; this renders channels grouped
 * by device-local categories (uncategorized channels — including every new
 * session — below all categories) and supports mouse highlight/open plus
 * per-row category assignment.
 */
export function DevChannelNavigator({
  groups,
  categoryOrder,
  channelCategory,
  highlightedId,
  dimmed,
  onHighlight,
  onOpen,
}: {
  /** Render-ordered groups; within each, newest channels render last. */
  groups: ChannelGroup[];
  categoryOrder: string[];
  channelCategory: (channelId: string) => string | null;
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
            <div key={group.category ?? "\u0000uncategorized"}>
              {group.category !== null ? (
                <div className="select-none px-2 pb-0.5 pt-2 text-xs uppercase tracking-wide text-muted-foreground/50">
                  {group.category}
                </div>
              ) : groups.length > 1 ? (
                <div className="mt-2 border-t border-border/40 pt-1" />
              ) : null}
              {group.channels.map((channel) => (
                <ChannelRow
                  key={channel.id}
                  categories={categoryOrder}
                  channel={channel}
                  currentCategory={channelCategory(channel.id)}
                  isHighlighted={channel.id === highlightedId}
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
