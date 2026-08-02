import { useNavigate } from "@tanstack/react-router";
import { ChevronDown, LayoutGrid } from "lucide-react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { cn } from "@/shared/lib/cn";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { useBoardsQuery } from "@/features/kanban/lib/boardQueries";

type BoardRailProps = {
  isCollapsed: boolean;
  onToggleCollapsed: () => void;
};

const SECTION_LABEL_CLASS =
  "group/section-label group-data-[collapsible=icon]:hidden flex w-fit max-w-full cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";

/**
 * "Boards" rail section, shown immediately after Direct messages.
 *
 * Lists the boards the current user can see (own + invited), each linking to
 * `/boards/:id`. Collapse state is owned by AppSidebar.
 */
export function BoardRail({ isCollapsed, onToggleCollapsed }: BoardRailProps) {
  const { data: identity } = useIdentityQuery();
  const me = identity?.pubkey;
  const boardsQuery = useBoardsQuery(me);
  const boards = boardsQuery.data ?? [];
  const navigate = useNavigate();

  return (
    <SidebarGroup data-sidebar-rail="boards">
      <SidebarGroupLabel asChild>
        <button
          aria-expanded={!isCollapsed}
          className={SECTION_LABEL_CLASS}
          data-testid="boards-section-label"
          onClick={onToggleCollapsed}
          type="button"
        >
          <span data-sidebar-section-title>Boards</span>
          <span aria-hidden="true" className="flex items-center">
            <ChevronDown
              className={cn(
                "size-2.5 transition-transform",
                isCollapsed ? "-rotate-90" : "rotate-0",
              )}
            />
          </span>
        </button>
      </SidebarGroupLabel>

      {!isCollapsed ? (
        <SidebarGroupContent>
          <SidebarMenu data-testid="boards-list">
            {boards.length === 0 ? (
              <div
                className="px-2 py-1 text-sm text-sidebar-foreground/60"
                data-testid="boards-list-empty"
              >
                No boards yet
              </div>
            ) : (
              boards.map((board) => (
                <SidebarMenuItem className="group/menu-item" key={board.id}>
                  <SidebarMenuButton
                    data-testid={`board-${board.id}`}
                    onClick={() =>
                      navigate({
                        to: "/boards/$boardId",
                        params: { boardId: board.id },
                      })
                    }
                    tooltip={board.name}
                    type="button"
                  >
                    <LayoutGrid className="size-4 shrink-0" />
                    <span className="min-w-0 flex-1 truncate">
                      {board.name}
                    </span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              ))
            )}
          </SidebarMenu>
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}
