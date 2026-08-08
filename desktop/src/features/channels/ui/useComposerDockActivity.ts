import * as React from "react";

import { useCardMintJobs } from "@/features/agents/cardMintStore";
import { useChannelWorkingAgentPubkeys } from "@/features/agents/agentWorkingSignal";

/**
 * Composer-dock activity signals that must NOT be subscribed from ChannelPane /
 * ChannelScreen. Those parents own the timeline + message composer; re-rendering
 * them on every typing/working/card-mint tick is the quiet→busy typing-latency
 * regression. Leaf dock chrome (frame class + activity accessory) owns this
 * hook instead.
 */
export function useComposerDockActivity(
  channelId: string | null | undefined,
  typingPubkeys: readonly string[],
): {
  hasActivity: boolean;
  workingBotPubkeys: string[];
} {
  const workingBotPubkeys = useChannelWorkingAgentPubkeys(channelId);
  const cardMintJobs = useCardMintJobs();
  const hasTypingActivity = typingPubkeys.length > 0;
  const hasComposerBotActivity = workingBotPubkeys.length > 0;
  const hasCardMintActivity = cardMintJobs.length > 0;
  const hasActivity =
    hasComposerBotActivity || hasTypingActivity || hasCardMintActivity;
  return React.useMemo(
    () => ({ hasActivity, workingBotPubkeys }),
    [hasActivity, workingBotPubkeys],
  );
}
