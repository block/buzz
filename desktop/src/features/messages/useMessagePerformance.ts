import * as React from "react";

import {
  CHANNEL_ROWS_PAINTED_MARK,
  CHANNEL_SWITCH_MEASURE,
  CHANNEL_SWITCH_START_MARK,
  THREAD_OPEN_MEASURE,
  THREAD_OPEN_START_MARK,
  THREAD_REPLIES_PAINTED_MARK,
  finishPerformanceMeasure,
  startPerformanceMark,
} from "@/features/messages/lib/messagePerformance";
import type { TimelineMessage } from "@/features/messages/types";

export function useChannelSelectionPerformanceMark(
  activeChannelId: string | null,
): void {
  React.useEffect(() => {
    const markChannelSelection = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      const channelId = target
        .closest<HTMLElement>("[data-channel-id]")
        ?.getAttribute("data-channel-id");
      if (channelId && channelId !== activeChannelId) {
        startPerformanceMark(CHANNEL_SWITCH_START_MARK);
      }
    };
    document.addEventListener("click", markChannelSelection, true);
    return () =>
      document.removeEventListener("click", markChannelSelection, true);
  }, [activeChannelId]);
}

export function useMeasuredOpenThread(
  openThreadHeadId: string | null,
  onOpenThread: (message: TimelineMessage) => void,
): (message: TimelineMessage) => void {
  return React.useCallback(
    (message: TimelineMessage) => {
      if (openThreadHeadId !== message.id) {
        startPerformanceMark(THREAD_OPEN_START_MARK);
      }
      onOpenThread(message);
    },
    [onOpenThread, openThreadHeadId],
  );
}

function useRowsPaintedPerformanceMeasure(
  identity: string | null,
  rowCount: number,
  endMark: string,
  measure: string,
  startMark: string,
): void {
  const measuredIdentityRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (
      !identity ||
      rowCount === 0 ||
      measuredIdentityRef.current === identity
    ) {
      return;
    }
    const frame = requestAnimationFrame(() => {
      finishPerformanceMeasure({ startMark, endMark, measure });
      measuredIdentityRef.current = identity;
    });
    return () => cancelAnimationFrame(frame);
  }, [endMark, identity, measure, rowCount, startMark]);
}

export function useChannelRowsPaintedPerformanceMeasure(
  channelId: string | null,
  rowCount: number,
): void {
  useRowsPaintedPerformanceMeasure(
    channelId,
    rowCount,
    CHANNEL_ROWS_PAINTED_MARK,
    CHANNEL_SWITCH_MEASURE,
    CHANNEL_SWITCH_START_MARK,
  );
}

export function useThreadRepliesPaintedPerformanceMeasure(
  threadHeadId: string | null,
  rowCount: number,
): void {
  useRowsPaintedPerformanceMeasure(
    threadHeadId,
    rowCount,
    THREAD_REPLIES_PAINTED_MARK,
    THREAD_OPEN_MEASURE,
    THREAD_OPEN_START_MARK,
  );
}
