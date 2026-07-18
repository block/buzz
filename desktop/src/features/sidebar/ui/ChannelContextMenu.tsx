import {
  Archive,
  Bell,
  BellOff,
  Check,
  CheckCircle2,
  CircleDot,
  Copy,
  LogOut,
  LoaderCircle,
  Plus,
  Star,
  StarOff,
  Trash2,
  TriangleAlert,
} from "lucide-react";

import {
  useArchiveChannelMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import { useChannelModerationCapabilities } from "@/features/channels/ui/ChannelManagementModerationActions";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import {
  ContextMenuIconSlot,
  deferMenuAction,
} from "@/features/sidebar/ui/sidebarMenuHelpers";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { Channel } from "@/shared/api/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/shared/ui/context-menu";

function MoveToSectionSubmenu({
  channelId,
  sections,
  assignments,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
}: {
  channelId: string;
  sections: ChannelSection[];
  assignments: Record<string, string>;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onUnassignChannel: (channelId: string) => void;
  onCreateSectionForChannel: (channelId: string) => void;
}) {
  const currentSectionId = assignments[channelId];

  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <ContextMenuIconSlot />
        <span>Move to section</span>
      </ContextMenuSubTrigger>
      <ContextMenuSubContent>
        {sections.map((section) => (
          <ContextMenuItem
            key={section.id}
            onSelect={() =>
              deferMenuAction(() => onAssignChannel(channelId, section.id))
            }
          >
            <ContextMenuIconSlot>
              {currentSectionId === section.id ? (
                <Check className="h-4 w-4" />
              ) : section.icon ? (
                <StatusEmoji className="h-4 w-4" value={section.icon} />
              ) : null}
            </ContextMenuIconSlot>
            <span>{section.name}</span>
          </ContextMenuItem>
        ))}
        {sections.length > 0 ? <ContextMenuSeparator /> : null}
        <ContextMenuItem
          onSelect={() =>
            deferMenuAction(() => onCreateSectionForChannel(channelId))
          }
        >
          <ContextMenuIconSlot>
            <Plus className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>New section...</span>
        </ContextMenuItem>
        {currentSectionId ? (
          <ContextMenuItem
            onSelect={() => deferMenuAction(() => onUnassignChannel(channelId))}
          >
            <ContextMenuIconSlot />
            <span>Remove from section</span>
          </ContextMenuItem>
        ) : null}
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

/**
 * The channel/DM context menu's Copy actions, grouped under a single
 * "Copy" submenu (channel name / channel ID).
 */
function CopyChannelSubmenu({ channel }: { channel: Channel }) {
  return (
    <ContextMenuSub>
      <ContextMenuSubTrigger>
        <ContextMenuIconSlot>
          <Copy className="h-4 w-4" />
        </ContextMenuIconSlot>
        <span>Copy</span>
      </ContextMenuSubTrigger>
      <ContextMenuSubContent>
        <ContextMenuItem
          onSelect={() =>
            copyTextToClipboard(
              channel.name,
              "Channel name copied to clipboard",
            )
          }
        >
          <span>Copy channel name</span>
        </ContextMenuItem>
        <ContextMenuItem
          onSelect={() =>
            copyTextToClipboard(channel.id, "Channel ID copied to clipboard")
          }
        >
          <span>Copy channel ID</span>
        </ContextMenuItem>
      </ContextMenuSubContent>
    </ContextMenuSub>
  );
}

export function ChannelContextMenuItems({
  channel,
  hasUnread,
  isMuted,
  isStarred,
  sections,
  assignments,
  onMarkChannelRead,
  onMarkChannelUnread,
  onMuteChannel,
  onUnmuteChannel,
  onStarChannel,
  onUnstarChannel,
  onAssignChannel,
  onUnassignChannel,
  onCreateSectionForChannel,
  onDeleteChannel,
  onLeaveChannel,
}: {
  channel: Channel;
  hasUnread: boolean;
  isMuted?: boolean;
  isStarred?: boolean;
  sections?: ChannelSection[];
  assignments?: Record<string, string>;
  onMarkChannelRead?: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelUnread?: (channelId: string) => void;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
  onAssignChannel?: (channelId: string, sectionId: string) => void;
  onUnassignChannel?: (channelId: string) => void;
  onCreateSectionForChannel?: (channelId: string) => void;
  onDeleteChannel?: (channel: Channel) => void;
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const canLoadOwnerActions =
    channel.channelType !== "dm" && Boolean(onDeleteChannel);
  const membersQuery = useChannelMembersQuery(channel.id, canLoadOwnerActions);
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const archiveChannel = useArchiveChannelMutation(channel.id);
  const {
    canDeleteChannel,
    canManageChannel,
    error: capabilityError,
    isLoading: isCapabilityLoading,
  } = useChannelModerationCapabilities(
    membersQuery.data,
    currentPubkey,
    canLoadOwnerActions,
  );
  const ownerActionsError = membersQuery.error ?? capabilityError;
  const ownerActionsLoading =
    canLoadOwnerActions && (membersQuery.isLoading || isCapabilityLoading);
  const showChannelActions = Boolean(
    onLeaveChannel ||
      ownerActionsLoading ||
      ownerActionsError ||
      canManageChannel ||
      canDeleteChannel,
  );
  const showStar = Boolean(onStarChannel && onUnstarChannel);
  const showReadToggle = hasUnread
    ? Boolean(onMarkChannelRead)
    : Boolean(onMarkChannelUnread);
  const showMuteToggle = Boolean(onMuteChannel && onUnmuteChannel);
  const showMove = Boolean(
    sections &&
      assignments &&
      onAssignChannel &&
      onUnassignChannel &&
      onCreateSectionForChannel,
  );

  return (
    <>
      <CopyChannelSubmenu channel={channel} />
      {showMove ? (
        <MoveToSectionSubmenu
          channelId={channel.id}
          sections={sections ?? []}
          assignments={assignments ?? {}}
          onAssignChannel={onAssignChannel ?? (() => {})}
          onUnassignChannel={onUnassignChannel ?? (() => {})}
          onCreateSectionForChannel={onCreateSectionForChannel ?? (() => {})}
        />
      ) : null}
      {showReadToggle ? <ContextMenuSeparator /> : null}
      {hasUnread && onMarkChannelRead ? (
        <ContextMenuItem
          onSelect={() =>
            deferMenuAction(() =>
              onMarkChannelRead(channel.id, channel.lastMessageAt),
            )
          }
        >
          <ContextMenuIconSlot>
            <CheckCircle2 className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Mark as read</span>
        </ContextMenuItem>
      ) : !hasUnread && onMarkChannelUnread ? (
        <ContextMenuItem
          onSelect={() =>
            deferMenuAction(() => onMarkChannelUnread(channel.id))
          }
        >
          <ContextMenuIconSlot>
            <CircleDot className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Mark unread</span>
        </ContextMenuItem>
      ) : null}
      {showMuteToggle || showStar ? <ContextMenuSeparator /> : null}
      {showMuteToggle ? (
        isMuted ? (
          <ContextMenuItem
            onSelect={() =>
              deferMenuAction(() => onUnmuteChannel?.(channel.id))
            }
          >
            <ContextMenuIconSlot>
              <Bell className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Unmute channel</span>
          </ContextMenuItem>
        ) : (
          <ContextMenuItem
            onSelect={() => deferMenuAction(() => onMuteChannel?.(channel.id))}
          >
            <ContextMenuIconSlot>
              <BellOff className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Mute channel</span>
          </ContextMenuItem>
        )
      ) : null}
      {showStar ? (
        isStarred ? (
          <ContextMenuItem
            onSelect={() =>
              deferMenuAction(() => onUnstarChannel?.(channel.id))
            }
          >
            <ContextMenuIconSlot>
              <StarOff className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Unstar channel</span>
          </ContextMenuItem>
        ) : (
          <ContextMenuItem
            onSelect={() => deferMenuAction(() => onStarChannel?.(channel.id))}
          >
            <ContextMenuIconSlot>
              <Star className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Star channel</span>
          </ContextMenuItem>
        )
      ) : null}
      {showChannelActions ? <ContextMenuSeparator /> : null}
      {onLeaveChannel ? (
        <ContextMenuItem
          className="text-destructive focus:text-destructive"
          onSelect={() => deferMenuAction(() => onLeaveChannel(channel))}
        >
          <ContextMenuIconSlot>
            <LogOut className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Leave channel</span>
        </ContextMenuItem>
      ) : null}
      {ownerActionsLoading ? (
        <ContextMenuItem disabled>
          <ContextMenuIconSlot>
            <LoaderCircle className="h-4 w-4 animate-spin" />
          </ContextMenuIconSlot>
          <span>Loading channel actions...</span>
        </ContextMenuItem>
      ) : ownerActionsError ? (
        <ContextMenuItem disabled>
          <ContextMenuIconSlot>
            <TriangleAlert className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Channel actions unavailable</span>
        </ContextMenuItem>
      ) : null}
      {canManageChannel ? (
        <ContextMenuItem
          data-testid={`archive-channel-${channel.name}`}
          disabled={archiveChannel.isPending}
          onSelect={() => deferMenuAction(() => archiveChannel.mutate())}
        >
          <ContextMenuIconSlot>
            <Archive className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Archive channel</span>
        </ContextMenuItem>
      ) : null}
      {canDeleteChannel ? (
        <ContextMenuItem
          className="text-destructive focus:text-destructive"
          data-testid={`delete-channel-${channel.name}`}
          onSelect={() => deferMenuAction(() => onDeleteChannel?.(channel))}
        >
          <ContextMenuIconSlot>
            <Trash2 className="h-4 w-4" />
          </ContextMenuIconSlot>
          <span>Delete channel</span>
        </ContextMenuItem>
      ) : null}
    </>
  );
}
