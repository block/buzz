import { ChevronDown, MessageSquare, StarOff } from "lucide-react";
import * as React from "react";
import { useLocation } from "@tanstack/react-router";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { StarredThreadEntry } from "@/features/sidebar/lib/threadStarsStorage";
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
  onSelect,
  onUnstar,
}: {
  entry: StarredThreadEntry;
  isActive: boolean;
  onSelect: () => void;
  onUnstar: () => void;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <SidebarMenuItem>
          <SidebarMenuButton
            className="h-8"
            data-testid={`starred-thread-${entry.rootId}`}
            isActive={isActive}
            onClick={onSelect}
            tooltip={`${entry.title} · #${entry.channelName}`}
          >
            <MessageSquare className="h-4 w-4 shrink-0" />
            <span className="min-w-0 flex-1 truncate text-left">
              {entry.title}
            </span>
            <span className="min-w-0 max-w-[40%] truncate text-2xs text-sidebar-foreground/55">
              #{entry.channelName}
            </span>
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
  isCollapsed,
  items,
  onToggleCollapsed,
  onUnstarThread,
}: {
  isCollapsed: boolean;
  items: readonly StarredThreadEntry[];
  onToggleCollapsed: () => void;
  onUnstarThread: (rootId: string) => void;
}) {
  const { goChannel } = useAppNavigation();
  const location = useLocation();
  const selectedThreadRootId = React.useMemo(
    () => selectedThreadRootFromSearch(location.search),
    [location.search],
  );
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
                entry={entry}
                isActive={selectedThreadRootId === entry.rootId}
                onSelect={() => {
                  void goChannel(entry.channelId, { thread: entry.rootId });
                }}
                onUnstar={() => onUnstarThread(entry.rootId)}
              />
            ))}
          </SidebarMenu>
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}
