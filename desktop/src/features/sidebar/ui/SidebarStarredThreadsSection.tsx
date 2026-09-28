import { ChevronDown, MessageSquare, StarOff } from "lucide-react";
import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import {
  countUnreadForStarredThreadRoot,
  formatSidebarUnreadCount,
} from "@/features/sidebar/lib/starredThreadSidebar";
import type { StarredThreadEntry } from "@/features/sidebar/lib/threadStarsStorage";
import {
  ChannelWorkingBadge,
  formatWorkingTooltip,
} from "@/features/sidebar/ui/channelWorkingBadge";
import { cn } from "@/shared/lib/cn";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "relative size-2.5 shrink-0 text-current opacity-0 transition-[color,opacity] group-hover/sidebar-section:opacity-100 group-hover/section-label:opacity-100 group-focus-within/sidebar-section:opacity-100 group-focus-visible/section-label:opacity-100";
const SECTION_LABEL_CHEVRON_ICON_CLASS =
  "absolute left-1/2 top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2";

function selectedThreadRootFromSearch(search: unknown): string | null {
  if (typeof search !== "object" || search === null) return null;
  const record = search as { thread?: unknown; threadRootId?: unknown };
  const thread = record.threadRootId ?? record.thread;
  return typeof thread === "string" && thread.trim() ? thread.trim() : null;
}

function StarredThreadRow({
  entry,
  isActive,
  unreadCount,
  activeWorking,
  onSelect,
  onUnstar,
}: {
  entry: StarredThreadEntry;
  isActive: boolean;
  unreadCount: number;
  activeWorking?: ActiveChannelTurnSummary;
  onSelect: () => void;
  onUnstar: () => void;
}) {
  const hasUnread = unreadCount > 0;
  const workingTitle = activeWorking
    ? formatWorkingTooltip(activeWorking)
    : undefined;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <SidebarMenuItem>
          <SidebarMenuButton
            className={cn(
              "h-8 data-[active=true]:font-normal",
              hasUnread &&
                "font-bold text-sidebar-foreground hover:text-sidebar-foreground data-[active=true]:font-bold",
            )}
            data-testid={`starred-thread-${entry.rootId}`}
            isActive={isActive}
            onClick={onSelect}
            title={
              workingTitle
                ? `${entry.title} · #${entry.channelName} · ${workingTitle}`
                : `${entry.title} · #${entry.channelName}`
            }
            tooltip={`${entry.title} · #${entry.channelName}`}
          >
            <MessageSquare className="h-4 w-4 shrink-0" />
            <span
              className={cn(
                "min-w-0 flex-1 truncate text-left",
                !isActive && !hasUnread && "opacity-80",
              )}
            >
              {entry.title}
            </span>
            {activeWorking ? (
              <ChannelWorkingBadge
                channelName={entry.channelName}
                isActive={isActive}
                summary={activeWorking}
                testId={`starred-thread-working-${entry.rootId}`}
              />
            ) : null}
            <span className="min-w-0 max-w-[40%] truncate text-2xs text-sidebar-foreground/55">
              #{entry.channelName}
            </span>
            {hasUnread ? (
              <span
                className={cn(
                  "ml-auto shrink-0 rounded-full px-1.5 text-2xs tabular-nums",
                  isActive
                    ? "bg-sidebar-active-foreground/20 text-sidebar-active-foreground"
                    : "bg-primary/15 text-primary",
                )}
                data-testid={`starred-thread-unread-${entry.rootId}`}
              >
                {formatSidebarUnreadCount(unreadCount)}
                <span className="sr-only">
                  {" "}
                  unread{unreadCount === 1 ? "" : "s"}
                </span>
              </span>
            ) : null}
          </SidebarMenuButton>
        </SidebarMenuItem>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          onSelect={() => {
            onUnstar();
          }}
        >
          <StarOff className="h-4 w-4" />
          <span>Unstar thread</span>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export function SidebarStarredThreadsSection({
  activeWorkingByChannelId,
  isCollapsed,
  items,
  onToggleCollapsed,
  onUnstarThread,
}: {
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  isCollapsed: boolean;
  items: readonly StarredThreadEntry[];
  onToggleCollapsed: () => void;
  onUnstarThread: (rootId: string) => void;
}) {
  const { goChannel } = useAppNavigation();
  const location = useLocation();
  const { unreadThreadFeedItems } = useAppShell();
  const selectedThreadRootId = React.useMemo(
    () => selectedThreadRootFromSearch(location.search),
    [location.search],
  );
  const unreadCountByRootId = React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const entry of items) {
      const count = countUnreadForStarredThreadRoot(
        unreadThreadFeedItems,
        entry.rootId,
      );
      if (count > 0) counts.set(entry.rootId, count);
    }
    return counts;
  }, [items, unreadThreadFeedItems]);
  const contentId = "sidebar-starred-threads-list";

  if (items.length === 0) {
    return null;
  }

  return (
    <SidebarGroup
      className="group/sidebar-section select-none"
      data-testid="starred-threads-section"
    >
      <div className="relative">
        <SidebarGroupLabel asChild>
          <button
            aria-controls={contentId}
            aria-expanded={!isCollapsed}
            className={SECTION_LABEL_BUTTON_CLASS}
            data-testid="starred-threads-section-label"
            onClick={onToggleCollapsed}
            type="button"
          >
            <span data-sidebar-section-title>Starred threads</span>
            <span aria-hidden="true" className={SECTION_LABEL_CHEVRON_CLASS}>
              <ChevronDown
                className={cn(
                  SECTION_LABEL_CHEVRON_ICON_CLASS,
                  isCollapsed ? "-rotate-90" : "rotate-0",
                )}
              />
            </span>
          </button>
        </SidebarGroupLabel>
      </div>
      {!isCollapsed ? (
        <SidebarGroupContent id={contentId}>
          <SidebarMenu data-testid="starred-threads-list">
            {items.map((entry) => (
              <StarredThreadRow
                key={entry.rootId}
                activeWorking={activeWorkingByChannelId?.get(entry.channelId)}
                entry={entry}
                isActive={selectedThreadRootId === entry.rootId}
                onSelect={() => {
                  void goChannel(entry.channelId, { thread: entry.rootId });
                }}
                onUnstar={() => onUnstarThread(entry.rootId)}
                unreadCount={unreadCountByRootId.get(entry.rootId) ?? 0}
              />
            ))}
          </SidebarMenu>
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}
