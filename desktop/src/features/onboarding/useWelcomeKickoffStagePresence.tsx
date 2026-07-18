import * as React from "react";

import { isWelcomeSetupSystemMessage } from "@/features/channels/ui/ChannelPane.helpers";
import type { TimelineMessage } from "@/features/messages/types";
import { WelcomeKickoffStage } from "@/features/onboarding/ui/WelcomeKickoffStage";
import { useWelcomeKickoffStage } from "@/features/onboarding/useWelcomeKickoffStage";
import type { Channel } from "@/shared/api/types";

/**
 * Composes the Welcome kickoff stage for the channel screen: gates on the
 * timeline's *visible* rows and returns the rendered stage element plus the
 * "still setting up" flag for the composer banner copy.
 *
 * Welcome setup system messages (channel_created / member_joined) render no
 * timeline rows — ChannelPane filters them out of the visible list. The stage
 * gates on the same visibility rule, or a "blank" Welcome channel counts as
 * non-empty and the stage never shows.
 */
export function useWelcomeKickoffStagePresence(
  activeChannel: Channel | null,
  timelineMessages: readonly TimelineMessage[],
  isTimelineLoading: boolean,
) {
  const hasVisibleTimelineMessages = React.useMemo(
    () =>
      timelineMessages.some((message) => !isWelcomeSetupSystemMessage(message)),
    [timelineMessages],
  );
  const { phase, handleExitComplete } = useWelcomeKickoffStage(
    activeChannel,
    hasVisibleTimelineMessages,
    isTimelineLoading,
  );
  const welcomeKickoffStage =
    phase !== "hidden" ? (
      <WelcomeKickoffStage onExitComplete={handleExitComplete} phase={phase} />
    ) : null;
  return {
    welcomeKickoffStage,
    welcomeKickoffSettingUp: phase === "active" || phase === "timed-out",
  };
}
