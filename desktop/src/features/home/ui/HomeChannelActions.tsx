import { createPortal } from "react-dom";

import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import type { Channel } from "@/shared/api/types";

type HomeChannelActionsProps = {
  channel: Channel | null;
  currentPubkey?: string;
  onOpenChannel: (channelId: string) => void;
};

export function HomeChannelActions({
  channel,
  currentPubkey,
  onOpenChannel,
}: HomeChannelActionsProps) {
  if (!channel || typeof document === "undefined") {
    return null;
  }

  return createPortal(
    <div className="fixed right-3 top-[9px] z-[45] flex shrink-0 items-center gap-1">
      <UpdateIndicator />
      <ChannelMembersBar
        channel={channel}
        currentPubkey={currentPubkey}
        onManageChannel={() => onOpenChannel(channel.id)}
        onToggleMembers={() => onOpenChannel(channel.id)}
      />
    </div>,
    document.body,
  );
}
