import {
  Check,
  ChevronDown,
  ChevronRight,
  Link2,
  MoreHorizontal,
  Plus,
  Settings2,
  LogOut,
  Unplug,
  Ticket,
  WifiOff,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  canRemoveCommunityLocally,
  type LeaveCommunityResult,
} from "@/features/communities/leaveCommunity";
import type { Community } from "@/features/communities/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
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
import { cn } from "@/shared/lib/cn";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { useActiveCommunityIcon } from "@/features/communities/useCommunityIcons";
import { EditCommunityDialog } from "./EditCommunityDialog";

// Community actions is a responsive navigation submenu, not an informational
// disclosure. Keep its short hover dwell explicit rather than inheriting the
// shared 500 ms Popover delay intended to prevent incidental inspection UI.
const PROFILE_MENU_HOVER_OPEN_DELAY_MS = 80;
const PROFILE_MENU_HOVER_CLOSE_DELAY_MS = 160;

const CONNECTION_STATE_LABEL: Record<ConnectionState, string> = {
  idle: "Not connected",
  connecting: "Connecting…",
  connected: "Connected",
  reconnecting: "Reconnecting to relay…",
  stalled: "Connection lost — relay is not responding",
  disconnected: "Disconnected from relay",
};

type CommunitySwitcherProps = {
  activeCommunity: Community | null;
  communities: Community[];
  variant?: "sidebar" | "profile" | "profile-menu";
  canInvite?: boolean;
  onInvite?: () => void;
  onSwitchCommunity: (id: string) => void;
  onAddCommunity: () => void;
  onUpdateCommunity: (
    id: string,
    updates: Partial<Pick<Community, "name" | "relayUrl" | "token">>,
  ) => void;
  onRemoveCommunity: (
    id: string,
    mode?: "leave" | "local-only",
  ) => Promise<LeaveCommunityResult | undefined>;
};

export function CommunityEmojiIcon({
  className,
  iconUrl,
}: {
  className: string;
  iconUrl?: string | null;
}) {
  if (iconUrl) {
    return (
      <span
        aria-hidden="true"
        className={cn(className, "h-5 overflow-hidden rounded-md")}
      >
        <img
          alt=""
          className="h-full w-full object-cover"
          draggable={false}
          src={iconUrl}
        />
      </span>
    );
  }
  return (
    <span aria-hidden="true" className={className}>
      <span className="-translate-y-px leading-normal">🐝</span>
    </span>
  );
}

export function CommunitySwitcher({
  activeCommunity,
  communities,
  variant = "sidebar",
  canInvite = false,
  onInvite,
  onSwitchCommunity,
  onAddCommunity,
  onUpdateCommunity,
  onRemoveCommunity,
}: CommunitySwitcherProps) {
  const [editingCommunity, setEditingCommunity] =
    React.useState<Community | null>(null);
  const [dropdownOpen, setDropdownOpen] = React.useState(false);
  const [leaveError, setLeaveError] = React.useState<string | null>(null);
  const [isLeaving, setIsLeaving] = React.useState(false);
  const [removingCommunity, setRemovingCommunity] =
    React.useState<Community | null>(null);
  const [unavailableCommunityId, setUnavailableCommunityId] = React.useState<
    string | null
  >(null);
  const profileMenuHoverTimer = React.useRef<number | null>(null);
  const connectionState = useRelayConnection();
  const degraded = isRelayConnectionDegraded(connectionState);
  const connectionLabel = CONNECTION_STATE_LABEL[connectionState];
  const activeIconQuery = useActiveCommunityIcon(activeCommunity?.relayUrl);
  const activeIcon = activeIconQuery.data ?? null;
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
      nextOpen
        ? PROFILE_MENU_HOVER_OPEN_DELAY_MS
        : PROFILE_MENU_HOVER_CLOSE_DELAY_MS,
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

  const handleRemoveCommunity = React.useCallback(
    async (community: Community, mode: "leave" | "local-only" = "leave") => {
      if (isLeaving) return;

      if (profileMenuHoverTimer.current !== null) {
        window.clearTimeout(profileMenuHoverTimer.current);
        profileMenuHoverTimer.current = null;
      }
      setIsLeaving(true);
      setLeaveError(null);
      if (mode === "leave") setUnavailableCommunityId(null);
      try {
        const result = await onRemoveCommunity(community.id, mode);
        setRemovingCommunity(null);
        setDropdownOpen(false);
        if (result?.status === "already-absent") {
          toast("Community removed", {
            description:
              "You were no longer a member, so Buzz removed the community from this device.",
          });
        }
      } catch (error) {
        if (mode === "leave" && canRemoveCommunityLocally(error)) {
          setUnavailableCommunityId(community.id);
        }
        setLeaveError(
          error instanceof Error
            ? error.message
            : "Couldn't remove the community. Try again.",
        );
        setDropdownOpen(true);
      } finally {
        setIsLeaving(false);
      }
    },
    [isLeaving, onRemoveCommunity],
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
        <CommunityEmojiIcon
          className={
            isProfileVariant
              ? "flex w-5 shrink-0 items-center justify-center rounded-md border border-sidebar-border/70 bg-sidebar-accent/40 text-2xs"
              : "flex w-5 shrink-0 items-center justify-center text-xs"
          }
          iconUrl={activeIcon}
        />
      )}
      <span
        className={
          degraded
            ? "min-w-0 flex-1 truncate font-medium text-destructive animate-pulse"
            : "min-w-0 flex-1 truncate font-medium"
        }
      >
        {activeCommunity?.name ?? "No community"}
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
                ? `${activeCommunity?.name ?? "Community"} — ${connectionLabel}`
                : "Community actions"
            }
            className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm text-popover-foreground outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none data-[state=open]:bg-muted/50 data-[state=open]:text-popover-foreground"
            data-testid="community-switcher"
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
          className="w-60 p-1"
          onMouseEnter={() => scheduleProfileMenu(true)}
          onMouseLeave={() => scheduleProfileMenu(false)}
          onOpenAutoFocus={(event) => event.preventDefault()}
          side="right"
          sideOffset={0}
        >
          <div
            aria-label="Community actions"
            data-testid="profile-community-actions"
            role="menu"
          >
            {activeCommunity ? (
              <>
                <button
                  className="flex min-h-9 w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none"
                  onClick={() => {
                    setDropdownOpen(false);
                    void writeTextToClipboard(activeCommunity.relayUrl);
                  }}
                  role="menuitem"
                  type="button"
                >
                  <Link2 className="h-4 w-4" />
                  <span>Copy community URL</span>
                </button>
                {canInvite && onInvite ? (
                  <button
                    className="flex min-h-9 w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none"
                    onClick={() => {
                      setDropdownOpen(false);
                      onInvite();
                    }}
                    role="menuitem"
                    type="button"
                  >
                    <Ticket className="h-4 w-4" />
                    <span>Invite to community</span>
                  </button>
                ) : null}
                <button
                  className="flex min-h-9 w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none"
                  onClick={() => {
                    setDropdownOpen(false);
                    setEditingCommunity(activeCommunity);
                  }}
                  role="menuitem"
                  type="button"
                >
                  <Settings2 className="h-4 w-4" />
                  <span>Community settings</span>
                </button>
                <button
                  className="flex min-h-9 w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm text-destructive outline-hidden transition-colors hover:bg-destructive/10 focus:bg-destructive/10 focus:outline-none focus-visible:bg-destructive/10 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
                  disabled={isLeaving}
                  onClick={() => void handleRemoveCommunity(activeCommunity)}
                  role="menuitem"
                  type="button"
                >
                  <LogOut className="h-4 w-4" />
                  <span>{isLeaving ? "Leaving…" : "Leave community"}</span>
                </button>
                {leaveError ? (
                  <p
                    className="px-3 py-1 text-xs text-destructive"
                    role="alert"
                  >
                    {leaveError}
                  </p>
                ) : null}
                {unavailableCommunityId === activeCommunity.id ? (
                  <button
                    className="flex min-h-9 w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm text-destructive outline-hidden transition-colors hover:bg-destructive/10 focus:bg-destructive/10 focus:outline-none focus-visible:bg-destructive/10 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
                    disabled={isLeaving}
                    onClick={() => {
                      clearProfileMenuHoverTimer();
                      setLeaveError(null);
                      setDropdownOpen(false);
                      setRemovingCommunity(activeCommunity);
                    }}
                    role="menuitem"
                    type="button"
                  >
                    <Unplug className="h-4 w-4" />
                    <span>Remove from this device</span>
                  </button>
                ) : null}
                <hr className="-mx-1 my-1 h-px border-0 bg-muted" />
              </>
            ) : null}
            <button
              className="flex min-h-9 w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none"
              onClick={() => {
                setDropdownOpen(false);
                onAddCommunity();
              }}
              role="menuitem"
              type="button"
            >
              <Plus className="h-4 w-4" />
              <span>Add a community</span>
            </button>
          </div>
        </PopoverContent>
      </Popover>
    ) : null;

  const switcherDropdown = (
    <DropdownMenu
      modal={false}
      open={dropdownOpen}
      onOpenChange={setDropdownOpen}
    >
      <DropdownMenuTrigger asChild>
        {variant === "profile" ? (
          <button
            aria-label={
              degraded
                ? `${activeCommunity?.name ?? "Community"} — ${connectionLabel}`
                : "Switch community"
            }
            className="flex min-w-0 max-w-full items-center gap-1.5 rounded-md py-0.5 text-left text-xs text-sidebar-foreground/50 outline-hidden transition-colors hover:text-sidebar-foreground focus:outline-none focus-visible:outline-none data-[state=open]:text-sidebar-foreground"
            data-testid="community-switcher"
            type="button"
          >
            {triggerContent}
          </button>
        ) : (
          <SidebarMenuButton
            aria-label={
              degraded
                ? `${activeCommunity?.name ?? "Community"} — ${connectionLabel}`
                : undefined
            }
            className="h-auto gap-2 rounded-xl px-2.5 py-2 data-[state=open]:bg-sidebar-accent"
            data-testid="community-switcher"
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
        {communities.map((community) => (
          <DropdownMenuItem
            key={community.id}
            className="group flex items-center gap-2 pr-1"
            onSelect={() => {
              onSwitchCommunity(community.id);
            }}
          >
            <span className="flex h-4 w-4 shrink-0 items-center justify-center">
              {activeCommunity?.id === community.id ? (
                <Check className="h-4 w-4 text-primary" />
              ) : null}
            </span>
            <span className="min-w-0 flex-1 truncate">{community.name}</span>
            <button
              aria-label={`Edit ${community.name}`}
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded opacity-0 hover:bg-accent group-hover:opacity-100 group-focus:opacity-100"
              onClick={(e) => {
                e.stopPropagation();
                e.preventDefault();
                setDropdownOpen(false);
                setEditingCommunity(community);
              }}
              type="button"
            >
              <MoreHorizontal className="h-4 w-4" />
            </button>
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={onAddCommunity}>
          <Plus className="h-4 w-4" />
          <span>Add a community</span>
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

      <AlertDialog
        open={removingCommunity !== null}
        onOpenChange={(open) => {
          if (!open && !isLeaving) setRemovingCommunity(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Remove {removingCommunity?.name} from this device?
            </AlertDialogTitle>
            <AlertDialogDescription>
              This removes the saved connection on this device without
              contacting the server. Your membership and the community on the
              server are unchanged. Your identity is kept. You can add the
              community again later.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {leaveError ? (
            <p className="text-sm text-destructive" role="alert">
              {leaveError}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={isLeaving}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={isLeaving}
              onClick={(event) => {
                event.preventDefault();
                if (removingCommunity) {
                  void handleRemoveCommunity(removingCommunity, "local-only");
                }
              }}
            >
              {isLeaving ? "Removing…" : "Remove from this device"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <EditCommunityDialog
        onOpenChange={(open) => {
          if (!open) setEditingCommunity(null);
        }}
        onSave={onUpdateCommunity}
        open={editingCommunity !== null}
        community={editingCommunity}
        showIconEditor={editingCommunity?.id === activeCommunity?.id}
      />
    </>
  );
}
