import { ChevronRight } from "lucide-react";
import type { ReactNode } from "react";

import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import type { ProjectMoveDestination } from "@/features/sidebar/lib/useProjectMoveDestinations";
import { ChannelContextMenuItems } from "@/features/sidebar/ui/ChannelContextMenu";
import {
  DraggableChannelRow,
  DroppableSectionBody,
} from "@/features/sidebar/ui/SidebarDnd";
import { ChannelMenuButton } from "@/features/sidebar/ui/SidebarSection";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { SidebarMenuItem } from "@/shared/ui/sidebar";

type ProjectChannelGroupProps = {
  activeWorkingByChannelId?: ReadonlyMap<string, ActiveChannelTurnSummary>;
  assignments: Record<string, string>;
  channels: Channel[];
  destination: ProjectMoveDestination;
  isActiveChannel: boolean;
  isExpanded: boolean;
  mutedChannelIds?: ReadonlySet<string>;
  projectDestinations: ProjectMoveDestination[];
  sectionId: string;
  sections: ChannelSection[];
  selectedChannelId: string | null;
  starredChannelIds?: ReadonlySet<string>;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onAssignChannelToProject: (channelId: string, projectAddress: string) => void;
  onCreateSectionForChannel: (channelId: string) => void;
  onDeleteChannel?: (channel: Channel) => void;
  onLeaveChannel?: (channel: Channel) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onMuteChannel?: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onStarChannel?: (channelId: string) => void;
  onToggleExpanded: () => void;
  onUnassignChannel: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
};

export type ProjectChannelGroupModel = {
  channels: Channel[];
  destination: ProjectMoveDestination;
  homeChannel: Channel;
  sectionId: string;
};

export function listProjectChannelGroups({
  channels,
  destinations,
  sectionChannels,
}: {
  channels: Channel[];
  destinations: ProjectMoveDestination[];
  sectionChannels: Record<string, Channel[]>;
}): ProjectChannelGroupModel[] {
  return destinations.flatMap((destination) => {
    if (!destination.sectionId) return [];
    const homeChannel = channels.find(
      (channel) => channel.id === destination.projectChannelId,
    );
    if (!homeChannel) return [];
    return [
      {
        channels: [
          homeChannel,
          ...(sectionChannels[destination.sectionId] ?? []).filter(
            (channel) => channel.id !== homeChannel.id,
          ),
        ],
        destination,
        homeChannel,
        sectionId: destination.sectionId,
      },
    ];
  });
}

export function ProjectChannelGroup(props: ProjectChannelGroupProps) {
  const homeChannel = props.channels.find(
    (channel) => channel.id === props.destination.projectChannelId,
  );
  if (!homeChannel) return null;

  const childChannels = props.channels.filter(
    (channel) => channel.id !== homeChannel.id,
  );
  const hasChildren = childChannels.length > 0;
  const isHomeActive =
    props.isActiveChannel && props.selectedChannelId === homeChannel.id;
  return (
    <>
      <ProjectChannelRow
        allowMove={false}
        channel={homeChannel}
        dropSectionId={props.sectionId}
        hideIcon={hasChildren}
        testId={`project-channel-group-${homeChannel.name}`}
        {...props}
      >
        {hasChildren ? (
          <button
            aria-expanded={props.isExpanded}
            aria-label={
              props.isExpanded
                ? `Hide channels in ${homeChannel.name}`
                : `Show channels in ${homeChannel.name}`
            }
            className={cn(
              "absolute left-1 top-1/2 z-10 flex size-6 -translate-y-1/2 items-center justify-center rounded-md outline-hidden ring-sidebar-ring transition-colors hover:bg-sidebar-accent focus-visible:ring-2",
              isHomeActive
                ? "text-sidebar-active-foreground/75 hover:text-sidebar-active-foreground"
                : "text-sidebar-foreground/70 hover:text-sidebar-foreground",
            )}
            data-testid={`project-channel-expand-${homeChannel.name}`}
            onClick={(event) => {
              event.stopPropagation();
              props.onToggleExpanded();
            }}
            type="button"
          >
            <ChevronRight
              className={cn(
                "size-4 transition-transform duration-150",
                props.isExpanded && "rotate-90",
              )}
            />
          </button>
        ) : null}
      </ProjectChannelRow>
      {hasChildren && props.isExpanded
        ? childChannels.map((channel) => (
            <ProjectChannelRow
              channel={channel}
              draggable
              key={channel.id}
              nested
              {...props}
            />
          ))
        : null}
    </>
  );
}

function ProjectChannelRow({
  allowMove = true,
  channel,
  children,
  draggable = false,
  dropSectionId,
  hideIcon = false,
  nested = false,
  testId,
  ...props
}: ProjectChannelGroupProps & {
  allowMove?: boolean;
  channel: Channel;
  children?: ReactNode;
  draggable?: boolean;
  dropSectionId?: string;
  hideIcon?: boolean;
  nested?: boolean;
  testId?: string;
}) {
  const button = (
    <ChannelMenuButton
      activeWorking={props.activeWorkingByChannelId?.get(channel.id)}
      channel={channel}
      className={cn(hideIcon && "pl-8", nested && "pl-7")}
      hasUnread={props.unreadChannelIds.has(channel.id)}
      hideIcon={hideIcon}
      isActive={props.isActiveChannel && props.selectedChannelId === channel.id}
      isMuted={props.mutedChannelIds?.has(channel.id)}
      onSelectChannel={props.onSelectChannel}
      unreadCount={props.unreadChannelCounts.get(channel.id) ?? 0}
    />
  );
  const content = (
    <>
      {draggable ? (
        <DraggableChannelRow channelId={channel.id}>
          {button}
        </DraggableChannelRow>
      ) : (
        button
      )}
      {children}
    </>
  );

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <SidebarMenuItem data-testid={testId}>
          {dropSectionId ? (
            <DroppableSectionBody sectionId={dropSectionId}>
              {content}
            </DroppableSectionBody>
          ) : (
            content
          )}
        </SidebarMenuItem>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ChannelContextMenuItems
          assignments={allowMove ? props.assignments : undefined}
          channel={channel}
          hasUnread={props.unreadChannelIds.has(channel.id)}
          isMuted={props.mutedChannelIds?.has(channel.id)}
          isStarred={props.starredChannelIds?.has(channel.id)}
          onAssignChannel={allowMove ? props.onAssignChannel : undefined}
          onAssignChannelToProject={
            allowMove ? props.onAssignChannelToProject : undefined
          }
          onCreateSectionForChannel={
            allowMove ? props.onCreateSectionForChannel : undefined
          }
          onDeleteChannel={props.onDeleteChannel}
          onLeaveChannel={props.onLeaveChannel}
          onMarkChannelRead={props.onMarkChannelRead}
          onMarkChannelUnread={props.onMarkChannelUnread}
          onMuteChannel={props.onMuteChannel}
          onStarChannel={props.onStarChannel}
          onUnassignChannel={allowMove ? props.onUnassignChannel : undefined}
          onUnmuteChannel={props.onUnmuteChannel}
          onUnstarChannel={props.onUnstarChannel}
          projectDestinations={
            allowMove ? props.projectDestinations : undefined
          }
          sections={allowMove ? props.sections : undefined}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}
