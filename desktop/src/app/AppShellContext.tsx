import * as React from "react";
import type { ForcedUnreadSource } from "@/features/channels/forcedUnreadStore";
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
    options?: {
      preserveForcedUnread?: boolean;
      topLevelOnly?: boolean;
    },
  ) => void;
  markChannelUnread: (channelId: string, source?: ForcedUnreadSource) => void;
  clearChannelUnreadSource: (
    channelId: string,
    source: ForcedUnreadSource,
  ) => void;
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
  // Read frontier for a channel-activity item, scoped to that item's own
  // message and channel rather than the currently mounted channel resolver.
  getChannelActivityItemReadAt: (
    item: Pick<FeedItem, "channelId" | "id">,
  ) => number | null;
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
  recordThreadInteraction: (rootId: string) => void;
  isThreadMuted: (rootId: string) => boolean;
  threadActivityItems: ThreadActivityItem[];
  threadActivityFeedItems: FeedItem[];
  // Home-feed items explicitly reopened from Inbox. Kept separate from live
  // thread activity so older rows can be projected into channel hover cards
  // without duplicating the Home feed itself.
  locallyUnreadFeedItems: FeedItem[];
  // Thread rows that remain unread until their own message/thread marker is
  // advanced. Unlike the broad channel unread set, this includes the active
  // channel so simply landing in it does not hide the wayfinding signal.
  unreadThreadFeedItems: FeedItem[];
  unreadThreadChannelIds: ReadonlySet<string>;
  // Ordinary unread channel-level activity. Sidebar rows use this for text
  // emphasis only; thread activity owns the dot.
  topLevelUnreadChannelIds: ReadonlySet<string>;
  // Lets isolated component tests retain the legacy hasUnread fallback while
  // the mounted shell uses the split projections above.
  hasSidebarUnreadProjections: boolean;
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
  clearChannelUnreadSource: () => {},
  openBrowseChannels: () => {},
  openCreateChannel: () => {},
  openChannelManagement: () => {},
  getChannelReadAt: () => null,
  getThreadReadAt: () => null,
  markThreadRead: () => {},
  getMessageReadAt: () => null,
  getChannelActivityItemReadAt: () => null,
  markMessageRead: () => {},
  readStateVersion: 0,
  setContextParentResolver: () => {},
  followThread: () => {},
  unfollowThread: () => {},
  isFollowingThread: () => false,
  isNotifiedForThread: () => false,
  recordThreadInteraction: () => {},
  isThreadMuted: () => false,
  threadActivityItems: [],
  threadActivityFeedItems: [],
  locallyUnreadFeedItems: [],
  unreadThreadFeedItems: [],
  unreadThreadChannelIds: EMPTY_SET,
  topLevelUnreadChannelIds: EMPTY_SET,
  hasSidebarUnreadProjections: false,
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
