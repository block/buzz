import * as React from "react";

import type { Channel } from "@/shared/api/types";

type ChannelNavigationContextValue = {
  channels: Channel[];
  /** Names of non-DM channels, memoised once so render-hot consumers (message
   *  rows) don't each re-filter the full channel list on every render. */
  nonDmChannelNames: string[];
};

const ChannelNavigationContext =
  React.createContext<ChannelNavigationContextValue>({
    channels: [],
    nonDmChannelNames: [],
  });

export function ChannelNavigationProvider({
  channels,
  children,
}: {
  channels: Channel[];
  children: React.ReactNode;
}) {
  const value = React.useMemo(
    () => ({
      channels,
      nonDmChannelNames: channels
        .filter((c) => c.channelType !== "dm")
        .map((c) => c.name),
    }),
    [channels],
  );

  return (
    <ChannelNavigationContext.Provider value={value}>
      {children}
    </ChannelNavigationContext.Provider>
  );
}

export function useChannelNavigation() {
  return React.useContext(ChannelNavigationContext);
}
