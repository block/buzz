import * as React from "react";
import { BellOff, ChevronDown, CircleDot, X, Pencil } from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";

import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { ChannelContextMenuItems } from "@/features/sidebar/ui/ChannelContextMenu";
import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { ChannelWorkingBadge } from "@/features/sidebar/ui/channelWorkingBadge";
import { ChannelGlyph } from "@/features/channels/ui/ChannelGlyph";
import { getEphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { EphemeralChannelBadge } from "@/features/channels/ui/EphemeralChannelBadge";
import {
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  ProfileAvatarWithStatus,
  scaleProfileAvatarStatusGeometry,
} from "@/features/profile/ui/ProfileAvatarWithStatus";
import type { Channel, PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useDraftsSnapshot } from "@/features/messages/lib/useDrafts";
import { channelShowsDraftIndicator } from "@/features/sidebar/lib/channelDraftIndicator";
import { useLocation } from "@tanstack/react-router";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { ChannelActivityPopover } from "@/features/sidebar/ui/ChannelActivityPopover";
import { useAppShell } from "@/app/AppShellContext";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useThreadStars } from "@/features/sidebar/lib/useThreadStars";
import { shouldSuppressChannelActiveForStarredThread } from "@/features/sidebar/lib/starredThreadSidebar";
import { UserNameIndicators } from "@/features/user-status/ui/UserNameIndicators";

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "relative size-2.5 shrink-0 text-current opacity-0 transition-[color,opacity] group-hover/sidebar-section:opacity-100 group-hover/section-label:opacity-100 group-focus-within/sidebar-section:opacity-100 group-focus-visible/section-label:opacity-100 group-data-[section-actions-open=true]/sidebar-section:opacity-100";
const SECTION_LABEL_CHEVRON_ICON_CLASS =
  "absolute left-1/2 top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2";
const SIDEBAR_ROW_ACTION_VISIBILITY_CLASS =
  "group-focus-within/menu-item:opacity-100 group-hover/menu-item:opacity-100 md:opacity-0";
const SIDEBAR_ROW_ACTION_REPLACED_BADGE_CLASS =
  "max-md:opacity-0 md:group-focus-within/menu-item:opacity-0 md:group-hover/menu-item:opacity-0";
const SIDEBAR_ROW_ICON_ACTION_CLASS =
  "flex size-6 items-center justify-center p-1 text-sidebar-foreground/45 transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring peer-data-[active=true]/menu-button:text-sidebar-active-foreground/75 peer-data-[active=true]/menu-button:hover:text-sidebar-active-foreground [&>svg]:size-4 [&>svg]:shrink-0";
const DM_AVATAR_SIZE = 24;
const DM_AVATAR_STATUS_GEOMETRY = scaleProfileAvatarStatusGeometry(
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  DM_AVATAR_SIZE,
);

function formatUnreadCount(count: number): string {
  return count > 99 ? "99+" : String(count);
}

function UnreadCountBadge({
  channelName,
  className,
  count,
}: {
  channelName: string;
  className?: string;
  count: number;
}) {
  return (
    <span
      className={cn(
        "flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground tabular-nums",
        className,
      )}
      data-testid={`channel-unread-${channelName}`}
    >
      {formatUnreadCount(count)}
      <span className="sr-only">
        {" "}
        unread notification{count === 1 ? "" : "s"}
      </span>
    </span>
  );
}

export type SidebarDmParticipant = {
  avatarUrl: string | null;
  label: string;
  isAgent?: boolean;
  pubkey: string;
};

function DmChannelIcon({
  channelName,
  isPair,
  participants,
  presenceStatus,
}: {
  channelName: string;
  isPair: boolean;
  participants?: SidebarDmParticipant[];
  presenceStatus?: PresenceStatus;
}) {
  const primaryParticipant = participants?.[0];

  if (!primaryParticipant) {
    return <CircleDot className="h-4 w-4" />;
  }

  if (!isPair && participants && participants.length > 1) {
    return (
      <span
        aria-hidden="true"
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-sidebar-accent/80 text-2xs font-semibold leading-none text-sidebar-foreground shadow-none"
        data-testid={`channel-dm-count-${channelName}`}
      >
        <span className="translate-x-px leading-none">
          {participants.length}
        </span>
      </span>
    );
  }

  if (isPair || !participants || participants.length <= 1) {
    return (
      <span className="relative flex h-6 w-6 shrink-0 items-center justify-center">
        <ProfileAvatarWithStatus
          avatarClassName="bg-sidebar-accent/80 text-2xs text-sidebar-foreground shadow-none"
          avatarUrl={primaryParticipant.avatarUrl}
          className="h-6 w-6"
          geometry={DM_AVATAR_STATUS_GEOMETRY}
          iconClassName="h-3.5 w-3.5"
          label={primaryParticipant.label}
          pubkey={primaryParticipant.pubkey}
          shape={primaryParticipant.isAgent ? "squircle" : "circle"}
          size={DM_AVATAR_SIZE}
          status={presenceStatus}
          statusTestId={`channel-presence-${channelName}`}
          testId={`channel-avatar-${channelName}`}
        />
      </span>
    );
  }

  return <CircleDot className="h-4 w-4" />;
}

function SidebarChannelIcon({
  channel,
  className,
  dmParticipants,
  presenceStatus,
}: {
  channel: Channel;
  className?: string;
  dmParticipants?: SidebarDmParticipant[];
  presenceStatus?: PresenceStatus;
}) {
  if (channel.channelType === "dm") {
    return (
      <DmChannelIcon
        channelName={channel.name}
        isPair={channel.participantPubkeys.length === 2}
        participants={dmParticipants}
        presenceStatus={
          dmParticipants?.length === 1 ||
          channel.participantPubkeys.length === 2
            ? presenceStatus
            : undefined
        }
      />
    );
  }

  return <ChannelGlyph channel={channel} className={className} />;
}

export function ChannelMenuButton({
  channel,
  label,
  isActive,
  hasUnread,
  unreadCount = 0,
  activeWorking,
  isMuted,
  dmParticipants,
  presenceStatus,
  onSelectChannel,
}: {
  channel: Channel;
  label?: string;
  isActive: boolean;
  hasUnread: boolean;
  /** Channel + thread unreads for this channel (observed unread count). */
  unreadCount?: number;
  activeWorking?: ActiveChannelTurnSummary;
  isMuted?: boolean;
  dmParticipants?: SidebarDmParticipant[];
  presenceStatus?: PresenceStatus;
  onSelectChannel: (channelId: string) => void;
}) {
  const resolvedLabel = label ?? channel.name;
  useDraftsSnapshot();
  const location = useLocation();
  const selectedThreadId = React.useMemo(() => {
    const search = location.search as {
      thread?: unknown;
      threadRootId?: unknown;
    };
    const thread = search.threadRootId ?? search.thread;
    return typeof thread === "string" && thread.trim() ? thread.trim() : null;
  }, [location.search]);
  const identityPubkey = useIdentityQuery().data?.pubkey;
  const { isThreadStarred } = useThreadStars(identityPubkey);
  const suppressActiveForStarredThread =
    shouldSuppressChannelActiveForStarredThread({
      isChannelActive: isActive,
      selectedThreadRootId: selectedThreadId,
      isThreadStarred,
    });
  const rowIsActive = isActive && !suppressActiveForStarredThread;
  const showDraftIndicator = channelShowsDraftIndicator(channel.id, {
    selectedChannelId: isActive ? channel.id : null,
    selectedThreadId: isActive ? selectedThreadId : null,
  });
  const ephemeralDisplay = getEphemeralChannelDisplay(channel);
  const {
    hasSidebarUnreadProjections,
    unreadThreadChannelIds,
    unreadThreadFeedItems,
  } = useAppShell();
  const hasThreadUnread =
    channel.channelType !== "dm" &&
    (hasSidebarUnreadProjections
      ? unreadThreadChannelIds.has(channel.id)
      : hasUnread);
  // Slack-style: bold when the channel itself or any of its threads is unread.
  const emphasizeUnread = hasUnread || hasThreadUnread;
  const threadFeedUnreadCount = React.useMemo(() => {
    if (channel.channelType === "dm") return 0;
    let count = 0;
    for (const item of unreadThreadFeedItems) {
      if (item.channelId === channel.id) count += 1;
    }
    return count;
  }, [channel.channelType, channel.id, unreadThreadFeedItems]);
  const sidebarUnreadBadgeCount = Math.max(
    unreadCount,
    threadFeedUnreadCount,
    hasThreadUnread ? 1 : 0,
  );
  const showsEphemeralBadge =
    Boolean(ephemeralDisplay) &&
    !activeWorking &&
    !isMuted &&
    sidebarUnreadBadgeCount === 0;
  const inactiveContentOpacity = cn(
    !rowIsActive && !emphasizeUnread && !isMuted && "opacity-80",
    !rowIsActive &&
      isMuted &&
      !emphasizeUnread &&
      "sidebar-muted-content opacity-50 dark:opacity-45",
  );

  const button = (
    <SidebarMenuButton
      className={cn(
        "data-[active=true]:font-normal",
        rowIsActive
          ? "group-hover/menu-item:bg-sidebar-active group-hover/menu-item:text-sidebar-active-foreground"
          : "group-hover/menu-item:bg-sidebar-accent group-hover/menu-item:text-sidebar-foreground",
        emphasizeUnread &&
          "font-bold text-sidebar-foreground hover:text-sidebar-foreground data-[active=true]:font-bold",
      )}
      data-channel-id={channel.id}
      data-testid={`channel-${channel.name}`}
      isActive={rowIsActive}
      onClick={() => onSelectChannel(channel.id)}
      tooltip={resolvedLabel}
      type="button"
    >
      <SidebarChannelIcon
        channel={channel}
        className={
          channel.channelType === "dm" ? undefined : inactiveContentOpacity
        }
        dmParticipants={dmParticipants}
        presenceStatus={presenceStatus}
      />
      <span
        className={cn(
          "flex min-w-0 flex-1 items-center gap-1",
          inactiveContentOpacity,
        )}
        data-sidebar-row-label
      >
        <span className="min-w-0 truncate">{resolvedLabel}</span>
        {channel.channelType === "dm" &&
        (channel.participantPubkeys.length === 2 ||
          dmParticipants?.length === 1) ? (
          <UserNameIndicators
            className="ml-1"
            pubkey={dmParticipants?.[0]?.pubkey}
            size="dm"
          />
        ) : null}
        {showDraftIndicator ? (
          <Pencil
            aria-label="Draft"
            className="h-3.5 w-3.5 shrink-0 text-sidebar-foreground/55"
            data-testid={`channel-draft-${channel.id}`}
          />
        ) : null}
      </span>
      {showsEphemeralBadge && ephemeralDisplay ? (
        <EphemeralChannelBadge
          display={ephemeralDisplay}
          testId={`channel-ephemeral-${channel.name}`}
          variant="sidebar"
        />
      ) : null}
      {channel.channelType === "dm" && dmParticipants?.length === 1 ? (
        <AgentManagementMarker
          pubkey={dmParticipants[0].pubkey}
          testId={`channel-agent-provenance-${channel.id}`}
        />
      ) : null}
      {activeWorking ? (
        <ChannelWorkingBadge
          channelName={channel.name}
          isActive={rowIsActive}
          summary={activeWorking}
        />
      ) : null}
      {isMuted ? (
        <BellOff
          className={cn(
            "ml-auto h-4 w-4 shrink-0",
            rowIsActive
              ? "text-sidebar-active-foreground/60"
              : "text-sidebar-foreground/40",
          )}
        />
      ) : null}
      {channel.channelType !== "dm" && sidebarUnreadBadgeCount > 0 ? (
        <span
          className={cn(
            "ml-auto shrink-0 rounded-full px-1.5 text-2xs tabular-nums",
            rowIsActive
              ? "bg-sidebar-active-foreground/20 text-sidebar-active-foreground"
              : "bg-primary/15 text-primary",
          )}
          data-testid={`channel-unread-${channel.name}`}
        >
          {formatUnreadCount(sidebarUnreadBadgeCount)}
          <span className="sr-only">
            {" "}
            unread{sidebarUnreadBadgeCount === 1 ? "" : "s"}
          </span>
        </span>
      ) : null}
    </SidebarMenuButton>
  );

  // Hover card is for thread activity / working agents — not plain channel counts.
  if (!activeWorking && !hasThreadUnread) {
    return button;
  }

  return (
    <ChannelActivityPopover activeWorking={activeWorking} channel={channel}>
      {button}
    </ChannelActivityPopover>
  );
}

export function SidebarSection({
  action,
  activeWorkingByChannelId,
  dmParticipantsByChannelId,
  emptyState,
  items,
  channelLabels,
  isCollapsed,
  isActiveChannel,
  presenceByChannelId,
  selectedChannelId,
  title,
  testId,
  unreadChannelCounts,
  unreadChannelIds,
  onHideDm,
  onMarkChannelRead,
  onMarkChannelUnread,
  onSelectChannel,
  onToggleCollapsed,
  mutedChannelIds,
  onMuteChannel,
  onUnmuteChannel,
  sectionActionsOpen,
}: {
  action?: React.ReactNode;
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  dmParticipantsByChannelId?: Record<string, SidebarDmParticipant[]>;
  emptyState?: React.ReactNode;
  items: Channel[];
  channelLabels?: Record<string, string>;
  isCollapsed?: boolean;
  isActiveChannel: boolean;
  presenceByChannelId?: Record<string, PresenceStatus>;
  selectedChannelId: string | null;
  title: string;
  testId: string;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  onHideDm?: (channelId: string) => void;
  onMarkChannelRead?: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread?: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onToggleCollapsed?: () => void;
  mutedChannelIds?: ReadonlySet<string>;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  sectionActionsOpen?: boolean;
}) {
  if (items.length === 0 && !action && !emptyState) {
    return null;
  }

  const contentId = `sidebar-${testId}`;
  const canToggle = Boolean(onToggleCollapsed);

  return (
    <SidebarGroup
      className="group/sidebar-section select-none"
      data-section-actions-open={sectionActionsOpen || undefined}
    >
      <div className="relative">
        <SidebarGroupLabel asChild={canToggle}>
          {canToggle ? (
            <button
              aria-controls={contentId}
              aria-expanded={!isCollapsed}
              className={SECTION_LABEL_BUTTON_CLASS}
              data-testid={`${testId}-section-label`}
              onClick={onToggleCollapsed}
              type="button"
            >
              <span data-sidebar-section-title>{title}</span>
              <span aria-hidden="true" className={SECTION_LABEL_CHEVRON_CLASS}>
                <ChevronDown
                  className={cn(
                    SECTION_LABEL_CHEVRON_ICON_CLASS,
                    isCollapsed ? "-rotate-90" : "rotate-0",
                  )}
                />
              </span>
            </button>
          ) : (
            title
          )}
        </SidebarGroupLabel>
        {action}
      </div>
      {!isCollapsed ? (
        <SidebarGroupContent id={contentId}>
          {items.length > 0 ? (
            <SidebarMenu data-testid={testId}>
              {items.map((channel) => {
                const menuItem = (
                  <SidebarMenuItem
                    key={onMarkChannelUnread ? undefined : channel.id}
                    className="group/menu-item"
                  >
                    <ChannelMenuButton
                      channel={channel}
                      activeWorking={activeWorkingByChannelId?.get(channel.id)}
                      dmParticipants={dmParticipantsByChannelId?.[channel.id]}
                      hasUnread={unreadChannelIds.has(channel.id)}
                      unreadCount={unreadChannelCounts.get(channel.id) ?? 0}
                      isMuted={mutedChannelIds?.has(channel.id)}
                      isActive={
                        isActiveChannel && selectedChannelId === channel.id
                      }
                      label={channelLabels?.[channel.id] ?? channel.name}
                      presenceStatus={presenceByChannelId?.[channel.id]}
                      onSelectChannel={onSelectChannel}
                    />
                    {channel.channelType === "dm" &&
                    unreadChannelIds.has(channel.id) &&
                    !(isActiveChannel && selectedChannelId === channel.id) ? (
                      <UnreadCountBadge
                        channelName={channel.name}
                        className={cn(
                          "pointer-events-none absolute right-1 top-1/2 -translate-y-1/2 transition-opacity",
                          onHideDm && SIDEBAR_ROW_ACTION_REPLACED_BADGE_CLASS,
                        )}
                        count={Math.max(
                          unreadChannelCounts.get(channel.id) ?? 0,
                          1,
                        )}
                      />
                    ) : null}
                    {channel.channelType === "dm" && onHideDm ? (
                      <button
                        aria-label="Close direct message"
                        className={cn(
                          "absolute right-1 top-1/2 z-10 -translate-y-1/2 after:absolute after:-inset-2 after:md:hidden group-data-[collapsible=icon]:hidden",
                          SIDEBAR_ROW_ICON_ACTION_CLASS,
                          SIDEBAR_ROW_ACTION_VISIBILITY_CLASS,
                        )}
                        data-sidebar="menu-action"
                        data-testid={`hide-dm-${channel.name}`}
                        onClick={(event) => {
                          event.stopPropagation();
                          onHideDm(channel.id);
                        }}
                        type="button"
                      >
                        <X />
                      </button>
                    ) : null}
                  </SidebarMenuItem>
                );

                // The shared menu always renders copy actions, so every row
                // gets a context menu regardless of read/mute availability.
                return (
                  <ContextMenu key={channel.id}>
                    <ContextMenuTrigger asChild>{menuItem}</ContextMenuTrigger>
                    <ContextMenuContent>
                      <ChannelContextMenuItems
                        channel={channel}
                        hasUnread={unreadChannelIds.has(channel.id)}
                        isMuted={mutedChannelIds?.has(channel.id)}
                        onMarkChannelRead={onMarkChannelRead}
                        onMarkChannelUnread={onMarkChannelUnread}
                        onMuteChannel={onMuteChannel}
                        onUnmuteChannel={onUnmuteChannel}
                      />
                    </ContextMenuContent>
                  </ContextMenu>
                );
              })}
            </SidebarMenu>
          ) : emptyState ? (
            <div
              className="px-2 py-1 text-sm text-sidebar-foreground/60"
              data-testid={`${testId}-empty`}
            >
              {emptyState}
            </div>
          ) : null}
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}
