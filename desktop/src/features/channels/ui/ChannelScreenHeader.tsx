import { LogIn } from "lucide-react";
import { createPortal } from "react-dom";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { getChannelDescription } from "@/features/channels/lib/channelDescription";
import { ChannelHeaderStatusBadge } from "@/features/channels/ui/ChannelHeaderStatusBadge";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import { Button } from "@/shared/ui/button";
import type { Channel, PresenceStatus } from "@/shared/api/types";

type ChannelScreenHeaderProps = {
  activeChannel: Channel | null;
  activeChannelEphemeralDisplay: EphemeralChannelDisplay | null;
  activeChannelTitle: string;
  actionsRightInset?: string;
  activeDmPresenceStatus: PresenceStatus | null;
  currentPubkey?: string;
  isJoining?: boolean;
  showHeaderContent?: boolean;
  onJoinChannel?: () => Promise<void>;
  onManageChannel: () => void;
  onToggleMembers: () => void;
};

export function ChannelScreenHeader({
  activeChannel,
  activeChannelEphemeralDisplay,
  activeChannelTitle,
  actionsRightInset,
  activeDmPresenceStatus,
  currentPubkey,
  isJoining = false,
  showHeaderContent = true,
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

  const actions = activeChannel ? (
    showJoinButton ? (
      <Button
        disabled={isJoining}
        onClick={() => void onJoinChannel()}
        size="sm"
        variant="default"
      >
        <LogIn className="mr-1.5 h-3.5 w-3.5" />
        {isJoining ? "Joining…" : "Join"}
      </Button>
    ) : (
      <ChannelMembersBar
        channel={activeChannel}
        currentPubkey={currentPubkey}
        onManageChannel={onManageChannel}
        onToggleMembers={onToggleMembers}
      />
    )
  ) : null;

  if (!showHeaderContent) {
    if (typeof document === "undefined") {
      return null;
    }

    return createPortal(
      <div className="fixed right-3 top-[9px] z-[45] flex shrink-0 items-center gap-1">
        <UpdateIndicator />
        {actions ? <div className="shrink-0">{actions}</div> : null}
      </div>,
      document.body,
    );
  }

  return (
    <ChatHeader
      belowSystemChrome
      density="compact"
      actions={actions}
      actionsRightInset={actionsRightInset}
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
