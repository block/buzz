import * as React from "react";
import type { ContextParentResolver } from "@/features/channels/readState/readStateManager";
import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";
import type { FeedItemState } from "@/features/home/useFeedItemState";
import type { FeedItem } from "@/shared/api/types";
import type { SettingsSection } from "@/features/settings/ui/SettingsPanels";

const EMPTY_SET = new Set<string>();

type AppShellContextValue = {
  markAllChannelsRead: () => void;
  markChannelRead: (
    channelId: string,
    readAt: string | null | undefined,
    options?: { topLevelOnly?: boolean },
  ) => void;
  markChannelUnread: (channelId: string) => void;
  openBrowseChannels: () => void;
  openCreateChannel: () => void;
  openChannelManagement: (channelId?: string) => void;
  // NIP-RS read marker for a channel as a unix-seconds timestamp, or null
  // when unknown. Backed by the single AppShell-mounted ReadStateManager so
  // every surface (sidebar, home, badges) projects from the same source.
  getChannelReadAt: (channelId: string) => number | null;
  // Thread read frontier as unix-seconds timestamp, or null when never read.
  // Uses `thread:<rootId>` context keys in the same ReadStateManager.
  getThreadReadAt: (rootId: string, channelId?: string | null) => number | null;
  // Advance the thread read frontier to the given unix-seconds timestamp.
  markThreadRead: (rootId: string, timestamp: number) => void;
  // Per-message read frontier as unix-seconds timestamp, or null when never
  // read. Uses `msg:<id>` context keys folded through the active channel by the
  // parent resolver (LP4 v3 per-message badge model).
  getMessageReadAt: (messageId: string) => number | null;
  // Advance a single message's read marker to the given unix-seconds timestamp.
  markMessageRead: (messageId: string, timestamp: number) => void;
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
  isThreadMuted: (rootId: string) => boolean;
  threadActivityItems: ThreadActivityItem[];
  threadActivityFeedItems: FeedItem[];
  feedItemState: FeedItemState;
  // Open the Settings panel at the given section. Available on all surfaces
  // that render under AppShell (channel, home, projects, pulse, agents).
  // Used by config-nudge cards to deep-link to Settings → Agents.
  onOpenSettings: ((section: SettingsSection) => void) | null;
};

const AppShellContext = React.createContext<AppShellContextValue>({
  markAllChannelsRead: () => {},
  markChannelRead: () => {},
  markChannelUnread: () => {},
  openBrowseChannels: () => {},
  openCreateChannel: () => {},
  openChannelManagement: () => {},
  getChannelReadAt: () => null,
  getThreadReadAt: () => null,
  markThreadRead: () => {},
  getMessageReadAt: () => null,
  markMessageRead: () => {},
  readStateVersion: 0,
  setContextParentResolver: () => {},
  followThread: () => {},
  unfollowThread: () => {},
  isFollowingThread: () => false,
  isNotifiedForThread: () => false,
  isThreadMuted: () => false,
  threadActivityItems: [],
  threadActivityFeedItems: [],
  feedItemState: {
    doneSet: EMPTY_SET,
    markDone: () => {},
    markUnread: () => {},
    undoDone: () => {},
    undoUnread: () => {},
    unreadSet: EMPTY_SET,
  },
  onOpenSettings: null,
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
