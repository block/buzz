import type * as React from "react";
import {
  BellOff,
  ChevronDown,
  CircleDot,
  FileText,
  Hash,
  Lock,
  X,
} from "lucide-react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";

import { ChannelContextMenuItems } from "@/features/sidebar/ui/ChannelContextMenu";
import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { getEphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { EphemeralChannelBadge } from "@/features/channels/ui/EphemeralChannelBadge";
import {
  DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  ProfileAvatarWithStatus,
  scaleProfileAvatarStatusGeometry,
} from "@/features/profile/ui/ProfileAvatarWithStatus";
import type { Channel, PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";

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
      <span className="sr-only"> new comment{count === 1 ? "" : "s"}</span>
    </span>
  );
}

function UnreadDotBadge({
  channelName,
  className,
}: {
  channelName: string;
  className?: string;
}) {
  return (
    <span
      className={cn("h-2 w-2 shrink-0 rounded-full bg-primary", className)}
      data-testid={`channel-unread-dot-${channelName}`}
    >
      <span className="sr-only">unread</span>
    </span>
  );
}

function formatAgentCount(count: number) {
  return `${count} ${count === 1 ? "agent" : "agents"}`;
}

export function formatWorkingTooltip(
  summary: ActiveChannelTurnSummary,
): string {
  const leadName = summary.agentNames?.[0];

  if (!leadName) {
    return `${formatAgentCount(summary.agentCount)} working`;
  }

  const remainingAgentCount = summary.agentCount - 1;
  if (remainingAgentCount <= 0) {
    return `${leadName} working`;
  }

  return `${leadName} and ${formatAgentCount(remainingAgentCount)} working`;
}

function ChannelWorkingBadge({
  channelName,
  isActive,
  summary,
}: {
  channelName: string;
  isActive: boolean;
  summary: ActiveChannelTurnSummary;
}) {
  const now = useNow(1000);
  const elapsed = formatElapsed(now - summary.anchorAt);
  const label =
    summary.agentCount > 1 ? `${elapsed} (${summary.agentCount})` : elapsed;
  const title = formatWorkingTooltip(summary);

  return (
    <span
      className={cn(
        "hidden max-w-32 shrink-0 truncate rounded-full px-1.5 py-0.5 text-2xs font-medium leading-none tabular-nums motion-safe:animate-pulse group-data-[collapsible=icon]:hidden sm:inline-flex",
        isActive
          ? "bg-sidebar-active-foreground/20 text-sidebar-active-foreground"
          : "bg-primary/10 text-primary",
      )}
      data-testid={`channel-working-${channelName}`}
      title={title}
    >
      {label}
    </span>
  );
}

export type SidebarDmParticipant = {
  avatarUrl: string | null;
  label: string;
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
          size={DM_AVATAR_SIZE}
          status={presenceStatus}
          statusTestId={`channel-presence-${channelName}`}
        />
      </span>
    );
  }

  return <CircleDot className="h-4 w-4" />;
}

function SidebarChannelIcon({
  channel,
  dmParticipants,
  presenceStatus,
}: {
  channel: Channel;
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

  if (channel.visibility === "private") {
    return <Lock className="h-4 w-4" />;
  }

  if (channel.channelType === "forum") {
    return <FileText className="h-4 w-4" />;
  }

  return <Hash className="h-4 w-4" />;
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
  unreadCount?: number;
  activeWorking?: ActiveChannelTurnSummary;
  isMuted?: boolean;
  dmParticipants?: SidebarDmParticipant[];
  presenceStatus?: PresenceStatus;
  onSelectChannel: (channelId: string) => void;
}) {
  const resolvedLabel = label ?? channel.name;
  const ephemeralDisplay = getEphemeralChannelDisplay(channel);

  return (
    <SidebarMenuButton
      className={cn(
        isActive
          ? "group-hover/menu-item:bg-sidebar-active group-hover/menu-item:text-sidebar-active-foreground"
          : "group-hover/menu-item:bg-sidebar-accent group-hover/menu-item:text-sidebar-accent-foreground",
        !isActive &&
          hasUnread &&
          "font-semibold text-sidebar-foreground hover:text-sidebar-foreground",
        !isActive && isMuted && !hasUnread && "opacity-50",
      )}
      data-channel-id={channel.id}
      data-testid={`channel-${channel.name}`}
      isActive={isActive}
      onClick={() => onSelectChannel(channel.id)}
      tooltip={resolvedLabel}
      type="button"
    >
      <SidebarChannelIcon
        channel={channel}
        dmParticipants={dmParticipants}
        presenceStatus={presenceStatus}
      />
      <span className="min-w-0 flex-1 truncate" data-sidebar-row-label>
        {resolvedLabel}
      </span>
      {ephemeralDisplay ? (
        <EphemeralChannelBadge
          display={ephemeralDisplay}
          testId={`channel-ephemeral-${channel.name}`}
          variant="sidebar"
        />
      ) : null}
      {activeWorking ? (
        <ChannelWorkingBadge
          channelName={channel.name}
          isActive={isActive}
          summary={activeWorking}
        />
      ) : null}
      {isMuted ? (
        <BellOff
          className={cn(
            "ml-auto h-4 w-4 shrink-0",
            isActive
              ? "text-sidebar-active-foreground/60"
              : "text-sidebar-foreground/40",
          )}
        />
      ) : null}
      {hasUnread && !isActive && channel.channelType !== "dm" ? (
        unreadCount > 0 ? (
          <UnreadCountBadge
            channelName={channel.name}
            className="ml-auto"
            count={unreadCount}
          />
        ) : (
          <UnreadDotBadge channelName={channel.name} className="ml-auto" />
        )
      ) : null}
    </SidebarMenuButton>
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
