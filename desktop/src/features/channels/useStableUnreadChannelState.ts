import * as React from "react";

import { mapsEqual } from "./unreadChannelCounts";

export type UnreadChannelState = {
  unreadChannelIds: ReadonlySet<string>;
  topLevelUnreadChannelIds: ReadonlySet<string>;
  highPriorityUnreadChannelIds: ReadonlySet<string>;
  blockedUnreadChannelIds: ReadonlySet<string>;
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelNotificationCount: number;
};

function setsEqual(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const item of a) if (!b.has(item)) return false;
  return true;
}

function stableSet(
  next: ReadonlySet<string>,
  previous: React.RefObject<ReadonlySet<string>>,
) {
  const stable = setsEqual(next, previous.current) ? previous.current : next;
  previous.current = stable;
  return stable;
}

export function useStableUnreadChannelState(
  state: UnreadChannelState,
): UnreadChannelState {
  const unreadRef = React.useRef<ReadonlySet<string>>(new Set());
  const topLevelRef = React.useRef<ReadonlySet<string>>(new Set());
  const highPriorityRef = React.useRef<ReadonlySet<string>>(new Set());
  const blockedRef = React.useRef<ReadonlySet<string>>(new Set());
  const countsRef = React.useRef<ReadonlyMap<string, number>>(new Map());

  const counts = mapsEqual(state.unreadChannelCounts, countsRef.current)
    ? countsRef.current
    : state.unreadChannelCounts;
  countsRef.current = counts;

  return {
    ...state,
    unreadChannelIds: stableSet(state.unreadChannelIds, unreadRef),
    topLevelUnreadChannelIds: stableSet(
      state.topLevelUnreadChannelIds,
      topLevelRef,
    ),
    highPriorityUnreadChannelIds: stableSet(
      state.highPriorityUnreadChannelIds,
      highPriorityRef,
    ),
    blockedUnreadChannelIds: stableSet(
      state.blockedUnreadChannelIds,
      blockedRef,
    ),
    unreadChannelCounts: counts,
  };
}
