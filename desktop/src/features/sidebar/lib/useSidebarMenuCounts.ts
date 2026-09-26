import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useCommunityBotsQuery } from "@/features/community-bots/hooks";
import { visibleCommunityDirectoryBots } from "@/features/community-bots/lib/directory";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { usePlaygroundSessions } from "@/features/playground/hooks";

import {
  deriveSidebarMenuCounts,
  type SidebarMenuCounts,
} from "./sidebarMenuCounts";
import {
  type SidebarMenuCountPreferences,
  useSidebarMenuCountPreferences,
} from "./sidebarMenuCountsPreference";
import { useSidebarInboxUnreadCount } from "./useSidebarInboxUnreadCount";

export type SidebarMenuCountsState = {
  /** Per-item Appearance toggles (Inbox / Browsers / Agents / Bots). */
  preferences: SidebarMenuCountPreferences;
  counts: SidebarMenuCounts;
};

/**
 * Live counts for the primary left-nav.
 * Inbox unread matches InboxListPane (not homeBadgeCount).
 * Browsers = playground browser groups (one row per group, matching
 * BrowsersScreen / browserRows); Agents = running/total managed roster;
 * Bots = visible community directory bots.
 */
export function useSidebarMenuCounts(): SidebarMenuCountsState {
  const preferences = useSidebarMenuCountPreferences();
  const inboxUnread = useSidebarInboxUnreadCount();
  const playground = usePlaygroundSessions();
  const managedAgentsQuery = useManagedAgentsQuery();
  const communityBotsQuery = useCommunityBotsQuery();
  const isArchived = useIsArchivedPredicate();

  // Match BrowsersScreen rows: one badge unit per browser group, not per tab.
  const browserGroupCount = playground.browsers.size;
  const agentTotalCount =
    managedAgentsQuery.data === undefined
      ? undefined
      : managedAgentsQuery.data.length;
  const agentRunningCount = React.useMemo(() => {
    if (managedAgentsQuery.data === undefined) return undefined;
    return managedAgentsQuery.data.filter((agent) =>
      isManagedAgentActive(agent),
    ).length;
  }, [managedAgentsQuery.data]);
  const botCount = React.useMemo(() => {
    if (communityBotsQuery.data === undefined) return undefined;
    return visibleCommunityDirectoryBots(communityBotsQuery.data, isArchived)
      .length;
  }, [communityBotsQuery.data, isArchived]);

  const counts = React.useMemo(
    () =>
      deriveSidebarMenuCounts({
        inboxUnread,
        browserGroupCount,
        agentRunningCount,
        agentTotalCount,
        botCount,
      }),
    [
      agentRunningCount,
      agentTotalCount,
      botCount,
      browserGroupCount,
      inboxUnread,
    ],
  );

  return { preferences, counts };
}
