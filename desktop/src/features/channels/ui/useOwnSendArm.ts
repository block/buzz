import * as React from "react";

import type { MessageTimelineHandle } from "@/features/messages/ui/MessageTimeline";

type OwnSendTimeline = Pick<
  MessageTimelineHandle,
  "cancelScrollToBottomOnNextUpdate" | "scrollToBottomOnNextUpdate"
>;

const noDisarm = () => {};

/**
 * Arms the timeline's own-send pin for a send to `channelId` and returns its
 * disarm for a failed send.
 *
 * A send can settle after the pane moved to another channel (a slow upload
 * spans the switch), so the arm is skipped unless `channelId` is the channel
 * the timeline shows now, read live from `getTimelineChannelId`. A skipped arm
 * leaves the generation alone, so it can't strand an earlier send's disarm.
 * The generation keeps a late failure from disarming a later send's pin.
 */
export function useOwnSendArm(
  timelineRef: React.RefObject<OwnSendTimeline | null>,
  getTimelineChannelId: () => string | null,
) {
  const generationRef = React.useRef(0);
  return React.useCallback(
    (channelId: string | null) => {
      if (!channelId || channelId !== getTimelineChannelId()) {
        return noDisarm;
      }
      const generation = ++generationRef.current;
      timelineRef.current?.scrollToBottomOnNextUpdate(channelId);
      return () => {
        if (generationRef.current !== generation) return;
        timelineRef.current?.cancelScrollToBottomOnNextUpdate();
      };
    },
    [getTimelineChannelId, timelineRef],
  );
}
