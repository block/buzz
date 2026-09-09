import type { Channel } from "@/shared/api/types";
import { PulseConversationSplitView } from "./PulseConversationSplitView";

export function PulseDmView({
  channels,
  currentPubkey,
  search,
}: {
  channels: Channel[];
  currentPubkey?: string;
  search: string;
}) {
  return (
    <PulseConversationSplitView
      channels={channels.filter((c) => c.channelType === "dm")}
      currentPubkey={currentPubkey}
      search={search}
      selectionKey="dm"
      label="Direct messages"
      testPrefix="pulse-dm"
    />
  );
}
