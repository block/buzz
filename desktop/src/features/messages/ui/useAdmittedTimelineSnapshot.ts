import * as React from "react";
import type { TimelineMessage } from "@/features/messages/types";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import { useBufferedTimelineMessages } from "./useBufferedTimelineMessages";
import { useSettleGatedPrependMessages } from "./useSettleGatedPrependMessages";

export type TimelineSnapshot = {
  channelId: string | null;
  messages: TimelineMessage[];
  historyExhausted: boolean;
  historyRevision: number;
  firstUnreadMessageId: string | null;
  threadSummaries?: ReadonlyMap<string, ChannelWindowThreadSummary>;
  mainEntries?: MainTimelineEntry[];
};
const EMPTY: TimelineSnapshot = {
  channelId: null,
  messages: [],
  historyExhausted: false,
  historyRevision: 0,
  firstUnreadMessageId: null,
};

/** Rows and structural metadata take the same concurrency/admission path.
 * Holding only messages would mint dividers or regroup rows against old data.
 */
export function useAdmittedTimelineSnapshot({
  snapshot,
  isAtBottom,
  scrollElementRef,
}: {
  snapshot: TimelineSnapshot;
  isAtBottom: boolean;
  scrollElementRef: { readonly current: HTMLElement | null };
}) {
  const deferred = React.useDeferredValue(snapshot, EMPTY);
  const buffered = useBufferedTimelineMessages({
    channelId: snapshot.channelId,
    isAtBottom,
    messages: deferred.messages,
  });
  const admitted = useSettleGatedPrependMessages({
    channelId: snapshot.channelId,
    messages: buffered.messages,
    meta: deferred,
    bypass: isAtBottom,
    scrollElementRef,
  });
  return { deferred, admitted, pendingCount: buffered.pendingCount };
}
