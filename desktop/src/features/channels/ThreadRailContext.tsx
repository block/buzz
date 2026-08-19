import * as React from "react";

import type { ThreadRailPin } from "./threadRailStorage";
import type { ThreadRail } from "./useThreadRail";

const EMPTY_RAIL: ThreadRail = {
  pins: [],
  collapsed: false,
  isScoped: false,
  pin: () => {},
  unpin: () => {},
  updateAnchor: () => {},
  updateExpandedReplyIds: () => {},
  toggleCollapsed: () => {},
};

const ThreadRailContext = React.createContext<ThreadRail>(EMPTY_RAIL);

export function ThreadRailProvider({
  children,
  rail,
}: {
  children: React.ReactNode;
  rail: ThreadRail;
}) {
  return (
    <ThreadRailContext.Provider value={rail}>
      {children}
    </ThreadRailContext.Provider>
  );
}

export function useThreadRailContext(): ThreadRail {
  return React.useContext(ThreadRailContext);
}

export function makeThreadRailPin({
  channelId,
  channelName,
  expandedReplyIds,
  returnAnchorId,
  rootExcerpt,
  rootId,
}: Omit<ThreadRailPin, "pinnedAt">): ThreadRailPin {
  return {
    channelId,
    channelName,
    expandedReplyIds,
    returnAnchorId,
    rootExcerpt,
    rootId,
    pinnedAt: Date.now(),
  };
}
