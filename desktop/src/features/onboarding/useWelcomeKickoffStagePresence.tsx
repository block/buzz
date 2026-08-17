import * as React from "react";

import { isWelcomeSetupSystemMessage } from "@/features/channels/ui/ChannelPane.helpers";
import type { TimelineMessage } from "@/features/messages/types";
import { WelcomeKickoffStage } from "@/features/onboarding/ui/WelcomeKickoffStage";
import {
  isWelcomeKickoffSettingUp,
  useWelcomeKickoffStage,
} from "@/features/onboarding/useWelcomeKickoffStage";
import {
  isWelcomeChannel,
  notifyWelcomeSurfaceReady,
} from "@/features/onboarding/welcome";
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
  // Announce the Welcome surface's first settled render (per channel) so the
  // onboarding "entering" curtain knows it can fade. Harmless outside
  // onboarding — nothing listens unless the curtain is up.
  const announcedChannelIdRef = React.useRef<string | null>(null);
  const channelId = activeChannel?.id ?? null;
  React.useEffect(() => {
    if (!channelId || isTimelineLoading) return;
    if (!isWelcomeChannel(activeChannel)) return;
    if (announcedChannelIdRef.current === channelId) return;
    announcedChannelIdRef.current = channelId;
    notifyWelcomeSurfaceReady(channelId);
  }, [activeChannel, channelId, isTimelineLoading]);
  const welcomeKickoffStage =
    phase !== "hidden" ? (
      <WelcomeKickoffStage onExitComplete={handleExitComplete} phase={phase} />
    ) : null;
  return {
    welcomeKickoffStage,
    welcomeKickoffSettingUp: isWelcomeKickoffSettingUp(phase),
  };
}
