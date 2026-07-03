import { Plus } from "lucide-react";

import type { Workspace } from "@/features/workspaces/types";
import { useWorkspaceIcons } from "@/features/workspaces/useWorkspaceIcons";
import {
  useWorkspaceUnread,
  type WorkspaceUnreadState,
} from "@/features/workspaces/useWorkspaceUnread";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";

type WorkspaceRailProps = {
  workspaces: Workspace[];
  activeWorkspaceId: string | null;
  onSwitchWorkspace: (id: string) => void;
  onAddWorkspace: () => void;
};

const MAX_BADGE = 99;

// Strip punctuation before initials so "B (relay)" yields "BR", not "B(".
export function workspaceInitials(name: string): string {
  const cleaned = name.replace(/[^\p{L}\p{N}\s]/gu, " ");
  return getInitials(cleaned);
}

/**
 * Presentation decisions for one workspace button, derived from its observed
 * mention state. Pure so it can be unit-tested without a DOM. The `state` guard
 * ensures we NEVER render a mention badge for a relay we could not observe
 * (`unknown`/`loading`/`error`) — only a `ready` observation is trusted.
 */
export function workspaceRailIndicators(unread: WorkspaceUnreadState): {
  mentionCount: number;
  showBadge: boolean;
  pending: boolean;
  badgeLabel: string;
} {
  const observed = unread.state === "ready";
  const mentionCount = observed ? (unread.count ?? 0) : 0;
  const showBadge = mentionCount > 0;
  return {
    mentionCount,
    showBadge,
    pending: unread.state === "unknown" || unread.state === "loading",
    badgeLabel:
      mentionCount > MAX_BADGE ? `${MAX_BADGE}+` : String(mentionCount),
  };
}

function WorkspaceButton({
  workspace,
  isActive,
  unread,
  iconUrl,
  onSwitch,
}: {
  workspace: Workspace;
  isActive: boolean;
  unread: WorkspaceUnreadState;
  iconUrl: string | null;
  onSwitch: () => void;
}) {
  const { mentionCount, showBadge, pending, badgeLabel } =
    workspaceRailIndicators(unread);

  const tooltipLabel = showBadge
    ? `${workspace.name} — ${mentionCount} mention${mentionCount === 1 ? "" : "s"}`
    : workspace.name;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-current={isActive ? "true" : undefined}
          aria-label={tooltipLabel}
          className="relative flex h-9 w-9 items-center justify-center outline-hidden focus:outline-none focus-visible:outline-none"
          data-testid={`workspace-rail-button-${workspace.id}`}
          onClick={onSwitch}
          type="button"
        >
          <span
            className={cn(
              "flex h-9 w-9 items-center justify-center overflow-hidden rounded-2xl text-xs font-semibold transition-all",
              isActive
                ? "rounded-xl bg-primary text-primary-foreground"
                : "bg-sidebar-accent/60 text-sidebar-foreground/80 hover:rounded-xl hover:bg-primary/80 hover:text-primary-foreground",
              pending && "opacity-60",
            )}
          >
            {iconUrl ? (
              <img
                alt=""
                className="h-full w-full object-cover"
                data-testid={`workspace-rail-icon-${workspace.id}`}
                draggable={false}
                src={iconUrl}
              />
            ) : (
              workspaceInitials(workspace.name) || "🐝"
            )}
          </span>
          {showBadge ? (
            <span
              className="absolute -bottom-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold text-primary-foreground ring-2 ring-sidebar"
              data-testid={`workspace-rail-mentions-${workspace.id}`}
            >
              {badgeLabel}
            </span>
          ) : null}
        </button>
      </TooltipTrigger>
      <TooltipContent side="right">{tooltipLabel}</TooltipContent>
    </Tooltip>
  );
}

/**
 * Discord/Slack-style vertical rail of workspaces on the far left of the app.
 * Shows a mention-count badge for inactive workspaces (observed via
 * `useWorkspaceUnread`) and switches relays on click.
 *
 * Hidden entirely with a single workspace — a rail of one adds no value.
 */
export function WorkspaceRail({
  workspaces,
  activeWorkspaceId,
  onSwitchWorkspace,
  onAddWorkspace,
}: WorkspaceRailProps) {
  const unreadByWorkspace = useWorkspaceUnread(workspaces, activeWorkspaceId);
  const iconsByWorkspace = useWorkspaceIcons(workspaces);
  const isFullscreen = useIsFullscreen();
  if (workspaces.length <= 1) {
    return null;
  }

  // macOS traffic lights overlay the top-left, so start buttons below them (they hide in fullscreen).
  const topPaddingClass =
    isMacPlatform() && !isFullscreen
      ? "pt-(--buzz-top-chrome-height,2.5rem)"
      : "pt-3";

  return (
    <nav
      aria-label="Workspaces"
      className={cn(
        "flex w-12 shrink-0 flex-col items-center gap-2 overflow-y-auto bg-sidebar pb-3",
        topPaddingClass,
      )}
      data-testid="workspace-rail"
    >
      {workspaces.map((workspace) => (
        <WorkspaceButton
          key={workspace.id}
          iconUrl={iconsByWorkspace[workspace.id] ?? null}
          isActive={workspace.id === activeWorkspaceId}
          onSwitch={() => onSwitchWorkspace(workspace.id)}
          unread={
            unreadByWorkspace[workspace.id] ?? {
              hasUnread: false,
              state: "unknown",
            }
          }
          workspace={workspace}
        />
      ))}
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            aria-label="Add workspace"
            className="flex h-9 w-9 items-center justify-center rounded-2xl bg-sidebar-accent/60 text-sidebar-foreground/70 outline-hidden transition-all hover:rounded-xl hover:bg-primary/80 hover:text-primary-foreground focus:outline-none focus-visible:outline-none"
            data-testid="workspace-rail-add"
            onClick={onAddWorkspace}
            type="button"
          >
            <Plus className="h-4 w-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">Add workspace</TooltipContent>
      </Tooltip>
    </nav>
  );
}
