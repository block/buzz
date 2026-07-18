import { CheckCheck, Link2, Plus, Settings2 } from "lucide-react";
import * as React from "react";

import type { Community } from "@/features/communities/types";
import { EditCommunityDialog } from "@/features/communities/ui/EditCommunityDialog";
import { useCommunityIcons } from "@/features/communities/useCommunityIcons";
import {
  useCommunityUnread,
  type CommunityUnreadState,
} from "@/features/communities/useCommunityUnread";
import { useAppShell } from "@/app/AppShellContext";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { cn } from "@/shared/lib/cn";
import { getInitials } from "@/shared/lib/initials";
import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type CommunityRailProps = {
  communities: Community[];
  activeCommunityId: string | null;
  onSwitchCommunity: (id: string) => void;
  onAddCommunity: () => void;
  onUpdateCommunity: (
    id: string,
    updates: Partial<Pick<Community, "name" | "relayUrl" | "token">>,
  ) => void;
  onRemoveCommunity: (id: string) => void;
};

const MAX_BADGE = 99;

/**
 * Presentation decisions for one community button, derived from its observed
 * mention state. Pure so it can be unit-tested without a DOM. The `state` guard
 * ensures we NEVER render any indicator for a relay we could not observe
 * (`unknown`/`loading`/`error`) — only a `ready` observation is trusted.
 *
 * Two-tier indicator system:
 * - `showBadge`: numeric mention count (mentions/thread-replies present).
 * - `showDot`: plain unread dot when there are regular channel unreads but no
 *   mentions. Mutually exclusive with `showBadge` by construction.
 */
export function communityRailIndicators(unread: CommunityUnreadState): {
  mentionCount: number;
  showBadge: boolean;
  showDot: boolean;
  pending: boolean;
  badgeLabel: string;
} {
  const observed = unread.state === "ready";
  const mentionCount = observed ? (unread.count ?? 0) : 0;
  const showBadge = mentionCount > 0;
  const showDot = observed && unread.hasUnread && !showBadge;
  return {
    mentionCount,
    showBadge,
    showDot,
    pending: unread.state === "unknown" || unread.state === "loading",
    badgeLabel:
      mentionCount > MAX_BADGE ? `${MAX_BADGE}+` : String(mentionCount),
  };
}

function CommunityButton({
  community,
  isActive,
  unread,
  iconUrl,
  onSwitch,
  menu,
}: {
  community: Community;
  isActive: boolean;
  unread: CommunityUnreadState;
  iconUrl: string | null;
  onSwitch: () => void;
  menu: React.ReactNode;
}) {
  const { mentionCount, showBadge, showDot, pending, badgeLabel } =
    communityRailIndicators(unread);

  const tooltipLabel = showBadge
    ? `${community.name} — ${mentionCount} mention${mentionCount === 1 ? "" : "s"}`
    : showDot
      ? `${community.name} — unread`
      : community.name;

  return (
    <ContextMenu modal={false}>
      <Tooltip>
        <TooltipTrigger asChild>
          <ContextMenuTrigger asChild>
            <button
              aria-current={isActive ? "true" : undefined}
              aria-label={tooltipLabel}
              className="relative flex h-9 w-9 items-center justify-center outline-hidden focus:outline-none focus-visible:outline-none"
              data-testid={`community-rail-button-${community.id}`}
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
                    data-testid={`community-rail-icon-${community.id}`}
                    draggable={false}
                    src={iconUrl}
                  />
                ) : (
                  getInitials(community.name) || "🐝"
                )}
              </span>
              {showBadge ? (
                <span
                  className="absolute -bottom-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold text-primary-foreground ring-2 ring-sidebar"
                  data-testid={`community-rail-mentions-${community.id}`}
                >
                  {badgeLabel}
                </span>
              ) : showDot ? (
                <span
                  className="absolute -bottom-0.5 -right-0.5 h-2 w-2 shrink-0 rounded-full bg-primary ring-2 ring-sidebar"
                  data-testid={`community-rail-unread-dot-${community.id}`}
                >
                  <span className="sr-only">unread</span>
                </span>
              ) : null}
            </button>
          </ContextMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="right">{tooltipLabel}</TooltipContent>
      </Tooltip>
      <ContextMenuContent data-testid={`community-rail-menu-${community.id}`}>
        {menu}
      </ContextMenuContent>
    </ContextMenu>
  );
}

/**
 * Discord/Slack-style vertical rail of communities on the far left of the app.
 * Shows a mention-count badge for inactive communities (observed via
 * `useCommunityUnread`) and switches relays on click. Right-click opens a
 * per-community menu: mark all as read, copy relay URL, community settings.
 *
 * Hidden entirely with a single community — a rail of one adds no value.
 */
export function CommunityRail({
  communities,
  activeCommunityId,
  onSwitchCommunity,
  onAddCommunity,
  onUpdateCommunity,
  onRemoveCommunity,
}: CommunityRailProps) {
  const { unreadByCommunity, markCommunityRead } = useCommunityUnread(
    communities,
    activeCommunityId,
  );
  const iconsByCommunity = useCommunityIcons(communities);
  const isFullscreen = useIsFullscreen();
  const { markAllChannelsRead } = useAppShell();
  const [editingCommunity, setEditingCommunity] =
    React.useState<Community | null>(null);
  if (communities.length <= 1) {
    return null;
  }

  const handleMarkAllRead = (community: Community) => {
    if (community.id === activeCommunityId) {
      markAllChannelsRead();
      return;
    }
    markCommunityRead(community.id).catch((error) => {
      console.warn(
        `[CommunityRail] mark all read failed community=${community.id}:`,
        error,
      );
    });
  };

  // macOS traffic lights overlay the top-left, so start buttons below them (they hide in fullscreen).
  const topPaddingClass =
    isMacPlatform() && !isFullscreen
      ? "pt-(--buzz-top-chrome-height,40px)"
      : "pt-3";

  return (
    <nav
      aria-label="Communities"
      className={cn(
        "flex w-12 shrink-0 flex-col items-center gap-2 overflow-y-auto bg-sidebar pb-3",
        topPaddingClass,
      )}
      data-testid="community-rail"
    >
      {communities.map((community) => (
        <CommunityButton
          key={community.id}
          iconUrl={iconsByCommunity[community.id] ?? null}
          isActive={community.id === activeCommunityId}
          menu={
            <>
              <ContextMenuItem onClick={() => handleMarkAllRead(community)}>
                <CheckCheck className="h-4 w-4" />
                Mark all as read
              </ContextMenuItem>
              <ContextMenuItem
                onClick={() => {
                  void writeTextToClipboard(community.relayUrl);
                }}
              >
                <Link2 className="h-4 w-4" />
                Copy relay URL
              </ContextMenuItem>
              <ContextMenuSeparator />
              <ContextMenuItem onClick={() => setEditingCommunity(community)}>
                <Settings2 className="h-4 w-4" />
                Community settings
              </ContextMenuItem>
            </>
          }
          onSwitch={() => onSwitchCommunity(community.id)}
          unread={
            unreadByCommunity[community.id] ?? {
              hasUnread: false,
              state: "unknown",
            }
          }
          community={community}
        />
      ))}
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            aria-label="Add community"
            className="flex h-9 w-9 items-center justify-center rounded-2xl bg-sidebar-accent/60 text-sidebar-foreground/70 outline-hidden transition-all hover:rounded-xl hover:bg-primary/80 hover:text-primary-foreground focus:outline-none focus-visible:outline-none"
            data-testid="community-rail-add"
            onClick={onAddCommunity}
            type="button"
          >
            <Plus className="h-4 w-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="right">Add community</TooltipContent>
      </Tooltip>
      <EditCommunityDialog
        canRemove={communities.length > 1}
        onOpenChange={(open) => {
          if (!open) setEditingCommunity(null);
        }}
        onRemove={onRemoveCommunity}
        onSave={onUpdateCommunity}
        open={editingCommunity !== null}
        community={editingCommunity}
      />
    </nav>
  );
}
