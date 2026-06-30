import * as React from "react";

import { getEphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import type { Channel } from "@/shared/api/types";

export function useEphemeralChannelDisplay(channel: Channel | null) {
  const [, setClockTick] = React.useState(0);
  const deadlineKey =
    channel?.ttlDeadline === null || channel === null
      ? null
      : `${channel.id}:${channel.ttlDeadline}`;

  React.useEffect(() => {
    if (deadlineKey === null) {
      return;
    }

    const intervalId = window.setInterval(() => {
      setClockTick((current) => current + 1);
    }, 60_000);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [deadlineKey]);

  const display = channel
    ? getEphemeralChannelDisplay(channel, Date.now())
    : null;
  // Stabilise the reference across renders when the rendered content is
  // unchanged: this object is recomputed from Date.now() every render, and it
  // feeds memoised header/timeline subtrees — a fresh ref each render would
  // defeat their React.memo on every background re-render.
  const stableRef = React.useRef(display);
  const prev = stableRef.current;
  if (
    prev?.detailLabel !== display?.detailLabel ||
    prev?.tooltipLabel !== display?.tooltipLabel
  ) {
    stableRef.current = display;
  }
  return stableRef.current;
}
