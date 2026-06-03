import * as React from "react";
import { ChevronRight, MessageSquare, Settings } from "lucide-react";

import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { SetStatusDialog } from "@/features/user-status/ui/SetStatusDialog";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { PresenceStatus } from "@/shared/api/types";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ProfilePopoverProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  displayName: string;
  nip05?: string | null;
  avatarUrl: string | null;
  currentStatus: PresenceStatus;
  isStatusPending?: boolean;
  userStatusText?: string;
  userStatusEmoji?: string;
  onSetStatus: (status: PresenceStatus) => void;
  onSetUserStatus: (text: string, emoji: string) => void;
  onClearUserStatus: () => void;
  onOpenSettings: () => void;
  children: React.ReactNode;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MENU_ITEM_CLASS =
  "flex w-full items-center gap-3 px-3 py-2.5 rounded-lg text-left hover:bg-accent cursor-pointer transition-colors";

const ALL_STATUSES: PresenceStatus[] = ["online", "away", "offline"];

// ---------------------------------------------------------------------------
// ProfilePopover
// ---------------------------------------------------------------------------

export function ProfilePopover({
  open,
  onOpenChange,
  displayName,
  nip05,
  avatarUrl,
  currentStatus,
  isStatusPending,
  userStatusText,
  userStatusEmoji,
  onSetStatus,
  onSetUserStatus,
  onClearUserStatus,
  onOpenSettings,
  children,
}: ProfilePopoverProps) {
  const isMac =
    typeof navigator !== "undefined" &&
    /Mac|iPod|iPhone|iPad/.test(navigator.userAgent);
  const [statusDialogOpen, setStatusDialogOpen] = React.useState(false);
  const [presenceMenuOpen, setPresenceMenuOpen] = React.useState(false);
  const presenceHoverTimer = React.useRef<number | null>(null);
  const hasUserStatus = Boolean(userStatusText || userStatusEmoji);

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
          sideOffset={8}
          className="w-[280px] rounded-xl border border-border bg-popover p-0 shadow-lg"
          data-testid="profile-popover"
        >
          <div aria-label="Profile menu" role="menu">
            {/* ── Identity block ─────────────────────────────────── */}
            <div className="flex items-center gap-3 px-4 py-3">
              <div className="relative shrink-0">
                <ProfileAvatar
                  avatarUrl={avatarUrl}
                  className="h-10 w-10 rounded-2xl text-sm"
                  iconClassName="h-5 w-5"
                  label={displayName}
                />
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-semibold text-popover-foreground">
                  {displayName}
                </p>
                <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  {nip05 ? <span className="truncate">@{nip05}</span> : null}
                  {nip05 ? <span aria-hidden="true">·</span> : null}
                  <span
                    className="inline-flex items-center gap-1.5"
                    data-testid="profile-popover-current-status"
                  >
                    <PresenceDot status={currentStatus} />
                    <span>{getPresenceLabel(currentStatus)}</span>
                  </span>
                </div>
                {hasUserStatus ? (
                  <p
                    className="mt-0.5 truncate text-xs text-muted-foreground"
                    data-testid="profile-popover-user-status"
                  >
                    {userStatusEmoji ? (
                      <StatusEmoji
                        className="mr-1 h-3.5 w-3.5"
                        value={userStatusEmoji}
                      />
                    ) : null}
                    {userStatusText}
                  </p>
                ) : null}
              </div>
            </div>

            <hr className="my-1 h-px border-0 bg-border" />

            {/* ── User status ──────────────────────────────────── */}
            <div className="px-1.5 py-1">
              <button
                className={MENU_ITEM_CLASS}
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
                <MessageSquare className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm text-popover-foreground">
                  {hasUserStatus ? "Update status" : "Set a status"}
                </span>
              </button>
            </div>

            <hr className="my-1 h-px border-0 bg-border" />

            {/* ── Presence status options ───────────────────────── */}
            <div className="px-1.5 py-1">
              <Popover
                onOpenChange={setPresenceMenuOpen}
                open={presenceMenuOpen}
              >
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
                    <PresenceDot
                      className="h-2.5 w-2.5"
                      status={currentStatus}
                    />
                    <span className="flex-1 text-sm text-popover-foreground">
                      {getPresenceLabel(currentStatus)}
                    </span>
                    <ChevronRight className="h-4 w-4 text-muted-foreground" />
                  </button>
                </PopoverTrigger>
                <PopoverContent
                  align="start"
                  className="w-44 rounded-xl border border-border bg-popover p-1.5 shadow-lg"
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
                        <span className="text-sm text-popover-foreground">
                          {getPresenceLabel(status)}
                        </span>
                      </button>
                    ))}
                  </div>
                </PopoverContent>
              </Popover>
            </div>

            <hr className="my-1 h-px border-0 bg-border" />

            {/* ── Settings ───────────────────────────────────────── */}
            <div className="px-1.5 py-1">
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
                <Settings className="h-4 w-4 text-muted-foreground" />
                <span className="flex-1 text-sm text-popover-foreground">
                  Settings
                </span>
                <kbd className="text-xs text-muted-foreground">
                  {isMac ? "⌘," : "Ctrl+,"}
                </kbd>
              </button>
            </div>
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
