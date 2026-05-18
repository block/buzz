import * as React from "react";

type AppShellContextValue = {
  markChannelRead: (
    channelId: string,
    readAt: string | null | undefined,
  ) => void;
  markChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  openChannelManagement: () => void;
};

const AppShellContext = React.createContext<AppShellContextValue>({
  markChannelRead: () => {},
  markChannelUnread: () => {},
  openChannelManagement: () => {},
});

export function AppShellProvider({
  children,
  value,
}: {
  children: React.ReactNode;
  value: AppShellContextValue;
}) {
  return (
    <AppShellContext.Provider value={value}>
      {children}
    </AppShellContext.Provider>
  );
}

export function useAppShell() {
  return React.useContext(AppShellContext);
}
