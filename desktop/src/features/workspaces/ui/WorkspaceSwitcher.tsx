import {
  Check,
  ChevronDown,
  ChevronRight,
  MoreHorizontal,
  Plus,
  WifiOff,
} from "lucide-react";
import * as React from "react";

import type { Workspace } from "@/features/workspaces/types";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { EditWorkspaceDialog } from "./EditWorkspaceDialog";

const CONNECTION_STATE_LABEL: Record<ConnectionState, string> = {
  idle: "Not connected",
  connecting: "Connecting…",
  connected: "Connected",
  reconnecting: "Reconnecting to relay…",
  stalled: "Connection lost — relay is not responding",
  disconnected: "Disconnected from relay",
};

type WorkspaceSwitcherProps = {
  activeWorkspace: Workspace | null;
  workspaces: Workspace[];
  variant?: "sidebar" | "profile" | "profile-menu";
  onSwitchWorkspace: (id: string) => void;
  onAddWorkspace: () => void;
  onUpdateWorkspace: (
    id: string,
    updates: Partial<Pick<Workspace, "name" | "relayUrl" | "token">>,
  ) => void;
  onRemoveWorkspace: (id: string) => void;
};

export function WorkspaceSwitcher({
  activeWorkspace,
  workspaces,
  variant = "sidebar",
  onSwitchWorkspace,
  onAddWorkspace,
  onUpdateWorkspace,
  onRemoveWorkspace,
}: WorkspaceSwitcherProps) {
  const [editingWorkspace, setEditingWorkspace] =
    React.useState<Workspace | null>(null);
  const [dropdownOpen, setDropdownOpen] = React.useState(false);
  const profileMenuHoverTimer = React.useRef<number | null>(null);
  const connectionState = useRelayConnection();
  const degraded = isRelayConnectionDegraded(connectionState);
  const connectionLabel = CONNECTION_STATE_LABEL[connectionState];
  const isProfileVariant = variant === "profile";

  function clearProfileMenuHoverTimer() {
    if (profileMenuHoverTimer.current !== null) {
      window.clearTimeout(profileMenuHoverTimer.current);
      profileMenuHoverTimer.current = null;
    }
  }

  function scheduleProfileMenu(nextOpen: boolean) {
    if (variant !== "profile-menu") return;
    clearProfileMenuHoverTimer();
    profileMenuHoverTimer.current = window.setTimeout(
      () => setDropdownOpen(nextOpen),
      nextOpen ? 80 : 160,
    );
  }

  function handleProfileMenuOpenChange(nextOpen: boolean) {
    if (variant !== "profile-menu") {
      setDropdownOpen(nextOpen);
      return;
    }
    if (!nextOpen) {
      clearProfileMenuHoverTimer();
    }
    setDropdownOpen(nextOpen);
  }

  React.useEffect(
    () => () => {
      if (profileMenuHoverTimer.current !== null) {
        window.clearTimeout(profileMenuHoverTimer.current);
      }
    },
    [],
  );

  const triggerContent = (
    <>
      {degraded ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <span
              aria-hidden="false"
              className={
                isProfileVariant
                  ? "flex h-5 w-5 shrink-0 animate-pulse items-center justify-center rounded-md border border-sidebar-border/70 bg-sidebar-accent/40 text-destructive"
                  : "flex h-5 w-5 shrink-0 animate-pulse items-center justify-center text-destructive"
              }
              data-testid="relay-connection-warning"
              role="img"
            >
              <WifiOff className={isProfileVariant ? "h-4 w-4" : "h-4 w-4"} />
            </span>
          </TooltipTrigger>
          <TooltipContent side={isProfileVariant ? "top" : "bottom"}>
            {connectionLabel}
          </TooltipContent>
        </Tooltip>
      ) : (
        <span
          className={
            isProfileVariant
              ? "flex h-5 w-5 shrink-0 items-center justify-center rounded-md border border-sidebar-border/70 bg-sidebar-accent/40 text-[10px] leading-none"
              : "flex h-5 w-5 shrink-0 items-center justify-center text-xs leading-none"
          }
        >
          🐝
        </span>
      )}
      <span
        className={
          degraded
            ? "min-w-0 flex-1 truncate font-medium text-destructive animate-pulse"
            : "min-w-0 flex-1 truncate font-medium"
        }
      >
        {activeWorkspace?.name ?? "No workspace"}
      </span>
      {variant === "profile-menu" ? (
        <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : (
        <ChevronDown
          className={
            isProfileVariant
              ? "h-4 w-4 shrink-0 text-sidebar-foreground/45"
              : "h-4 w-4 shrink-0 text-sidebar-foreground/50"
          }
        />
      )}
    </>
  );

  const profileMenuPopover =
    variant === "profile-menu" ? (
      <Popover open={dropdownOpen} onOpenChange={handleProfileMenuOpenChange}>
        <PopoverTrigger asChild>
          <button
            aria-expanded={dropdownOpen}
            aria-haspopup="menu"
            aria-label={
              degraded
                ? `${activeWorkspace?.name ?? "Workspace"} — ${connectionLabel}`
                : "Switch workspace"
            }
            className="flex w-full items-center gap-2 px-4 py-2 text-left text-sm text-popover-foreground outline-hidden transition-colors hover:bg-accent focus:bg-accent focus:outline-none focus-visible:bg-accent focus-visible:outline-none data-[state=open]:bg-accent data-[state=open]:text-popover-foreground"
            data-testid="workspace-switcher"
            onMouseEnter={() => scheduleProfileMenu(true)}
            onMouseLeave={() => scheduleProfileMenu(false)}
            role="menuitem"
            type="button"
          >
            {triggerContent}
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="end"
          className="w-56 rounded-xl border border-border bg-popover p-1 shadow-lg"
          onMouseEnter={() => scheduleProfileMenu(true)}
          onMouseLeave={() => scheduleProfileMenu(false)}
          side="right"
          sideOffset={0}
        >
          <div aria-label="Workspaces" role="menu">
            {workspaces.map((workspace) => (
              <div
                className="group flex items-center rounded-xs transition-colors hover:bg-accent focus-within:bg-accent"
                key={workspace.id}
              >
                <button
                  className="flex min-w-0 flex-1 items-center gap-2 px-2 py-1.5 text-left text-sm outline-hidden focus:outline-none"
                  onClick={() => {
                    onSwitchWorkspace(workspace.id);
                    setDropdownOpen(false);
                  }}
                  role="menuitem"
                  type="button"
                >
                  <span className="flex h-4 w-4 shrink-0 items-center justify-center">
                    {activeWorkspace?.id === workspace.id ? (
                      <Check className="h-4 w-4 text-primary" />
                    ) : null}
                  </span>
                  <span className="min-w-0 flex-1 truncate">
                    {workspace.name}
                  </span>
                </button>
                <button
                  aria-label={`Edit ${workspace.name}`}
                  className="mr-1 flex h-5 w-5 shrink-0 items-center justify-center rounded opacity-0 hover:bg-accent group-hover:opacity-100 group-focus-within:opacity-100"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDropdownOpen(false);
                    setEditingWorkspace(workspace);
                  }}
                  type="button"
                >
                  <MoreHorizontal className="h-4 w-4" />
                </button>
              </div>
            ))}
            <div className="-mx-1 my-1 h-px bg-muted" />
            <button
              className="flex w-full items-center gap-2 rounded-xs px-2 py-1.5 text-left text-sm outline-hidden transition-colors hover:bg-accent focus:bg-accent focus:outline-none focus-visible:bg-accent focus-visible:outline-none"
              onClick={() => {
                setDropdownOpen(false);
                onAddWorkspace();
              }}
              role="menuitem"
              type="button"
            >
              <Plus className="h-4 w-4" />
              <span>Add Workspace</span>
            </button>
          </div>
        </PopoverContent>
      </Popover>
    ) : null;

  const switcherDropdown = (
    <DropdownMenu open={dropdownOpen} onOpenChange={setDropdownOpen}>
      <DropdownMenuTrigger asChild>
        {variant === "profile" ? (
          <button
            aria-label={
              degraded
                ? `${activeWorkspace?.name ?? "Workspace"} — ${connectionLabel}`
                : "Switch workspace"
            }
            className="flex min-w-0 max-w-full items-center gap-1.5 rounded-md py-0.5 text-left text-xs text-sidebar-foreground/50 outline-hidden transition-colors hover:text-sidebar-foreground focus:outline-none focus-visible:outline-none data-[state=open]:text-sidebar-foreground"
            data-testid="workspace-switcher"
            type="button"
          >
            {triggerContent}
          </button>
        ) : (
          <SidebarMenuButton
            aria-label={
              degraded
                ? `${activeWorkspace?.name ?? "Workspace"} — ${connectionLabel}`
                : undefined
            }
            className="h-auto gap-2 rounded-xl px-2.5 py-2 data-[state=open]:bg-sidebar-accent"
            data-testid="workspace-switcher"
            type="button"
          >
            {triggerContent}
          </SidebarMenuButton>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="w-(--radix-dropdown-menu-trigger-width) min-w-[220px]"
        onCloseAutoFocus={(e) => e.preventDefault()}
        side={variant === "profile" ? "top" : "bottom"}
        sideOffset={4}
      >
        {workspaces.map((workspace) => (
          <DropdownMenuItem
            key={workspace.id}
            className="group flex items-center gap-2 pr-1"
            onSelect={() => {
              onSwitchWorkspace(workspace.id);
            }}
          >
            <span className="flex h-4 w-4 shrink-0 items-center justify-center">
              {activeWorkspace?.id === workspace.id ? (
                <Check className="h-4 w-4 text-primary" />
              ) : null}
            </span>
            <span className="min-w-0 flex-1 truncate">{workspace.name}</span>
            <button
              aria-label={`Edit ${workspace.name}`}
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded opacity-0 hover:bg-accent group-hover:opacity-100 group-focus:opacity-100"
              onClick={(e) => {
                e.stopPropagation();
                e.preventDefault();
                setDropdownOpen(false);
                setEditingWorkspace(workspace);
              }}
              type="button"
            >
              <MoreHorizontal className="h-4 w-4" />
            </button>
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onAddWorkspace}>
          <Plus className="h-4 w-4" />
          <span>Add Workspace</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  return (
    <>
      {variant === "profile" ? (
        switcherDropdown
      ) : variant === "profile-menu" ? (
        profileMenuPopover
      ) : (
        <SidebarMenu>
          <SidebarMenuItem>{switcherDropdown}</SidebarMenuItem>
        </SidebarMenu>
      )}

      <EditWorkspaceDialog
        canRemove={workspaces.length > 1}
        onOpenChange={(open) => {
          if (!open) setEditingWorkspace(null);
        }}
        onRemove={onRemoveWorkspace}
        onSave={onUpdateWorkspace}
        open={editingWorkspace !== null}
        workspace={editingWorkspace}
      />
    </>
  );
}
