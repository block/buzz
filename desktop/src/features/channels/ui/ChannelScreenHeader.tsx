import { LogIn } from "lucide-react";
import type * as React from "react";

import { ChatHeader } from "@/features/chat/ui/ChatHeader";
import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import type { ActiveDmHeaderParticipant } from "@/features/channels/useActiveChannelHeader";
import { getChannelDescription } from "@/features/channels/lib/channelDescription";
import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import { ChannelHeaderStatusBadge } from "@/features/channels/ui/ChannelHeaderStatusBadge";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { Button } from "@/shared/ui/button";
import type { Channel, PresenceStatus } from "@/shared/api/types";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ChannelScreenHeaderProps = {
  activeChannel: Channel | null;
  activeChannelEphemeralDisplay: EphemeralChannelDisplay | null;
  activeChannelTitle: string;
  actionsVariant?: "inline" | "compact";
  activeDmAvatarUrl: string | null;
  activeDmHeaderParticipants: ActiveDmHeaderParticipant[];
  activeDmPresenceStatus: PresenceStatus | null;
  chromeWrapperRef?: React.Ref<HTMLDivElement>;
  currentPubkey?: string;
  isAddBotOpen?: boolean;
  isJoining?: boolean;
  showHeaderContent?: boolean;
  transparentChrome?: boolean;
  onAddBotOpenChange?: (open: boolean) => void;
  onJoinChannel?: () => Promise<void>;
  onManageChannel: () => void;
  onToggleMembers: () => void;
};

export function ChannelScreenHeader({
  activeChannel,
  activeChannelEphemeralDisplay,
  activeChannelTitle,
  actionsVariant = "inline",
  activeDmAvatarUrl,
  activeDmHeaderParticipants,
  activeDmPresenceStatus,
  chromeWrapperRef,
  currentPubkey,
  isAddBotOpen,
  isJoining = false,
  onAddBotOpenChange,
  showHeaderContent = true,
  transparentChrome = false,
  onJoinChannel,
  onManageChannel,
  onToggleMembers,
}: ChannelScreenHeaderProps) {
  const isGroupDm =
    activeChannel?.channelType === "dm" &&
    activeDmHeaderParticipants.length > 1;
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
        <LogIn className="mr-1.5 h-4 w-4" />
        {isJoining ? "Joining…" : "Join"}
      </Button>
    ) : (
      <ChannelMembersBar
        channel={activeChannel}
        currentPubkey={currentPubkey}
        isAddBotOpen={isAddBotOpen}
        onAddBotOpenChange={onAddBotOpenChange}
        onManageChannel={onManageChannel}
        onToggleMembers={onToggleMembers}
        variant={actionsVariant}
      />
    )
  ) : null;

  if (!showHeaderContent) {
    return null;
  }

  return (
    <ChatHeader
      belowSystemChrome
      chromeWrapperRef={chromeWrapperRef}
      actions={actions}
      channelType={activeChannel?.channelType}
      description={getChannelDescription(activeChannel)}
      leadingContent={
        activeChannel?.channelType === "dm" ? (
          isGroupDm ? (
            <DmHeaderParticipantStack
              participants={activeDmHeaderParticipants}
            />
          ) : (
            <ProfileAvatar
              avatarUrl={activeDmAvatarUrl}
              className="h-6 w-6 rounded-full text-2xs"
              iconClassName="h-3.5 w-3.5"
              label={activeChannelTitle}
              testId="chat-header-dm-avatar"
            />
          )
        ) : undefined
      }
      statusBadge={
        <ChannelHeaderStatusBadge
          channelType={activeChannel?.channelType}
          ephemeralDisplay={activeChannelEphemeralDisplay}
          presenceStatus={isGroupDm ? null : activeDmPresenceStatus}
        />
      }
      title={activeChannelTitle}
      transparentChrome={transparentChrome}
      visibility={activeChannel?.visibility}
    />
  );
}

function DmHeaderParticipantStack({
  participants,
}: {
  participants: ActiveDmHeaderParticipant[];
}) {
  const { hiddenCount, visibleParticipants } =
    getDmParticipantPreview(participants);
  const stackItemCount = visibleParticipants.length + (hiddenCount > 0 ? 1 : 0);

  return (
    <div
      aria-hidden="true"
      className="mr-1 flex shrink-0 items-center"
      data-testid="chat-header-dm-avatar-stack"
    >
      {visibleParticipants.map((participant, index) => (
        <div
          className={index > 0 ? "-ml-2" : ""}
          data-testid="chat-header-dm-avatar-stack-participant"
          key={participant.pubkey}
          style={{
            zIndex: index + 1,
            ...(index < stackItemCount - 1 && {
              mask: "radial-gradient(circle 16px at calc(100% + 4px) 50%, transparent 99%, #fff 100%)",
              WebkitMask:
                "radial-gradient(circle 16px at calc(100% + 4px) 50%, transparent 99%, #fff 100%)",
            }),
          }}
        >
          <UserAvatar
            avatarUrl={participant.avatarUrl}
            className="h-7 w-7 text-2xs"
            displayName={participant.displayName}
            size="sm"
          />
        </div>
      ))}
      {hiddenCount > 0 ? (
        <div
          className={visibleParticipants.length > 0 ? "-ml-2" : ""}
          data-testid="chat-header-dm-avatar-stack-more"
          style={{ zIndex: stackItemCount }}
        >
          <span className="flex h-7 w-7 items-center justify-center rounded-full bg-secondary font-semibold text-secondary-foreground shadow-xs">
            <span className="text-2xs leading-none">+{hiddenCount}</span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
