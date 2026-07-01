import { Plus } from "lucide-react";

import type { Workspace } from "@/features/workspaces/types";
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

/**
 * Workspace-button initials. Strips punctuation/symbols first so a name like
 * "B (relay)" yields "BR", not "B(".
 */
export function workspaceInitials(name: string): string {
  const cleaned = name.replace(/[^\p{L}\p{N}\s]/gu, " ");
  return getInitials(cleaned);
}

/**
 * Presentation decisions for one workspace button, derived from its observed
 * unread state. Pure so it can be unit-tested without a DOM. The `state` guard
 * ensures we NEVER render a "no unread" affordance for a relay we could not
 * observe (`unknown`/`loading`/`error`) — only a `ready` observation is trusted.
 */
export function workspaceRailIndicators(unread: WorkspaceUnreadState): {
  showDot: boolean;
  mentionCount: number;
  showBadge: boolean;
  pending: boolean;
  badgeLabel: string;
} {
  const observed = unread.state === "ready";
  const mentionCount = observed ? (unread.count ?? 0) : 0;
  const showBadge = mentionCount > 0;
  return {
    showDot: observed && unread.hasUnread,
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
  onSwitch,
}: {
  workspace: Workspace;
  isActive: boolean;
  unread: WorkspaceUnreadState;
  onSwitch: () => void;
}) {
  const { showDot, mentionCount, showBadge, pending, badgeLabel } =
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
          className="relative flex h-10 w-10 items-center justify-center outline-hidden focus:outline-none focus-visible:outline-none"
          data-testid={`workspace-rail-button-${workspace.id}`}
          onClick={onSwitch}
          type="button"
        >
          {/* Active/unread indicator pill on the left edge (Discord-style). */}
          <span
            aria-hidden="true"
            className={cn(
              "absolute -left-2 w-1 rounded-r-full bg-sidebar-foreground transition-all",
              isActive
                ? "h-6 opacity-100"
                : showDot
                  ? "h-2 opacity-100"
                  : "h-2 opacity-0",
            )}
          />
          <span
            className={cn(
              "flex h-9 w-9 items-center justify-center rounded-2xl text-xs font-semibold transition-all",
              isActive
                ? "rounded-xl bg-primary text-primary-foreground"
                : "bg-sidebar-accent/60 text-sidebar-foreground/80 hover:rounded-xl hover:bg-primary/80 hover:text-primary-foreground",
              pending && "opacity-60",
            )}
          >
            {workspaceInitials(workspace.name) || "🐝"}
          </span>
          {showBadge ? (
            <span
              className="absolute -bottom-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-2xs font-semibold text-destructive-foreground ring-2 ring-sidebar"
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
 * Shows an unread dot and a mention-count badge for inactive workspaces
 * (observed via `useWorkspaceUnread`) and switches relays on click.
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
        "flex w-14 shrink-0 flex-col items-center gap-2 overflow-y-auto bg-sidebar pb-3",
        topPaddingClass,
      )}
      data-testid="workspace-rail"
    >
      {workspaces.map((workspace) => (
        <WorkspaceButton
          key={workspace.id}
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
