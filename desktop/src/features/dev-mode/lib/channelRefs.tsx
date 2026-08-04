import * as React from "react";

/**
 * Channel references for developer mode: `#channel-name` in message text is
 * the wire format the standard UI and mobile already render as channel links,
 * so dev mode reads and writes the same plain text. The shell provides the
 * known channels and its open handler once; composers use them for `#`
 * autocomplete and message rows use them to render clickable references.
 */

export type ChannelRef = { id: string; name: string };

type ChannelRefsValue = {
  channels: ChannelRef[];
  openChannel: (channelId: string) => void;
};

const ChannelRefsContext = React.createContext<ChannelRefsValue>({
  channels: [],
  openChannel: () => {},
});

export function DevChannelRefsProvider({
  channels,
  openChannel,
  children,
}: ChannelRefsValue & { children: React.ReactNode }) {
  const value = React.useMemo(
    () => ({ channels, openChannel }),
    [channels, openChannel],
  );
  return (
    <ChannelRefsContext.Provider value={value}>
      {children}
    </ChannelRefsContext.Provider>
  );
}

export function useChannelRefs(): ChannelRefsValue {
  return React.useContext(ChannelRefsContext);
}
