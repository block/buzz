/**
 * Pure helpers for primary left-nav menu counts.
 * Inbox = unread; Browsers / Bots = roster sizes; Agents = running/total.
 * Browsers counts browser groups (BrowsersScreen rows), not tabs/sessions.
 */

export type SidebarAgentsCount = {
  running: number;
  total: number;
};

export type SidebarMenuCounts = {
  inbox: number | undefined;
  browsers: number | undefined;
  /** Compact `running/total` string, e.g. `3/12`. */
  agents: string | undefined;
  bots: number | undefined;
};

export type SidebarMenuCountId = keyof SidebarMenuCounts;

/** Cap displayed numeric chips at 99 (matches existing Inbox badge). */
export function formatSidebarMenuCount(count: number): number {
  return Math.min(Math.max(0, Math.floor(count)), 99);
}

/**
 * Resolve a displayable numeric count. Undefined / non-finite stays undefined
 * so the chip can hide while a roster is still loading.
 */
export function resolveSidebarMenuCount(
  count: number | undefined | null,
): number | undefined {
  if (count == null || !Number.isFinite(count) || count < 0) return undefined;
  return formatSidebarMenuCount(count);
}

/**
 * Agents badge: running (active ACP / deployed) over configured roster total.
 * Product form: `3/12`.
 */
export function formatSidebarAgentsCount(
  running: number | undefined | null,
  total: number | undefined | null,
): string | undefined {
  if (
    running == null ||
    total == null ||
    !Number.isFinite(running) ||
    !Number.isFinite(total) ||
    running < 0 ||
    total < 0
  ) {
    return undefined;
  }
  return `${formatSidebarMenuCount(running)}/${formatSidebarMenuCount(total)}`;
}

/**
 * When the preference is on, show every defined count (including 0 / `0/12`).
 * When off, Inbox keeps the legacy >0 unread chip; roster items stay hidden.
 */
export function shouldShowSidebarMenuCount(input: {
  preferenceEnabled: boolean;
  count: number | string | undefined | null;
  legacyWhenPositive?: boolean;
}): boolean {
  if (typeof input.count === "string") {
    if (input.count.length === 0) return false;
    return input.preferenceEnabled;
  }
  const resolved = resolveSidebarMenuCount(input.count);
  if (resolved === undefined) return false;
  if (input.preferenceEnabled) return true;
  return Boolean(input.legacyWhenPositive && resolved > 0);
}

export function deriveSidebarMenuCounts(input: {
  inboxUnread: number | undefined | null;
  /** Browser groups / BrowsersScreen rows (not tab/session count). */
  browserGroupCount: number | undefined | null;
  agentRunningCount: number | undefined | null;
  agentTotalCount: number | undefined | null;
  botCount: number | undefined | null;
}): SidebarMenuCounts {
  return {
    inbox: resolveSidebarMenuCount(input.inboxUnread),
    browsers: resolveSidebarMenuCount(input.browserGroupCount),
    agents: formatSidebarAgentsCount(
      input.agentRunningCount,
      input.agentTotalCount,
    ),
    bots: resolveSidebarMenuCount(input.botCount),
  };
}
