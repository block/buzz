import * as React from "react";

import {
  aggregateUnreadMains,
  type SubChannelIndex,
} from "@/features/dev-mode/lib/subChannels";

export function useDevUnreadNavigatorIds(
  subIndex: SubChannelIndex,
  unreadChannelIds: ReadonlySet<string>,
  highPriorityUnreadChannelIds: ReadonlySet<string>,
  blockedUnreadChannelIds: ReadonlySet<string>,
  sendFailureChannelIds: ReadonlySet<string>,
) {
  const navigatorUnreadIds = React.useMemo(
    () => aggregateUnreadMains(subIndex, unreadChannelIds),
    [subIndex, unreadChannelIds],
  );
  const navigatorHighPriorityIds = React.useMemo(
    () => aggregateUnreadMains(subIndex, highPriorityUnreadChannelIds),
    [subIndex, highPriorityUnreadChannelIds],
  );
  const navigatorBlockedIds = React.useMemo(
    () => aggregateUnreadMains(subIndex, blockedUnreadChannelIds),
    [subIndex, blockedUnreadChannelIds],
  );
  // A failed send in a sub tab lights the parent's row — subs have no row
  // of their own in the left list.
  const navigatorSendFailureIds = React.useMemo(
    () => aggregateUnreadMains(subIndex, sendFailureChannelIds),
    [subIndex, sendFailureChannelIds],
  );
  return {
    navigatorBlockedIds,
    navigatorHighPriorityIds,
    navigatorSendFailureIds,
    navigatorUnreadIds,
  };
}
