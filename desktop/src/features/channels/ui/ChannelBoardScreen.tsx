import * as React from "react";

import {
  useChannelMembersQuery,
  useJoinChannelMutation,
} from "@/features/channels/hooks";
import { useActiveChannelHeader } from "@/features/channels/useActiveChannelHeader";
import { ChannelBoard } from "@/features/channels/ui/ChannelBoard";
import { ChannelManagementSheet } from "@/features/channels/ui/ChannelManagementSheet";
import { ChannelScreenHeader } from "@/features/channels/ui/ChannelScreenHeader";
import { MembersSidebar } from "@/features/channels/ui/MembersSidebar";
import { useChannelViewMode } from "@/features/channels/ui/ChannelViewModeContext";
import { useCommunities } from "@/features/communities/useCommunities";
import type { CanvasResponse, Channel } from "@/shared/api/types";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_SHORT,
} from "@/shared/lib/relayError";

type ChannelBoardScreenProps = {
  canvas: CanvasResponse | undefined;
  canvasError: unknown;
  canvasLoading: boolean;
  channel: Channel;
  currentPubkey?: string;
};

export function ChannelBoardScreen({
  canvas,
  canvasError,
  canvasLoading,
  channel,
  currentPubkey,
}: ChannelBoardScreenProps) {
  const { activeCommunity } = useCommunities();
  const channelViewMode = useChannelViewMode();
  const membersQuery = useChannelMembersQuery(channel.id);
  const joinChannelMutation = useJoinChannelMutation(channel.id);
  const members = membersQuery.data ?? [];
  const agentCount = members.filter(
    (member) => member.isAgent || member.role === "bot",
  ).length;
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  const [isChannelManagementOpen, setIsChannelManagementOpen] =
    React.useState(false);
  const [isAddBotOpen, setIsAddBotOpen] = React.useState(false);
  const {
    activeChannelEphemeralDisplay,
    activeChannelTitle,
    activeDmAvatarUrl,
    activeDmHeaderParticipants,
    activeDmPresenceStatus,
  } = useActiveChannelHeader(channel, currentPubkey);
  const canvasErrorMessage =
    canvasError instanceof Error
      ? isRelayUnreachableError(canvasError)
        ? RELAY_UNREACHABLE_SHORT
        : canvasError.message
      : undefined;

  return (
    <React.Fragment>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <ChannelScreenHeader
          activeChannel={channel}
          activeChannelEphemeralDisplay={activeChannelEphemeralDisplay}
          activeChannelTitle={activeChannelTitle}
          activeDmAvatarUrl={activeDmAvatarUrl}
          activeDmHeaderParticipants={activeDmHeaderParticipants}
          activeDmPresenceStatus={activeDmPresenceStatus}
          currentPubkey={currentPubkey}
          isAddBotOpen={isAddBotOpen}
          isJoining={joinChannelMutation.isPending}
          onAddBotOpenChange={setIsAddBotOpen}
          onJoinChannel={joinChannelMutation.mutateAsync}
          onManageChannel={() => setIsChannelManagementOpen(true)}
          onToggleMembers={() => setIsMembersSidebarOpen((open) => !open)}
        />
        <ChannelBoard
          agentCount={agentCount}
          author={canvas?.author ?? null}
          channelName={activeChannelTitle}
          content={canvas?.content ?? null}
          errorMessage={canvasErrorMessage}
          isLoading={canvasLoading}
          memberCount={members.length || channel.memberCount}
          onManageBoard={() => setIsChannelManagementOpen(true)}
          onOpenMembers={() => setIsMembersSidebarOpen(true)}
          onOpenStream={() => channelViewMode.onModeChange("stream")}
          updatedAt={canvas?.updatedAt ?? null}
        />
      </div>

      <ChannelManagementSheet
        channel={channel}
        currentPubkey={currentPubkey}
        onOpenChange={setIsChannelManagementOpen}
        open={isChannelManagementOpen}
      />
      <MembersSidebar
        channel={channel}
        currentPubkey={currentPubkey}
        onOpenChange={setIsMembersSidebarOpen}
        open={isMembersSidebarOpen}
        relayUrl={activeCommunity?.relayUrl}
      />
    </React.Fragment>
  );
}
