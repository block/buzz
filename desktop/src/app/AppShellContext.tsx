import * as React from "react";
import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";

type AppShellContextValue = {
  markAllChannelsRead: () => void;
  markChannelRead: (
    channelId: string,
    readAt: string | null | undefined,
  ) => void;
  markChannelUnread: (channelId: string) => void;
  openCreateChannel: () => void;
  openChannelManagement: () => void;
  // NIP-RS read marker for a channel as a unix-seconds timestamp, or null
  // when unknown. Backed by the single AppShell-mounted ReadStateManager so
  // every surface (sidebar, home, badges) projects from the same source.
  getChannelReadAt: (channelId: string) => number | null;
  // Bump-counter that invalidates whenever the read marker changes. Include
  // in memo deps that consume getChannelReadAt.
  readStateVersion: number;
  followThread: (rootId: string) => void;
  unfollowThread: (rootId: string) => void;
  isFollowingThread: (rootId: string) => boolean;
  isNotifiedForThread: (rootId: string) => boolean;
  setTopbarSearchHidden: (hidden: boolean) => void;
  threadActivityItems: ThreadActivityItem[];
};

const AppShellContext = React.createContext<AppShellContextValue>({
  markAllChannelsRead: () => {},
  markChannelRead: () => {},
  markChannelUnread: () => {},
  openCreateChannel: () => {},
  openChannelManagement: () => {},
  getChannelReadAt: () => null,
  readStateVersion: 0,
  followThread: () => {},
  unfollowThread: () => {},
  isFollowingThread: () => false,
  isNotifiedForThread: () => false,
  setTopbarSearchHidden: () => {},
  threadActivityItems: [],
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
