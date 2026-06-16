import * as React from "react";

import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { useSelfProfileCache } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { ProfilePopover } from "@/features/profile/ui/ProfilePopover";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { Workspace } from "@/features/workspaces/types";
import { WorkspaceSwitcher } from "@/features/workspaces/ui/WorkspaceSwitcher";
import type { PresenceStatus, Profile, UserStatus } from "@/shared/api/types";
import { useReconnectRelay } from "@/shared/api/useReconnectRelay";
import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";
import { cn } from "@/shared/lib/cn";

type SidebarProfileCardProps = {
  activeWorkspace: Workspace | null;
  isPresencePending?: boolean;
  onOpenAddWorkspace: () => void;
  onOpenSettings: (section?: "profile" | "appearance") => void;
  onRemoveWorkspace: (id: string) => void;
  onSetPresenceStatus?: (status: PresenceStatus) => void;
  onSetUserStatus: (text: string, emoji: string) => void;
  onClearUserStatus: () => void;
  onSwitchWorkspace: (id: string) => void;
  onUpdateWorkspace: (
    id: string,
    updates: Partial<Pick<Workspace, "name" | "relayUrl" | "token">>,
  ) => void;
  profile?: Profile;
  resolvedDisplayName: string;
  selfPresenceStatus: PresenceStatus;
  selfUserStatus?: UserStatus;
  workspaces: Workspace[];
};

export function SidebarProfileCard({
  activeWorkspace,
  isPresencePending,
  onOpenAddWorkspace,
  onOpenSettings,
  onRemoveWorkspace,
  onSetPresenceStatus,
  onSetUserStatus,
  onClearUserStatus,
  onSwitchWorkspace,
  onUpdateWorkspace,
  profile,
  resolvedDisplayName,
  selfPresenceStatus,
  selfUserStatus,
  workspaces,
}: SidebarProfileCardProps) {
  // Called locally rather than threading props from AppShell — both hooks are
  // workspace-provider and QueryClient safe at this level.
  const selfProfileCache = useSelfProfileCache();
  const { isPending, reconnect } = useReconnectRelay();
  // Only offer reconnect when the relay is actually degraded — keep the item
  // visible while a reconnect is in flight so it does not vanish mid-click if
  // the live state briefly flips.
  const isRelayConnectionDegradedNow = isRelayConnectionDegraded(
    useRelayConnection(),
  );

  const [profilePopoverOpen, setProfilePopoverOpen] = React.useState(false);
  const profileCardRef = React.useRef<HTMLDivElement | null>(null);
  const toggleProfilePopover = React.useCallback(
    () => setProfilePopoverOpen((prev) => !prev),
    [],
  );
  const handleCardClick = React.useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      const target = event.target;
      if (
        !(target instanceof Node) ||
        !profileCardRef.current?.contains(target)
      ) {
        return;
      }
      toggleProfilePopover();
    },
    [toggleProfilePopover],
  );
  const hasStatus = Boolean(selfUserStatus?.text || selfUserStatus?.emoji);
  const workspaceLabel = activeWorkspace?.name ?? "No workspace";
  const readonlyWorkspaceLabel = (
    <span className="flex min-w-0 cursor-pointer items-center gap-1 text-xs leading-snug text-sidebar-foreground/70">
      <span aria-hidden="true" className="shrink-0 text-[10px] leading-none">
        🐝
      </span>
      <span className="truncate">{workspaceLabel}</span>
    </span>
  );

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions lint/a11y/useKeyWithClickEvents: child buttons provide keyboard access; wrapper fills pointer gaps between them.
    <div
      className="group/profile-card cursor-pointer rounded-xl px-2 py-2 transition-colors hover:bg-sidebar-border/35 dark:hover:bg-sidebar-border/30"
      data-testid="sidebar-profile-card"
      onClick={handleCardClick}
      ref={profileCardRef}
    >
      <div className="flex min-w-0 items-center gap-3">
        <button
          aria-label={`Open profile menu for ${resolvedDisplayName}`}
          className="relative shrink-0 rounded-xl outline-hidden focus:outline-none focus-visible:outline-none"
          data-testid="sidebar-profile-avatar-button"
          onClick={(event) => {
            event.stopPropagation();
            toggleProfilePopover();
          }}
          type="button"
        >
          <ProfileAvatar
            avatarDataUrl={selfProfileCache?.avatarDataUrl ?? null}
            avatarUrl={profile?.avatarUrl ?? null}
            className="h-8 w-8 text-xs"
            iconClassName="h-4 w-4"
            label={resolvedDisplayName}
            testId="sidebar-profile-avatar"
          />
          <span
            aria-label={getPresenceLabel(selfPresenceStatus)}
            className="absolute -bottom-0.5 -right-0.5 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-sidebar"
            data-testid="self-presence-badge"
            role="img"
          >
            <PresenceDot className="h-2 w-2" status={selfPresenceStatus} />
          </span>
        </button>

        <div className="min-w-0 flex-1">
          <ProfilePopover
            open={profilePopoverOpen}
            onOpenChange={setProfilePopoverOpen}
            avatarDataUrl={selfProfileCache?.avatarDataUrl ?? null}
            avatarUrl={profile?.avatarUrl ?? null}
            currentStatus={selfPresenceStatus}
            displayName={resolvedDisplayName}
            isReconnectPending={isPending}
            isStatusPending={isPresencePending}
            onClearUserStatus={onClearUserStatus}
            onOpenSettings={onOpenSettings}
            onReconnect={
              isRelayConnectionDegradedNow || isPending
                ? () => void reconnect()
                : undefined
            }
            onSetStatus={onSetPresenceStatus ?? (() => {})}
            onSetUserStatus={onSetUserStatus}
            triggerContainerRef={profileCardRef}
            userStatusEmoji={selfUserStatus?.emoji}
            userStatusText={selfUserStatus?.text}
            workspaceSwitcherSlot={
              <WorkspaceSwitcher
                activeWorkspace={activeWorkspace}
                onAddWorkspace={onOpenAddWorkspace}
                onRemoveWorkspace={onRemoveWorkspace}
                onSwitchWorkspace={onSwitchWorkspace}
                onUpdateWorkspace={onUpdateWorkspace}
                variant="profile-menu"
                workspaces={workspaces}
              />
            }
          >
            <button
              onClick={(event) => {
                event.stopPropagation();
                toggleProfilePopover();
              }}
              className="block w-full min-w-0 rounded-sm text-left text-sidebar-foreground outline-hidden focus:outline-none focus-visible:outline-none"
              data-testid="open-settings"
              type="button"
            >
              <p
                className="truncate text-sm font-semibold leading-tight text-current"
                data-testid="sidebar-profile-name"
              >
                {resolvedDisplayName}
              </p>
            </button>
          </ProfilePopover>

          {hasStatus ? (
            <div className="relative mt-0.5">
              <button
                aria-label={`Open profile menu for ${resolvedDisplayName}`}
                className={cn(
                  "flex w-full min-w-0 items-center truncate rounded-sm text-left text-xs leading-snug text-sidebar-foreground/70 outline-hidden transition-opacity duration-150 focus:outline-none focus-visible:outline-none group-hover/profile-card:opacity-0",
                  profilePopoverOpen && "opacity-100",
                )}
                data-testid="sidebar-profile-user-status"
                onClick={(event) => {
                  event.stopPropagation();
                  toggleProfilePopover();
                }}
                type="button"
              >
                {selfUserStatus?.emoji ? (
                  <StatusEmoji
                    className="mr-1 h-3.5 w-3.5"
                    value={selfUserStatus.emoji}
                  />
                ) : null}
                <span className="truncate">{selfUserStatus?.text}</span>
              </button>
              <div
                className={cn(
                  "pointer-events-none absolute inset-0 flex min-w-0 items-center text-xs leading-snug text-sidebar-foreground/70 opacity-0 transition-opacity duration-150 group-hover/profile-card:opacity-100",
                  profilePopoverOpen && "opacity-0",
                )}
              >
                {readonlyWorkspaceLabel}
              </div>
            </div>
          ) : (
            <div className="relative mt-0.5">{readonlyWorkspaceLabel}</div>
          )}
        </div>
      </div>
    </div>
  );
}
