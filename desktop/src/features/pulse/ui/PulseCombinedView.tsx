import * as React from "react";
import type { Channel } from "@/shared/api/types";
import type { PulseConversation } from "../lib/unifiedFeed";
import { sortConversationsByRecency } from "../lib/conversationRecency";
import { PulseConversationSplitView } from "./PulseConversationSplitView";

export function PulseCombinedView({
  channels,
  conversations,
  currentPubkey,
  scrollRef,
  children,
  view,
  onSelectView,
}: {
  channels: Channel[];
  conversations: PulseConversation[];
  currentPubkey?: string;
  scrollRef: React.RefCallback<HTMLDivElement>;
  children: React.ReactNode;
  view: "search" | "all" | "conversation";
  onSelectView: (view: "search" | "all") => void;
}) {
  const sorted = React.useMemo(
    () => sortConversationsByRecency(channels, conversations),
    [channels, conversations],
  );
  return (
    <PulseConversationSplitView
      channels={sorted}
      currentPubkey={currentPubkey}
      selectionKey="conversation"
      label="Conversations by recent activity"
      testPrefix="pulse-combined"
      allMessages={{ content: children, scrollRef }}
      navigation={{ view, onSelectView }}
    />
  );
}
