import * as React from "react";

import {
  Archive,
  Bell,
  BellOff,
  Check,
  CheckCircle2,
  CircleDot,
  Copy,
  LogOut,
  Pencil,
  Plus,
  Star,
  StarOff,
} from "lucide-react";
import { toast } from "sonner";

import { useAppShell } from "@/app/AppShellContext";
import {
  useArchiveChannelMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ChannelSection } from "@/features/sidebar/lib/useChannelSections";
import {
  ContextMenuIconSlot,
  deferMenuAction,
} from "@/features/sidebar/ui/sidebarMenuHelpers";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import type { Channel } from "@/shared/api/types";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/shared/ui/context-menu";

export function ChannelContextMenu({
  children,
  contentProps,
  modal,
}: {
  children: React.ReactNode;
  contentProps: React.ComponentProps<typeof ChannelContextMenuItems>;
  modal?: boolean;
}) {
  return (
    <ContextMenu modal={modal}>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent>
        <ChannelContextMenuItems {...contentProps} />
      </ContextMenuContent>
    </ContextMenu>
  );
}

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
  onLeaveChannel?: (channel: Channel) => void;
}) {
  const { openChannelManagement } = useAppShell();
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const archiveChannelMutation = useArchiveChannelMutation(channel.id);
  const membersQuery = useChannelMembersQuery(
    channel.id,
    channel.channelType !== "dm",
  );
  const ownerPubkeys =
    membersQuery.data
      ?.filter(
        (member) => member.role === "owner" && member.pubkey !== currentPubkey,
      )
      .map((member) => member.pubkey) ?? [];
  const ownerProfilesQuery = useUsersBatchQuery(ownerPubkeys, {
    enabled: ownerPubkeys.length > 0,
  });
  const selfMember = membersQuery.data?.find(
    (member) => member.pubkey === currentPubkey,
  );
  const canManageOwnedAgentChannel = ownerPubkeys.some((pubkey) =>
    ownsAuthorAgent(
      ownerProfilesQuery.data?.profiles[normalizePubkey(pubkey)],
      currentPubkey,
    ),
  );
  const canManageChannel =
    selfMember?.role === "owner" ||
    selfMember?.role === "admin" ||
    canManageOwnedAgentChannel;
  const hasResolvedMembers =
    membersQuery.data !== undefined || membersQuery.isError;
  const hasResolvedOwnerProfiles =
    ownerPubkeys.length === 0 ||
    ownerProfilesQuery.data !== undefined ||
    ownerProfilesQuery.isError;
  const isManagementCandidate =
    channel.channelType !== "dm" && channel.archivedAt === null;
  // Context-menu content mounts per open session. Once these rows are reserved,
  // keep them until close so permission resolution cannot move another action
  // beneath the pointer; eligible rows transition from disabled to enabled.
  const [showManagementActions] = React.useState(
    () =>
      isManagementCandidate &&
      (!hasResolvedMembers || !hasResolvedOwnerProfiles || canManageChannel),
  );
  const disableManagementActions = !canManageChannel;
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
      {showManagementActions ? (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem
            disabled={disableManagementActions}
            onSelect={() =>
              deferMenuAction(() =>
                openChannelManagement(channel.id, { edit: true }),
              )
            }
          >
            <ContextMenuIconSlot>
              <Pencil className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Edit channel</span>
          </ContextMenuItem>
          <ContextMenuItem
            className="text-destructive focus:text-destructive"
            disabled={disableManagementActions}
            onSelect={() =>
              deferMenuAction(() => {
                void archiveChannelMutation.mutateAsync().catch((error) => {
                  toast.error(
                    error instanceof Error
                      ? error.message
                      : "Failed to archive channel.",
                  );
                });
              })
            }
          >
            <ContextMenuIconSlot>
              <Archive className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Archive channel</span>
          </ContextMenuItem>
        </>
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
      {onLeaveChannel ? (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem
            className="text-destructive focus:text-destructive"
            onSelect={() => deferMenuAction(() => onLeaveChannel(channel))}
          >
            <ContextMenuIconSlot>
              <LogOut className="h-4 w-4" />
            </ContextMenuIconSlot>
            <span>Leave channel</span>
          </ContextMenuItem>
        </>
      ) : null}
    </>
  );
}
