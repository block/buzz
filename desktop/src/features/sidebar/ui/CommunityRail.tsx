import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
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
  onReorderCommunities: (orderedIds: string[]) => void;
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
  dragListeners,
  dragAttributes,
  isDragging,
}: {
  community: Community;
  isActive: boolean;
  unread: CommunityUnreadState;
  iconUrl: string | null;
  onSwitch: () => void;
  menu: React.ReactNode;
  dragListeners?: React.HTMLAttributes<HTMLElement>;
  dragAttributes?: React.HTMLAttributes<HTMLElement>;
  isDragging?: boolean;
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
              className={cn(
                "relative flex h-9 w-9 items-center justify-center touch-none outline-hidden focus:outline-none focus-visible:outline-none",
                isDragging && "opacity-30",
              )}
              data-testid={`community-rail-button-${community.id}`}
              onClick={onSwitch}
              type="button"
              {...dragAttributes}
              {...dragListeners}
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

function CommunityDragOverlay({
  community,
  iconUrl,
}: {
  community: Community;
  iconUrl: string | null;
}) {
  return (
    <div
      className="flex h-9 w-9 cursor-grabbing items-center justify-center overflow-hidden rounded-xl bg-primary text-xs font-semibold text-primary-foreground opacity-90 shadow-lg ring-1 ring-sidebar-border"
      data-buzz-flat
    >
      {iconUrl ? (
        <img
          alt=""
          className="h-full w-full object-cover"
          draggable={false}
          src={iconUrl}
        />
      ) : (
        getInitials(community.name) || "🐝"
      )}
    </div>
  );
}

function SortableCommunityButton({
  community,
  activeCommunityId,
  iconsByCommunity,
  unreadByCommunity,
  onSwitchCommunity,
  onMarkAllRead,
  onSetEditingCommunity,
}: {
  community: Community;
  activeCommunityId: string | null;
  iconsByCommunity: Record<string, string | null | undefined>;
  unreadByCommunity: Record<string, CommunityUnreadState>;
  onSwitchCommunity: (id: string) => void;
  onMarkAllRead: (community: Community) => void;
  onSetEditingCommunity: (community: Community) => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: community.id });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div ref={setNodeRef} style={style}>
      <CommunityButton
        community={community}
        dragAttributes={attributes}
        dragListeners={listeners}
        iconUrl={iconsByCommunity[community.id] ?? null}
        isActive={community.id === activeCommunityId}
        isDragging={isDragging}
        menu={
          <>
            <ContextMenuItem onClick={() => onMarkAllRead(community)}>
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
            <ContextMenuItem onClick={() => onSetEditingCommunity(community)}>
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
      />
    </div>
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
  onReorderCommunities,
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
  const [draggingId, setDraggingId] = React.useState<string | null>(null);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  if (communities.length <= 1) {
    return null;
  }

  const communityIds = communities.map((c) => c.id);
  const draggingCommunity = draggingId
    ? (communities.find((c) => c.id === draggingId) ?? null)
    : null;

  const handleDragStart = (event: DragStartEvent) => {
    setDraggingId(event.active.id as string);
  };

  const handleDragEnd = (event: DragEndEvent) => {
    setDraggingId(null);
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIdx = communityIds.indexOf(active.id as string);
    const newIdx = communityIds.indexOf(over.id as string);
    if (oldIdx !== -1 && newIdx !== -1) {
      onReorderCommunities(arrayMove(communityIds, oldIdx, newIdx));
    }
  };

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
      <DndContext
        onDragEnd={handleDragEnd}
        onDragStart={handleDragStart}
        sensors={sensors}
      >
        <SortableContext
          items={communityIds}
          strategy={verticalListSortingStrategy}
        >
          {communities.map((community) => (
            <SortableCommunityButton
              key={community.id}
              activeCommunityId={activeCommunityId}
              community={community}
              iconsByCommunity={iconsByCommunity}
              unreadByCommunity={unreadByCommunity}
              onMarkAllRead={handleMarkAllRead}
              onSetEditingCommunity={setEditingCommunity}
              onSwitchCommunity={onSwitchCommunity}
            />
          ))}
        </SortableContext>
        <DragOverlay>
          {draggingCommunity ? (
            <CommunityDragOverlay
              community={draggingCommunity}
              iconUrl={iconsByCommunity[draggingCommunity.id] ?? null}
            />
          ) : null}
        </DragOverlay>
      </DndContext>
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
