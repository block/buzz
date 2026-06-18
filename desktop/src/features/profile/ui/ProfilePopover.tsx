import * as React from "react";
import { ChevronRight, Smile } from "lucide-react";

import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { SetStatusDialog } from "@/features/user-status/ui/SetStatusDialog";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { PresenceStatus } from "@/shared/api/types";
import { isMacPlatform } from "@/shared/lib/platform";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ProfilePopoverProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  displayName: string;
  avatarUrl: string | null;
  avatarDataUrl?: string | null;
  currentStatus: PresenceStatus;
  isStatusPending?: boolean;
  userStatusText?: string;
  userStatusEmoji?: string;
  onSetStatus: (status: PresenceStatus) => void;
  onSetUserStatus: (text: string, emoji: string) => void;
  onClearUserStatus: () => void;
  onOpenSettings: (section?: "profile" | "appearance") => void;
  children: React.ReactNode;
  // Optional outer container whose clicks should NOT close the popover.
  // Used when auxiliary triggers (avatar, status text) live alongside the
  // primary PopoverTrigger and toggle the popover via controlled `open`.
  triggerContainerRef?: React.RefObject<HTMLElement | null>;
  // Optional slot rendered between the identity block and the menu items.
  // Used by the sidebar to surface the workspace/relay selector inside the
  // profile menu instead of on the sidebar card.
  workspaceSwitcherSlot?: React.ReactNode;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MENU_ITEM_CLASS =
  "flex min-h-9 w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-2 text-left text-sm text-popover-foreground outline-hidden transition-colors hover:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none";

const ALL_STATUSES: PresenceStatus[] = ["online", "away", "offline"];

// ---------------------------------------------------------------------------
// ProfilePopover
// ---------------------------------------------------------------------------

export function ProfilePopover({
  open,
  onOpenChange,
  displayName,
  avatarUrl,
  avatarDataUrl,
  currentStatus,
  isStatusPending,
  userStatusText,
  userStatusEmoji,
  onSetStatus,
  onSetUserStatus,
  onClearUserStatus,
  onOpenSettings,
  children,
  triggerContainerRef,
  workspaceSwitcherSlot,
}: ProfilePopoverProps) {
  const [statusDialogOpen, setStatusDialogOpen] = React.useState(false);
  const [presenceMenuOpen, setPresenceMenuOpen] = React.useState(false);
  const presenceHoverTimer = React.useRef<number | null>(null);
  const hasUserStatus = Boolean(userStatusText || userStatusEmoji);
  const settingsShortcutLabel = isMacPlatform() ? "⌘," : "Ctrl+,";

  function clearPresenceHoverTimer() {
    if (presenceHoverTimer.current !== null) {
      window.clearTimeout(presenceHoverTimer.current);
      presenceHoverTimer.current = null;
    }
  }

  function schedulePresenceMenu(nextOpen: boolean) {
    clearPresenceHoverTimer();
    presenceHoverTimer.current = window.setTimeout(
      () => setPresenceMenuOpen(nextOpen),
      nextOpen ? 80 : 160,
    );
  }

  React.useEffect(
    () => () => {
      if (presenceHoverTimer.current !== null) {
        window.clearTimeout(presenceHoverTimer.current);
      }
    },
    [],
  );

  function handlePopoverOpenChange(nextOpen: boolean) {
    if (!nextOpen) {
      setPresenceMenuOpen(false);
    }
    onOpenChange(nextOpen);
  }

  function closePopover() {
    clearPresenceHoverTimer();
    setPresenceMenuOpen(false);
    onOpenChange(false);
  }

  function handlePresenceSelect(status: PresenceStatus) {
    onSetStatus(status);
    closePopover();
  }

  return (
    <>
      <Popover open={open} onOpenChange={handlePopoverOpenChange}>
        <PopoverTrigger asChild>{children}</PopoverTrigger>

        <PopoverContent
          side="top"
          align="start"
          sideOffset={-32}
          className="w-[280px] p-1"
          data-testid="profile-popover"
          onInteractOutside={(event) => {
            const target = event.target as Node | null;
            if (target && triggerContainerRef?.current?.contains(target)) {
              // Click on an auxiliary trigger inside the same card
              // (e.g. avatar or status) — let that trigger toggle the
              // controlled state instead of auto-closing here.
              event.preventDefault();
            }
          }}
        >
          <div aria-label="Profile menu" role="menu">
            {/* ── Identity block ─────────────────────────────────── */}
            <div className="flex items-center gap-2 px-3 pt-2 pb-2">
              <div className="relative shrink-0">
                <ProfileAvatar
                  avatarDataUrl={avatarDataUrl}
                  avatarUrl={avatarUrl}
                  className="h-8 w-8 text-xs"
                  iconClassName="h-4 w-4"
                  label={displayName}
                />
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-semibold leading-tight text-popover-foreground">
                  {displayName}
                </p>
                <div
                  className="inline-flex items-center gap-1.5 text-xs text-muted-foreground"
                  data-testid="profile-popover-current-status"
                >
                  <PresenceDot status={currentStatus} />
                  <span>{getPresenceLabel(currentStatus)}</span>
                </div>
              </div>
            </div>

            {/* ── Status input (Slack-style) ──────────────────────── */}
            <div className="px-2 pt-0 pb-1">
              <button
                className="flex w-full items-center gap-2 rounded-lg border border-border/60 bg-transparent px-3 py-2 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none"
                data-testid="profile-popover-set-status"
                onClick={() => {
                  closePopover();
                  window.requestAnimationFrame(() => {
                    setStatusDialogOpen(true);
                  });
                }}
                role="menuitem"
                type="button"
              >
                <Smile className="h-4 w-4 shrink-0 text-muted-foreground" />
                {hasUserStatus ? (
                  <span className="flex min-w-0 flex-1 items-center gap-1 truncate text-popover-foreground">
                    {userStatusEmoji ? (
                      <StatusEmoji
                        className="w-5 shrink-0 text-base"
                        value={userStatusEmoji}
                      />
                    ) : null}
                    <span className="truncate">{userStatusText}</span>
                  </span>
                ) : (
                  <span className="flex-1 truncate text-muted-foreground">
                    Update your status
                  </span>
                )}
              </button>
            </div>

            {/* ── Presence ────────────────────────────────────────── */}
            <Popover onOpenChange={setPresenceMenuOpen} open={presenceMenuOpen}>
              <PopoverTrigger asChild>
                <button
                  aria-expanded={presenceMenuOpen}
                  aria-haspopup="menu"
                  className={MENU_ITEM_CLASS}
                  data-testid="profile-popover-presence-trigger"
                  disabled={isStatusPending}
                  onClick={() => {
                    clearPresenceHoverTimer();
                    setPresenceMenuOpen((prev) => !prev);
                  }}
                  onMouseEnter={() => schedulePresenceMenu(true)}
                  onMouseLeave={() => schedulePresenceMenu(false)}
                  role="menuitem"
                  type="button"
                >
                  <PresenceDot className="h-2.5 w-2.5" status={currentStatus} />
                  <span className="flex-1">
                    {getPresenceLabel(currentStatus)}
                  </span>
                  <ChevronRight className="h-4 w-4 text-muted-foreground" />
                </button>
              </PopoverTrigger>
              <PopoverContent
                align="start"
                className="w-60 p-1"
                onMouseEnter={() => schedulePresenceMenu(true)}
                onMouseLeave={() => schedulePresenceMenu(false)}
                side="right"
                sideOffset={4}
              >
                <div aria-label="Presence status" role="menu">
                  {ALL_STATUSES.map((status) => (
                    <button
                      className={MENU_ITEM_CLASS}
                      data-testid={`profile-popover-status-${status}`}
                      disabled={isStatusPending}
                      key={status}
                      onClick={() => handlePresenceSelect(status)}
                      role="menuitem"
                      type="button"
                    >
                      <PresenceDot className="h-2.5 w-2.5" status={status} />
                      <span>{getPresenceLabel(status)}</span>
                    </button>
                  ))}
                </div>
              </PopoverContent>
            </Popover>

            <hr className="my-1 h-px border-0 bg-border" />

            {/* ── Settings ───────────────────────────────────────── */}
            <button
              className={MENU_ITEM_CLASS}
              data-testid="profile-popover-settings"
              onClick={() => {
                closePopover();
                window.requestAnimationFrame(() => {
                  onOpenSettings();
                });
              }}
              role="menuitem"
              type="button"
            >
              <span className="flex-1">Settings</span>
              <kbd className="text-xs text-muted-foreground">
                {settingsShortcutLabel}
              </kbd>
            </button>

            {workspaceSwitcherSlot ? (
              <>
                <hr className="my-1 h-px border-0 bg-border" />
                {/* ── Workspace / relay selector ─────────────────── */}
                <div data-testid="profile-popover-workspace">
                  {workspaceSwitcherSlot}
                </div>
              </>
            ) : null}
          </div>
        </PopoverContent>
      </Popover>

      <SetStatusDialog
        hasExistingStatus={hasUserStatus}
        initialEmoji={userStatusEmoji}
        initialText={userStatusText}
        onClear={onClearUserStatus}
        onOpenChange={setStatusDialogOpen}
        onSave={onSetUserStatus}
        open={statusDialogOpen}
      />
    </>
  );
}
