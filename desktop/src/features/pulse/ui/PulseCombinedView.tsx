import * as React from "react";
import type { PulseView } from "../lib/workspaceNavigation";
import type { Channel } from "@/shared/api/types";
import type { PulseConversation } from "../lib/unifiedFeed";
import { sortConversationsByRecency } from "../lib/conversationRecency";
import { PulseConversationSplitView } from "./PulseConversationSplitView";

export function PulseCombinedView({
  grouped = false,
  channels,
  conversations,
  currentPubkey,
  scrollRef,
  children,
  view,
  onSelectView,
}: {
  grouped?: boolean;
  channels: Channel[];
  conversations: PulseConversation[];
  currentPubkey?: string;
  scrollRef: React.RefCallback<HTMLDivElement>;
  children: React.ReactNode;
  view: PulseView;
  onSelectView: (view: PulseView) => void;
}) {
  const sorted = React.useMemo(() => {
    const recent = sortConversationsByRecency(channels, conversations);
    return grouped
      ? [
          ...recent.filter((channel) => channel.channelType === "dm"),
          ...recent.filter((channel) => channel.channelType !== "dm"),
        ]
      : recent;
  }, [channels, conversations, grouped]);
  return (
    <PulseConversationSplitView
      channels={sorted}
      currentPubkey={currentPubkey}
      selectionKey="conversation"
      grouped={grouped}
      label={
        grouped
          ? "Direct messages and channels"
          : "Conversations by recent activity"
      }
      testPrefix="pulse-combined"
      allMessages={{ content: children, scrollRef }}
      navigation={{ view, onSelectView }}
    />
  );
}
