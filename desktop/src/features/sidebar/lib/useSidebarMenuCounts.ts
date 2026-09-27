import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { countNewBrowserGroups } from "@/features/browsers/lib/browserAttention";
import {
  configureBrowserAttentionScope,
  getBrowserAttentionState,
  markBrowserGroupsAsSeen,
  reconcileBrowserAttentionWithRoster,
  subscribeBrowserAttention,
} from "@/features/browsers/lib/browserAttentionStore";
import { useCommunityBotsQuery } from "@/features/community-bots/hooks";
import { visibleCommunityDirectoryBots } from "@/features/community-bots/lib/directory";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { findBrowserByTabSid } from "@/features/playground/lib/browserGroups";
import { usePlaygroundSessions } from "@/features/playground/hooks";
import { usePendingProcedureCount } from "@/features/site-runbook/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";

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
 * Browsers = new (unseen) browser groups + pending runbook proposals;
 * Agents = running/total managed roster; Bots = visible community directory bots.
 */
export function useSidebarMenuCounts(): SidebarMenuCountsState {
  const preferences = useSidebarMenuCountPreferences();
  const inboxUnread = useSidebarInboxUnreadCount();
  const playground = usePlaygroundSessions();
  const managedAgentsQuery = useManagedAgentsQuery();
  const communityBotsQuery = useCommunityBotsQuery();
  const isArchived = useIsArchivedPredicate();
  const pendingRunbookCount = usePendingProcedureCount();

  const identity = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const pubkey = identity.data?.pubkey ?? "";
  const relayUrl = activeCommunity?.relayUrl ?? "";

  React.useEffect(() => {
    if (!pubkey || !relayUrl) return;
    configureBrowserAttentionScope(pubkey, relayUrl);
  }, [pubkey, relayUrl]);

  const [attentionEpoch, bumpAttention] = React.useReducer(
    (n: number) => n + 1,
    0,
  );
  React.useEffect(() => subscribeBrowserAttention(bumpAttention), []);

  const browserGroupIds = React.useMemo(
    () => [...playground.browsers.keys()],
    [playground.browsers],
  );

  // Seed / dispose housekeeping whenever the roster changes.
  React.useEffect(() => {
    reconcileBrowserAttentionWithRoster(browserGroupIds);
  }, [browserGroupIds]);

  // Opening / focusing a browser (overlay) marks that group as seen.
  React.useEffect(() => {
    const sid = playground.overlaySid;
    if (!sid) return;
    const browser = findBrowserByTabSid([...playground.browsers.values()], sid);
    if (!browser) return;
    markBrowserGroupsAsSeen([browser.browserId]);
  }, [playground.overlaySid, playground.browsers]);

  const newBrowserCount = React.useMemo(() => {
    void attentionEpoch;
    const attention = getBrowserAttentionState();
    if (!attention.seeded) return 0;
    return countNewBrowserGroups(browserGroupIds, attention.seenIds);
  }, [attentionEpoch, browserGroupIds]);

  const browserAttentionCount = newBrowserCount + pendingRunbookCount;

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
        browserAttentionCount,
        agentRunningCount,
        agentTotalCount,
        botCount,
      }),
    [
      agentRunningCount,
      agentTotalCount,
      botCount,
      browserAttentionCount,
      inboxUnread,
    ],
  );

  return { preferences, counts };
}
