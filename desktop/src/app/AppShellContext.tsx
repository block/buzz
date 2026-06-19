import * as React from "react";
import type { ContextParentResolver } from "@/features/channels/readState/readStateManager";
import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";

type AppShellContextValue = {
  markAllChannelsRead: () => void;
  markChannelRead: (
    channelId: string,
    readAt: string | null | undefined,
    options?: { topLevelOnly?: boolean },
  ) => void;
  markChannelUnread: (channelId: string) => void;
  openCreateChannel: () => void;
  openChannelManagement: () => void;
  // NIP-RS read marker for a channel as a unix-seconds timestamp, or null
  // when unknown. Backed by the single AppShell-mounted ReadStateManager so
  // every surface (sidebar, home, badges) projects from the same source.
  getChannelReadAt: (channelId: string) => number | null;
  // Thread read frontier as unix-seconds timestamp, or null when never read.
  // Uses `thread:<rootId>` context keys in the same ReadStateManager.
  getThreadReadAt: (rootId: string) => number | null;
  // Advance the thread read frontier to the given unix-seconds timestamp.
  markThreadRead: (rootId: string, timestamp: number) => void;
  // Bump-counter that invalidates whenever the read marker changes. Include
  // in memo deps that consume getChannelReadAt.
  readStateVersion: number;
  // Inject the thread→channel parent resolver derived from the event graph
  // (NIP-RS hierarchical frontier). Set by the active channel surface.
  setContextParentResolver: (resolver: ContextParentResolver | null) => void;
  followThread: (rootId: string) => void;
  unfollowThread: (rootId: string) => void;
  isFollowingThread: (rootId: string) => boolean;
  isNotifiedForThread: (rootId: string) => boolean;
  threadActivityItems: ThreadActivityItem[];
};

const AppShellContext = React.createContext<AppShellContextValue>({
  markAllChannelsRead: () => {},
  markChannelRead: () => {},
  markChannelUnread: () => {},
  openCreateChannel: () => {},
  openChannelManagement: () => {},
  getChannelReadAt: () => null,
  getThreadReadAt: () => null,
  markThreadRead: () => {},
  readStateVersion: 0,
  setContextParentResolver: () => {},
  followThread: () => {},
  unfollowThread: () => {},
  isFollowingThread: () => false,
  isNotifiedForThread: () => false,
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
