import * as React from "react";
import { Smile } from "lucide-react";

import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  MaskedAvatarBadgeFrame,
  STATUS_DOT_MASK_CURVE,
} from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import {
  getPresenceChipClassName,
  getPresenceLabel,
} from "@/features/presence/lib/presence";
import { SetStatusDialog } from "@/features/user-status/ui/SetStatusDialog";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";

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
  onSendFeedback?: () => void;
  children: React.ReactNode;
  // Optional outer container whose clicks should NOT close the popover.
  // Used when auxiliary triggers (avatar, status text) live alongside the
  // primary PopoverTrigger and toggle the popover via controlled `open`.
  triggerContainerRef?: React.RefObject<HTMLElement | null>;
  // Optional slot rendered between the identity block and the menu items.
  // Used by the sidebar to surface the community/relay selector inside the
  // profile menu instead of on the sidebar card.
  communitySwitcherSlot?: React.ReactNode;
}

const MENU_ITEM_CLASS =
  "flex min-h-9 w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-2 text-left text-sm text-popover-foreground outline-hidden transition-colors hover:bg-muted/50 focus:outline-none focus-visible:bg-muted/50 focus-visible:outline-none";

const ALL_STATUSES: PresenceStatus[] = ["online", "away", "offline"];

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
  onSendFeedback,
  children,
  triggerContainerRef,
  communitySwitcherSlot,
}: ProfilePopoverProps) {
  const [statusDialogOpen, setStatusDialogOpen] = React.useState(false);
  const [presenceMenuOpen, setPresenceMenuOpen] = React.useState(false);
  const hasUserStatus = Boolean(userStatusText || userStatusEmoji);
  const settingsShortcutLabel = isMacPlatform() ? "⌘," : "Ctrl+,";

  function handlePopoverOpenChange(nextOpen: boolean) {
    if (!nextOpen) {
      setPresenceMenuOpen(false);
    }
    onOpenChange(nextOpen);
  }

  function closePopover() {
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
              <MaskedAvatarBadgeFrame
                badge={
                  <span
                    aria-label={getPresenceLabel(currentStatus)}
                    className="flex h-3.5 w-3.5 items-center justify-center rounded-full"
                    data-testid="profile-popover-current-status"
                    role="img"
                  >
                    <PresenceDot className="h-2 w-2" status={currentStatus} />
                  </span>
                }
                badgeBox={{ bottom: -2, height: 14, right: -2, width: 14 }}
                className="h-8 w-8"
                curve={STATUS_DOT_MASK_CURVE}
                cutout={{ cx: 28, cy: 28, r: 7.5 }}
                size={32}
              >
                <ProfileAvatar
                  avatarDataUrl={avatarDataUrl}
                  avatarUrl={avatarUrl}
                  className="h-full w-full text-xs"
                  iconClassName="h-4 w-4"
                  label={displayName}
                />
              </MaskedAvatarBadgeFrame>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-semibold leading-tight text-popover-foreground">
                  {displayName}
                </p>
                {/* ── Presence chip (opens status chooser) ─────────── */}
                <Popover
                  onOpenChange={setPresenceMenuOpen}
                  open={presenceMenuOpen}
                >
                  <PopoverTrigger asChild>
                    <button
                      aria-expanded={presenceMenuOpen}
                      aria-haspopup="menu"
                      className={cn(
                        "mt-0.5 inline-flex max-w-full items-center rounded-md px-2 py-0.5 text-xs font-medium outline-hidden transition-opacity hover:opacity-80 focus-visible:opacity-80 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
                        getPresenceChipClassName(currentStatus),
                      )}
                      data-testid="profile-popover-presence-trigger"
                      disabled={isStatusPending}
                      onClick={() => setPresenceMenuOpen((prev) => !prev)}
                      role="menuitem"
                      type="button"
                    >
                      <span className="truncate">
                        {getPresenceLabel(currentStatus)}
                      </span>
                    </button>
                  </PopoverTrigger>
                  <PopoverContent
                    align="start"
                    className="w-52 p-1"
                    side="bottom"
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
                          <PresenceDot
                            className="h-2.5 w-2.5"
                            status={status}
                          />
                          <span>{getPresenceLabel(status)}</span>
                        </button>
                      ))}
                    </div>
                  </PopoverContent>
                </Popover>
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

            {onSendFeedback ? (
              <button
                className={MENU_ITEM_CLASS}
                data-testid="profile-popover-send-feedback"
                onClick={() => {
                  closePopover();
                  window.requestAnimationFrame(() => {
                    onSendFeedback();
                  });
                }}
                role="menuitem"
                type="button"
              >
                <span className="flex-1">Send feedback</span>
              </button>
            ) : null}

            {communitySwitcherSlot ? (
              <>
                <hr className="my-1 h-px border-0 bg-border" />
                {/* ── Community / relay selector ─────────────────── */}
                <div data-testid="profile-popover-community">
                  {communitySwitcherSlot}
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
