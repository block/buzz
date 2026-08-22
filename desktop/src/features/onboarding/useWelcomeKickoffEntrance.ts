import * as React from "react";

import { useWelcomeKickoff } from "@/features/onboarding/welcomeKickoff";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * Runs the Welcome kickoff choreography and tracks the one-shot entrance
 * animation for the newly posted opener.
 *
 * The opener lands in the main timeline with a one-shot entrance; the user
 * opens the thread themselves. Auto-opening it caused a cascade of loading
 * states (thread skeleton, "No replies" flash) on first run.
 */
export function useWelcomeKickoffEntrance(
  activeChannel: Channel | null,
  resolvedMessages: readonly RelayEvent[],
  threadReplyEvents: readonly RelayEvent[],
) {
  const welcomeKickoffEvents = React.useMemo(
    () => [...resolvedMessages, ...threadReplyEvents],
    [resolvedMessages, threadReplyEvents],
  );
  const [entranceMessageId, setEntranceMessageId] = React.useState<
    string | null
  >(null);
  React.useEffect(() => {
    void activeChannel?.id;
    setEntranceMessageId(null);
  }, [activeChannel?.id]);
  const handleKickoffOpenerPosted = React.useCallback((eventId: string) => {
    setEntranceMessageId(eventId);
  }, []);
  const handleEntranceComplete = React.useCallback((eventId: string) => {
    setEntranceMessageId((current) => (current === eventId ? null : current));
  }, []);
  useWelcomeKickoff(
    activeChannel,
    welcomeKickoffEvents,
    handleKickoffOpenerPosted,
  );

  return { entranceMessageId, handleEntranceComplete };
}
