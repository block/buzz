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

import { ChannelContextMenuItems } from "@/features/sidebar/ui/CustomChannelSection";
import { getEphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { EphemeralChannelBadge } from "@/features/channels/ui/EphemeralChannelBadge";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { Channel, PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";

import { PresenceDot } from "@/features/presence/ui/PresenceBadge";

const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "h-2.5 w-2.5 shrink-0 opacity-0 text-sidebar-foreground/45 transition-[color,opacity,transform] group-hover/section-label:opacity-100 group-hover/section-label:text-sidebar-foreground group-focus-visible/section-label:opacity-100 group-focus-visible/section-label:text-sidebar-foreground";
const SIDEBAR_ROW_ACTION_VISIBILITY_CLASS =
  "group-focus-within/menu-item:opacity-100 group-hover/menu-item:opacity-100 md:opacity-0";
const SIDEBAR_ROW_ICON_ACTION_CLASS =
  "flex size-6 items-center justify-center p-1 text-sidebar-foreground/45 transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring peer-data-[active=true]/menu-button:text-sidebar-active-foreground/75 peer-data-[active=true]/menu-button:hover:text-sidebar-active-foreground [&>svg]:size-4 [&>svg]:shrink-0";

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
  const secondaryParticipant = participants?.[1];

  if (!primaryParticipant) {
    return <CircleDot className="h-4 w-4" />;
  }

  if (isPair || !secondaryParticipant) {
    return (
      <span className="relative flex h-5 w-5 shrink-0 items-center justify-center">
        <ProfileAvatar
          avatarUrl={primaryParticipant.avatarUrl}
          className="h-5 w-5 rounded-full border border-sidebar-border/80 bg-sidebar-accent/80 text-[9px] text-sidebar-foreground shadow-none"
          iconClassName="h-3 w-3"
          label={primaryParticipant.label}
        />
        {presenceStatus ? (
          <span className="absolute -bottom-0.5 -right-0.5 flex h-2.5 w-2.5 items-center justify-center rounded-full bg-sidebar">
            <PresenceDot
              className="h-1.5 w-1.5"
              data-testid={`channel-presence-${channelName}`}
              status={presenceStatus}
            />
          </span>
        ) : null}
      </span>
    );
  }

  return (
    <span className="relative flex h-5 w-7 shrink-0 items-center">
      <ProfileAvatar
        avatarUrl={primaryParticipant.avatarUrl}
        className="absolute left-0 top-0 h-[18px] w-[18px] rounded-full border-2 border-sidebar bg-sidebar-accent/80 text-[8px] text-sidebar-foreground shadow-none"
        iconClassName="h-2.5 w-2.5"
        label={primaryParticipant.label}
      />
      <ProfileAvatar
        avatarUrl={secondaryParticipant.avatarUrl}
        className="absolute bottom-0 right-0 h-[18px] w-[18px] rounded-full border-2 border-sidebar bg-sidebar-accent/80 text-[8px] text-sidebar-foreground shadow-none"
        iconClassName="h-2.5 w-2.5"
        label={secondaryParticipant.label}
      />
      {participants && participants.length > 2 ? (
        <span className="absolute -bottom-1 -right-1 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-sidebar-primary px-1 text-[8px] font-semibold text-sidebar-primary-foreground">
          {participants.length}
        </span>
      ) : null}
    </span>
  );
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
  isMuted,
  dmParticipants,
  presenceStatus,
  onSelectChannel,
}: {
  channel: Channel;
  label?: string;
  isActive: boolean;
  hasUnread: boolean;
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
      <span className="min-w-0 flex-1 truncate">{resolvedLabel}</span>
      {ephemeralDisplay ? (
        <EphemeralChannelBadge
          display={ephemeralDisplay}
          testId={`channel-ephemeral-${channel.name}`}
          variant="sidebar"
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
        <span
          aria-hidden="true"
          className="ml-auto h-2.5 w-2.5 shrink-0 rounded-full bg-primary"
          data-testid={`channel-unread-${channel.name}`}
        />
      ) : null}
    </SidebarMenuButton>
  );
}

export function SidebarSection({
  action,
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
  unreadChannelIds,
  onHideDm,
  onMarkChannelRead,
  onMarkChannelUnread,
  onSelectChannel,
  onToggleCollapsed,
  mutedChannelIds,
  onMuteChannel,
  onUnmuteChannel,
}: {
  action?: React.ReactNode;
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
}) {
  if (items.length === 0 && !action && !emptyState) {
    return null;
  }

  const contentId = `sidebar-${testId}`;
  const canToggle = Boolean(onToggleCollapsed);

  return (
    <SidebarGroup>
      <div className="group/sidebar-section relative">
        <SidebarGroupLabel asChild={canToggle}>
          {canToggle ? (
            <button
              aria-controls={contentId}
              aria-expanded={!isCollapsed}
              className={SECTION_LABEL_BUTTON_CLASS}
              onClick={onToggleCollapsed}
              type="button"
            >
              <span>{title}</span>
              <ChevronDown
                aria-hidden="true"
                className={cn(
                  SECTION_LABEL_CHEVRON_CLASS,
                  isCollapsed ? "-rotate-90" : "rotate-0",
                )}
              />
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
                      dmParticipants={dmParticipantsByChannelId?.[channel.id]}
                      hasUnread={unreadChannelIds.has(channel.id)}
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
                      <span
                        aria-hidden="true"
                        className="absolute right-[9px] top-1/2 h-2.5 w-2.5 -translate-y-1/2 rounded-full bg-primary"
                        data-testid={`channel-unread-${channel.name}`}
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

                const hasContextAction =
                  (unreadChannelIds.has(channel.id) && onMarkChannelRead) ||
                  (!unreadChannelIds.has(channel.id) && onMarkChannelUnread) ||
                  (onMuteChannel && onUnmuteChannel);

                return hasContextAction ? (
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
                ) : (
                  menuItem
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
