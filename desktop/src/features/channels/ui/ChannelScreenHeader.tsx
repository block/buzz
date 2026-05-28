import { LogIn } from "lucide-react";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { getChannelDescription } from "@/features/channels/lib/channelDescription";
import { ChannelHeaderStatusBadge } from "@/features/channels/ui/ChannelHeaderStatusBadge";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { Button } from "@/shared/ui/button";
import type { Channel, PresenceStatus } from "@/shared/api/types";

type ChannelScreenHeaderProps = {
  activeChannel: Channel | null;
  activeChannelEphemeralDisplay: EphemeralChannelDisplay | null;
  activeChannelTitle: string;
  activeDmPresenceStatus: PresenceStatus | null;
  currentPubkey?: string;
  isJoining?: boolean;
  onJoinChannel?: () => Promise<void>;
  onManageChannel: () => void;
  onToggleMembers: () => void;
};

export function ChannelScreenHeader({
  activeChannel,
  activeChannelEphemeralDisplay,
  activeChannelTitle,
  activeDmPresenceStatus,
  currentPubkey,
  isJoining = false,
  onJoinChannel,
  onManageChannel,
  onToggleMembers,
}: ChannelScreenHeaderProps) {
  const showJoinButton =
    activeChannel !== null &&
    !activeChannel.isMember &&
    activeChannel.visibility === "open" &&
    !activeChannel.archivedAt &&
    onJoinChannel;

  return (
    <ChatHeader
      actionsPlacement="top-right"
      belowSystemChrome
      density="compact"
      actions={
        activeChannel ? (
          showJoinButton ? (
            <Button
              disabled={isJoining}
              onClick={() => void onJoinChannel()}
              size="sm"
              variant="default"
            >
              <LogIn className="mr-1.5 h-3.5 w-3.5" />
              {isJoining ? "Joining\u2026" : "Join"}
            </Button>
          ) : (
            <ChannelMembersBar
              channel={activeChannel}
              currentPubkey={currentPubkey}
              onManageChannel={onManageChannel}
              onToggleMembers={onToggleMembers}
            />
          )
        ) : null
      }
      channelType={activeChannel?.channelType}
      description={getChannelDescription(activeChannel)}
      statusBadge={
        <ChannelHeaderStatusBadge
          channelType={activeChannel?.channelType}
          ephemeralDisplay={activeChannelEphemeralDisplay}
          presenceStatus={activeDmPresenceStatus}
        />
      }
      title={activeChannelTitle}
      visibility={activeChannel?.visibility}
    />
  );
}
