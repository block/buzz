import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { ChannelContextMenuItems } from "@/features/sidebar/ui/ChannelContextMenu";
import { DraggableChannelRow } from "@/features/sidebar/ui/SidebarDnd";
import { ChannelMenuButton } from "@/features/sidebar/ui/SidebarSection";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import type { ProjectMoveDestination } from "@/features/sidebar/lib/useProjectMoveDestinations";
import type { Channel } from "@/shared/api/types";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { SidebarMenuItem } from "@/shared/ui/sidebar";

export type SidebarProjectChildChannelRowContext = {
  activeWorkingByChannelId: ReadonlyMap<string, ActiveChannelTurnSummary>;
  assignments: Record<string, string>;
  isActiveChannel: boolean;
  mutedChannelIds?: ReadonlySet<string>;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onAssignChannelToProject: (channelId: string, projectAddress: string) => void;
  onCreateSectionForChannel: (channelId: string) => void;
  onDeleteChannel: (channel: Channel) => void;
  onLeaveChannel: (channel: Channel) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onMuteChannel?: (channelId: string) => void;
  onSelectChannel: (channelId: string) => void;
  onStarChannel?: (channelId: string) => void;
  onUnassignChannel: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
  projectDestinations: ProjectMoveDestination[];
  sections: ChannelSection[];
  selectedChannelId: string | null;
  starredChannelIds?: ReadonlySet<string>;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
};

export function SidebarProjectChildChannelRow({
  channel,
  context,
  projectDtag,
}: {
  channel: Channel;
  context: SidebarProjectChildChannelRowContext;
  projectDtag: string;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <SidebarMenuItem
          className="group/menu-item"
          data-testid={`sidebar-project-channel-${projectDtag}-${channel.name}`}
        >
          <DraggableChannelRow channelId={channel.id}>
            <ChannelMenuButton
              activeWorking={context.activeWorkingByChannelId.get(channel.id)}
              channel={channel}
              className="h-7 pl-7 text-sidebar-foreground/70 data-[active=true]:!bg-transparent data-[active=true]:font-semibold data-[active=true]:text-sidebar-foreground data-[active=true]:shadow-none"
              hasUnread={context.unreadChannelIds.has(channel.id)}
              isActive={
                context.isActiveChannel &&
                context.selectedChannelId === channel.id
              }
              isMuted={context.mutedChannelIds?.has(channel.id)}
              onSelectChannel={context.onSelectChannel}
              unreadCount={context.unreadChannelCounts.get(channel.id) ?? 0}
            />
          </DraggableChannelRow>
        </SidebarMenuItem>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ChannelContextMenuItems
          assignments={context.assignments}
          channel={channel}
          hasUnread={context.unreadChannelIds.has(channel.id)}
          isMuted={context.mutedChannelIds?.has(channel.id)}
          isStarred={context.starredChannelIds?.has(channel.id)}
          onAssignChannel={context.onAssignChannel}
          onAssignChannelToProject={context.onAssignChannelToProject}
          onCreateSectionForChannel={context.onCreateSectionForChannel}
          onDeleteChannel={context.onDeleteChannel}
          onLeaveChannel={context.onLeaveChannel}
          onMarkChannelRead={context.onMarkChannelRead}
          onMarkChannelUnread={context.onMarkChannelUnread}
          onMuteChannel={context.onMuteChannel}
          onStarChannel={context.onStarChannel}
          onUnassignChannel={context.onUnassignChannel}
          onUnmuteChannel={context.onUnmuteChannel}
          onUnstarChannel={context.onUnstarChannel}
          projectDestinations={context.projectDestinations}
          sections={context.sections}
        />
      </ContextMenuContent>
    </ContextMenu>
  );
}
