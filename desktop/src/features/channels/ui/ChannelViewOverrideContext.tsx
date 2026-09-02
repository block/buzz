import * as React from "react";

type ChannelViewOverride = {
  headerNavigation: React.ReactNode;
  hideMainColumnBody?: boolean;
  isChannelViewActive: boolean;
  mainColumnHeader?: React.ReactNode;
  mainColumnHeaderPlacement?: "top" | "right";
  mainContent: React.ReactNode;
  onSelectChannelView: () => void;
};

const ChannelViewOverrideContext =
  React.createContext<ChannelViewOverride | null>(null);

export function ChannelViewOverrideProvider({
  children,
  value,
}: {
  children: React.ReactNode;
  value: ChannelViewOverride;
}) {
  return (
    <ChannelViewOverrideContext.Provider value={value}>
      {children}
    </ChannelViewOverrideContext.Provider>
  );
}

export function useChannelViewOverride() {
  return React.useContext(ChannelViewOverrideContext);
}
